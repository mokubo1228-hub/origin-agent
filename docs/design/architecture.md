# アーキテクチャと設計判断

origin-agent の内部構造と、その形にした理由。コード上の実体（crate / モジュール / 関数名）に対応づけて書いてあるので、記述はすべて `crates/` を辿れば検証できる。

体験・文体・視覚・interaction の標準(色 / panel / selection / progress state / 承認文言)は [`design.md`](design.md)。本ファイルは構造のみを扱う。

## 技術スタック

| 領域 | 採用 | 備考 |
|---|---|---|
| 言語 / edition | Rust 2021 / rustc 1.75+ | workspace 全体で `unsafe_code = "forbid"`（`Cargo.toml`） |
| TUI | `ratatui` 0.29 + `crossterm` 0.28 | 代替スクリーン・raw mode の後始末を含む |
| HTTP | `reqwest` 0.12（`blocking` + `rustls-tls`） | OpenAI 互換エンドポイントへの同期呼び出し。OpenSSL 非依存 |
| 直列化 | `serde` / `serde_json` / `toml` | protocol 型・設定ファイル・API JSON |
| ハッシュ | `ring`（SHA-256） | transcript に本文でなく指紋を残すため |
| 乱数 | `rand` | セッション ID 生成 |

外部 LLM ベンダー SDK は持たない。理由は後述（設計判断 2）。

## crate 構成

6 crate の workspace（約 29,500 LOC）。依存は一方向で、`origin-protocol` を底、`origin-cli` を頂点とする。

```text
origin-cli ─┬─ origin-config ─┐
            ├─ origin-provider ┤
            ├─ origin-session ─┼─→ origin-protocol   （kernel: 依存は serde のみ）
            └─ origin-tools ───┘
```

| crate | LOC | 責務 | 主なモジュール |
|---|---|---|---|
| `origin-protocol` | ~324 | 共有 kernel 型。chat / model / patch / provider / session の DTO | `chat` `model` `patch` `provider` `session` |
| `origin-config` | ~451 | 設定解決。builtin 既定・ファイル読込・precedence | `builtin` `file` `precedence` |
| `origin-provider` | ~1,051 | OpenAI 互換抽象。trait・実装・preset・registry | `provider(trait ModelProvider)` `openai_compatible/*` `presets/{lmstudio,openai}` `registry` |
| `origin-session` | ~1,400 | transcript 永続化と秘匿・要約 | `redact_value` `redact_sensitive_text` `ring::digest` |
| `origin-tools` | ~1,696 | 書き込み安全パイプラインの部品 | `diff_extract` `patch` `git` `approval` `workspace` `file_read` |
| `origin-cli` | ~24,579 | オーケストレーション（CLI / shell / TUI） | `runtime` `chat_workflow` `edit_workflow` `status_snapshot` `commands` `shell` `tui/*`（23 module） |

`origin-cli` が LOC の大半を占め、そのうち `tui/`（`agent` `composer` `routing` `render` `style` `state` `model_picker` `edit_flow` `context` `messages` `workflow` ほか）が対話体験の実装。ロジック層（provider / tools / session / config）はここから分離されており、CLI・shell・TUI で再利用される。

---

## 設計判断

### 1. 依存ゼロの kernel を底に置き、依存方向を一方向に固定する

`origin-protocol` は `serde` 以外に依存しない純粋な型の層で、他の全 crate がこれに依存し、これは誰にも依存しない。HTTP・ファイル IO・UI といった副作用を持つ関心事が中核の型定義に逆流しないようにするための dependency inversion。`origin-cli` だけが全 crate を束ねてオーケストレーションを行う。

workspace 全体で `unsafe_code = "forbid"`。エージェントがファイルを書き換える性質のツールなので、メモリ安全性は妥協点にしない。

### 2. Provider は「OpenAI 互換抽象」1 本にする（ベンダー SDK を持たない）

`origin-provider` は `trait ModelProvider` を境界に、実装は `openai_compatible/`（`chat_completions` / `http` / `models` / `types`）1 系統だけを持つ。LM Studio と OpenAI は別実装ではなく `presets/` に載る preset で、`registry`（`builtin_registry` / `default_provider_id`）が束ねる。

ローカル優先。既定 provider は LM Studio 側に寄せてあり、クラウドキーが無くても起動して動く。ベンダーごとに SDK を抱えるより、OpenAI 互換という事実上の標準に一点張りする方が、対応先を増やすコストが preset 追加だけで済む。

### 3. モデル出力は直接ディスクに触れない — 書き込み安全パイプライン

このプロジェクトの中核。モデルが生成した diff は、次の関門を順に通過しない限りファイルに適用されない。関数はすべて `origin-tools` にある。

```text
モデル応答
  → extract_diff_proposal      : 応答から diff 提案を抽出（diff_extract）
  → validate_diff_targets       : 指定外ファイルを触る diff を拒否（patch）
  → validate_diff_integrity     : 壊れた diff を拒否（patch）
  → /diff プレビュー            : 変更内容を提示（適用しない）
  → git_apply_check             : `git apply --check`。ディスクは変更しない（git）
  → parse_approval_decision     : y / n の明示承認ゲート（approval）
  → git_apply                   : 承認された時だけ適用（git）
```

要点は 2 つ。第一に **auto-apply が存在しない**こと。第二に **「どの案で進めるかの選択」と「実行の承認」を別概念として扱う**こと。前者は plan/action selection、後者は create/update/write 直前の permission gate（`y / n`）で、UI 上も別物として現れる。`git apply --check` は preview（`/check`）でも apply（`/apply`）でも走り、失敗した diff は決して適用しない。

### 4. safe new-file だけは narrow exception を設ける

新規ファイル作成は、既存ファイルへのパッチと違って「壊すもの」が無いため、限定的に許す。`validate_diff_targets_allowing_new_files`（`patch`）＋ `workspace`（`resolve_existing_file` / `WorkspaceError`）のパス検証で、対象が current directory 内・親が存在・対象は未存在・hidden や `.env` / secret 風 / 絶対パス / 親への traversal でない・本文が空でない、を確認する。origin は `/dev/null` からの unified diff をローカルで組み立てるので、この経路は非 git workspace でも成立する。

### 5. transcript は本文を保存せず、秘匿してから書く

`origin-session` は会話・操作ログを永続化するが、**ファイル本文の全文は保存しない**。`ring` の SHA-256 で `bytes=<N> sha256=<...>` という指紋だけを記録する（`file_read` イベント等）。加えて書き込み前に `redact_value` が JSON を再帰的に走査し、`redact_sensitive_text` が `Authorization` / `OPENAI_API_KEY` / `api_key` 系の文字列・フィールドを `[redacted]` に置換する。コンテキスト用の要約も秘匿後に長さ切り詰め（4,000）してから使う。

秘匿は「表示しない」ではなく「そもそも保存経路に本文と鍵を流さない」構造で担保している。

### 6. 設定は決定的な precedence で解決する

`origin-config` の `resolve_config`（`precedence`）は CLI override > user config file > builtin default の順で解決し、設定欠落時は `builtin::DEFAULT_PROVIDER_ID` に落ちる。どの状態からでも起動時の provider が一意に決まるので、「設定を書かないと動かない」を避けつつ、挙動が環境依存でぶれない。

### 7. plan/action 選択と permission gate を UI grammar として分ける（TUI）

`tui/`（23 module）は上記の安全モデルを操作体験に落としたもの。`routing` / `state` / `composer` が入力を、`agent` / `edit_flow` / `workflow` が経路を、`style` / `render` が見た目を担う。設計方針として、固定操作メニューや focused clarification には `1/2/3` や上下 + Enter の selection を使い、create/update/write の直前だけ `y / n` の permission panel を出す。選択行は literal marker より背景・アクセントで示し、`esc to interrupt` の類は実際に中断できる状態でのみ表示する。

---

## 安全境界の一覧（不変条件）

- 既定で外部状態を変更しない。auto-apply しない。
- 変更を伴う実行の直前だけ明示承認（`y / n`）を要求する。
- `git apply --check` が失敗したら適用しない。指定外ファイルを触る diff・壊れた diff は拒否する。
- broad な delete / rename パッチは現時点では拒否する。new-file だけ narrow exception。
- ファイル本文の全文・API キー・Authorization 値は表示も保存もしない。

これらは design 上の努力目標ではなく、`origin-tools` / `origin-session` の関数境界で強制されている。
