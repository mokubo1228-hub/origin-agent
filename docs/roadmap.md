# origin-agent v1.0 Roadmap

The v1.0.0 baseline is complete.

This document now serves two purposes:

- historical record of the path to v1.0
- post-v1 guidance for choosing future work safely

For the current implementation and release state, use `README.md` and `CHANGELOG.md` as the source of truth.

Post-v1 priority: make origin-agent feel natural for daily lightweight local-first AI work. The core v1.0 safety workflow exists; the next work should reduce friction before adding large features.

## 1. Project Purpose

`origin-agent` は、現時点で Codex / Claude Code の完全代替ではない。

`origin-agent` は、LM Studio / OpenAI / OpenAI-compatible provider を扱う安全承認型 AI 編集 CLI である。

目的:

- LM Studio 経由のローカル LLM を扱う。
- OpenAI API / OpenAI-compatible API を同じ操作感で扱う。
- 明示指定したファイルを読ませる。
- unified diff を提案させる。
- `git apply --check` で検証する。
- 人間が承認した場合だけ apply する。
- session transcript を保存する。
- AI coding CLI の構造を自分で理解・制御できるようにする。
- 配信で開発過程やローカル LLM 運用を見せる。

最終的には Codex / Claude Code / Gemini CLI / aider から、origin-agent の safety-first / explicit-scope 設計に合う UX と workflow を取り入れていく。

ただし、これは「意図的に弱い道具に留める」という意味ではない。

origin-agent は local-first / OpenAI-compatible provider oriented な safety-first AI workbench として育てる。十分に強い local model や OpenAI-compatible model が使えるようになれば、origin-agent もその能力を活かして実用上の上限を上げられるべきである。

制約は「低い能力上限」ではなく、「明示的な safety design なしに unsafe autonomous behavior を default にしないこと」である。

## 2. v1.0 Definition

English definition:

> origin-agent reaches v1.0 when users can launch a TUI, chat with a selected local/API model, switch provider/model, propose edits for explicit files, inspect diffs, run checks, and apply changes only after human approval.

日本語定義:

> origin-agent は、TUI 上でローカル/API モデルを切り替えながらチャットし、明示指定したファイルに対して diff 提案・確認・承認 apply まで安全に実行できる状態を v1.0 とする。

v1.0 は「完成された Codex 代替」ではない。

これは「Codex / Claude Code より劣る状態を目指す」という意味ではない。v1.0 以降も、composer、chat pane、command flow、diff/check/apply visibility、明示的 context selection、安全な multi-file workflow などを通じて、使いやすさと実用上限を上げていく。

v1.0 の主眼は、TUI 上で以下が安全に通ることである。

- chat
- provider/model switching
- explicit file propose
- diff inspection
- check
- human approval apply
- session transcript

## 3. Session / Resume Definitions

### session

session は「実行記録」である。

`origin` が chat / propose / check / apply などを実行したときに、JSONL transcript として保存されるログを指す。

保存先:

```text
~/.origin/sessions/YYYY/MM/<session-id>.jsonl
```

session に保存するもの:

- command
- provider
- model
- user message / instruction
- file path / bytes / sha256
- model response
- diff proposal
- validation result
- check result
- approval decision
- apply result
- error result

session に保存しないもの:

- API key
- Authorization header
- `OPENAI_API_KEY` の値
- file contents 全文

つまり session は「過去ログ」「作業記録」「ノート」である。

### resume

resume は「過去 session を次の実行の background context として使う機能」である。

session をそのまま完全復元するのではなく、過去 session から要約的な context を作って、次の chat / propose に添える。

resume context の例:

- session_id
- command
- provider
- model
- user_message
- file_read path / bytes / sha256
- diff_proposed yes/no
- approval_decision
- patch_applied / patch_rejected
- status

resume の重要方針:

- 過去 session は background context として扱う。
- 過去 session は現在の file contents ではない。
- propose 時は現在の file contents を必ず読み直す。
- 過去 diff を勝手に復元しない。
- 過去 pending proposal を勝手に復元しない。
- 過去 session をもとに自動 apply しない。

つまり resume は「過去のノートを見ながら続きを話す」程度の軽量機能である。

### advanced resume

advanced resume は、過去 session から作業状態そのものを復元する機能である。

例:

- pending proposal 復元
- provider/model/shell state 復元
- 前回開いていたファイル群の復元
- 過去 diff の再利用
- 前回状態からそのまま `/check` / `/apply` できる状態復元

これは便利だが危険である。

理由:

- 過去 session 時点のファイル状態と現在のファイル状態がズレる可能性がある。
- 昨日の diff が今日の README.md に安全に適用できるとは限らない。
- pending proposal の復元には、現在ファイルとの整合性確認が必要になる。
- state 復元は agent 化・自動化に近づきやすい。

そのため、advanced resume は v1.0 ではやらない。

advanced resume は v1.1 以降の候補とする。

## 4. 現在地の扱い

最新の機能一覧や実装済み内容は、以下を正とする。

- README.md
- CHANGELOG.md

このロードマップは、最新状態を逐一更新するための文書ではない。
v1.0 までの道筋と、v1.0 後の方向性を固定するための文書である。

そのため、細かい現在地は README.md / CHANGELOG.md に寄せ、この文書では大きな流れだけを扱う。

v1.0.0 までの大まかな進行段階は以下。

1. plain CLI
2. interactive shell
3. shell file edit loop
4. TUI prototype
5. TUI provider/model switching
6. TUI file edit loop
7. v1.0 release candidate
8. v1.0.0 stable baseline

## 5. v0.5.x Roadmap

Shell UX polish.

候補:

- `/sessions` shell 表示 polish
- shell startup intro 整理
- shell error message 統一
- shell `/help` 整理
- shell state formatter 分離
- pending proposal 表示の最終調整
- README に demo flow を追加
- shell workflow を TUI の原型として固める

v0.5.x の目的は、TUI に入る前に shell 体験を安定させることである。

## 6. v0.6.x Roadmap

TUI prototype.

最初は以下だけでよい:

- chat pane
- status pane
- command input
- provider/model/status 表示
- TUI で chat できる

この時点では、TUI 上の file edit loop はまだ最小でよい。

まず「`origin` を起動すると TUI で chat できる」を成立させる。

## 7. v0.7.x Roadmap

TUI provider/model switching.

候補:

- provider list
- model list
- selected provider/model 表示
- session-local switching
- config は書き換えない
- shell の `/model set` / `/provider set` 相当を TUI に持ち込む

この段階で、Codex CLI 風の `/model` 体験に近づける。

## 8. v0.8.x Roadmap

TUI file edit loop.

候補:

- explicit files only
- propose
- diff view
- check
- approval apply
- pending proposal 表示
- clear

ここが v1.0 に向けた最大の山である。

## 9. v0.9.x Roadmap

Release candidate polish.

候補:

- demo flow 固定
- README / help 整理
- screenshots or asciinema 的な説明
- error wording
- provider failure handling
- transcript safety 再確認

配信で見せる最小シナリオもこの段階で固定する。

## 10. v1.0

TUI 上で以下が安全に通る状態:

- chat
- provider/model switching
- explicit file propose
- diff inspection
- check
- human approval apply
- session transcript

v1.0 では、TUI 上の安全な file edit loop が主眼である。

v1.0.0 でこの baseline は達成済み。

## 11. Things Not for v1.0

v1.0 ではやらない:

- 高度な resume
- pending proposal 復元
- 過去 diff 再利用
- repo-wide context
- autonomous planning
- MCP
- provider 固有パラメータの深掘り
- new/delete/rename patch 完全対応
- config 自動書き換え
- plugin system
- 自動 apply

これらは便利だが、v1.0 の軸をぼかすため後回しにする。

v1.0.0 後も、これらを近い将来の default 方針にはしない。
実装する場合は、個別に小さく scope を切る。

## 12. Design Risks

- `shell.rs` が TUI 前に肥大化しすぎる。
- shell と TUI で command semantics が二重実装になる。
- pending proposal state が shell 専用設計になりすぎる。
- TUI で diff 表示を入れたときに validation/apply flow が分散する。
- resume を状態復元に寄せたくなる。

対策:

- TUI に入る前に、shell で command semantics を固める。
- shell state / edit loop / provider-model resolution を TUI からも呼べる構造へ少しずつ寄せる。
- ただし、早すぎる大規模 `origin-core` 化は避ける。

## 13. How to choose the next step

次に実装する内容は、この文書だけで判断しない。

まず以下を確認する。

1. `README.md` の current usage
2. `CHANGELOG.md` の latest release / Unreleased
3. `git log --oneline --decorate -10`
4. `git status --short`

そのうえで、v1.0.0 後の安全な拡張として次の Phase を選ぶ。

post-v1 の候補:

1. v1.0.1: bugfix / wording / safety fixes
2. v1.1+: slash command suggestions / command palette
3. v1.1+: state-aware next-action hints
4. v1.1+: chat panel and message layout polish
5. v1.1+: provider routing UX and local / OpenAI / cloud tradeoff visibility
6. v1.1+: TUI sessions viewer
7. v1.1+: multi-file TUI propose
8. v1.1+: explicit context selection for safe scoped work
9. v1.1+: safe multi-file workflows

Phase 23.4 で、Agent mode の state / Ask-vs-Agent contract correction が入った。これは Phase 23.5.1〜23.5.6 とともに **`v1.3.0` として release 済み**(現在の最新 released tag は `v1.4.0`)。

Phase 23.4 の受け入れ済み解釈:

- 主要な完了点は UI polish だけではなく、Ask / Agent mode-state correction である。
- Agent review は fixed 3-choice action selection になった。
- `y / n` は Agent review action selection ではない。
- Proposal review が write/apply permission gate である。
- Proposal review の `Up` / `Down` + `Enter` は、明示的な permission panel 内だけで許容される。
- Agent preview generation は background worker で走り、`Esc` / `Ctrl+C` で interrupt できる。
- cancellation 後の late provider result は無視される。
- normal provider waiting は running-state UI を持つ。
- Ask / Agent mode contract は provider context 上で明示される。
- Agent read-only fallback は Ask-mode-like wording ではなく Agent-mode preface を使う。
- `はい`, `yes`, `ok`, `お願い`, `進めて`, `continue`, `go ahead` のような short confirmation は、適切な場合に prior Agent task を continue / clarify できる。
- Ask mode と Agent mode は、Agent context がある場合に同じ short input を異なるものとして扱える。

今後の近い候補:

- Phase 23.5 split (visual hierarchy / interaction grammar / command surface unification) — 23.5.1〜23.5.6 すべて実装済みで、`v1.3.0` として release 済み:
  - 23.5.1 + 23.5.2 — shared TUI style/component layer、left-rail live panel、`? more actions` reveal、input-buffer + Enter の Agent review selection(footer microcopy fix 含む)。実装済み。
  - 23.5.3 — `/models` numbering + numeric buffer。実装済み。
  - 23.5.4 — composer / command palette de-bordering。実装済み。
  - 23.5.5 — command-surface cleanup、planned `/resume` slot、`/sessions` advanced/direct 化、message history reskin。実装済み。
  - 23.5.6 — startup mascot landing(block-art mascot、`origin-agent — local-first AI workbench` wordmark、`Type a message to start.` hint。古い status block と edit-loop pipeline を empty state から除去)。実装済み。
  - これらは実装済みで `## v1.3.0` CHANGELOG entry にまとめられ、**v1.3.0 として release 済み**(現在の最新 released tag は `v1.4.0`)。
  - 以前 "23.5.6" と呼ばれた Agent / Proposal input-model 統一(instant `y` / `n` 撤去)は v1.3.0 scope 外で、post-v1.3 follow-up に deferred。
- Phase 24 / 25 / 26 は **`v1.4.0` として release 済み**(現在の最新 released tag。以前 post-v1.3 の product-owner 確定 sequencing だったもの):
  - Phase 24 ORIGIN.md workspace instructions — `<cwd>/ORIGIN.md` を起動時に一度 load し、Ask / Agent に read-only workspace background として渡す。session cache + `/status` 表示、instruction 本文は transcript に保存しない(safety / approval / routing は不変)。
  - Phase 25 Agent intent classification — Rust の verb/confirmation lexicon をやめ、model-driven action planner(read-only answer / clarification / 既存1ファイル update / 新規1ファイル create / cancel / no action のみ)に置換。origin が review 前に validate し、unsafe / invalid / multi-file は clarification に fallback。
  - Phase 26 explicit model selection — 起動時の silent auto-select を廃止し、`No model selected — press /model to choose` を出して選択まで送信を block。`/model` picker は発見済み候補を 1-`Enter` recommendation として提示。
- post-v1.4 direction:
  - **実装済み(unreleased、`CHANGELOG.md` の `## Unreleased` 参照)** — Ask/Agent mode contract 行 + `/mode` 切替の会話コンテキスト記録、`~/.origin/memory.md` の read-only user memory(origin は書かない・metadata-only transcript)、TUI lightweight `/resume [latest|<session-id>]`(bounded read-only background、状態復元なし)、`/status` の daily 情報設計(Now / Context / 診断 / Privacy / Next)、wording / visual grammar 統一(palette `›` marker 撤去、permission panel の Active tone 等)、`docs/design/design.md`(体験標準)。
  - future backlog(direction のみ・未スケジュール)— agent が memory を書く経路(必ず専用の明示承認 gate が要る)、sessions viewer の拡充。次の committed phase は product owner の判断待ち。

近い将来の default にしないもの:

- repo-wide agent behavior
- MCP
- auto-apply
- config auto-writing
- advanced resume / state restoration

## 14. Post-v1 Daily-use UX Direction

v1.0.0 後の問題は「中核機能が足りないこと」ではなく、「軽いタスクで毎日使うにはまだ摩擦があること」である。

origin-agent は local-first AI work の入口として育てる。

Routing ladder:

1. Local LLM

   - privacy が最も強い
   - local compute を使う
   - cloud AI quota を使わない
   - 軽いタスク、docs、小さい編集、private notes、routine work 向け

2. OpenAI / OpenAI-compatible external API

   - local より性能が必要な時の中間選択肢
   - Codex / Claude Code の使用枠を温存できる
   - 重要度が高すぎない作業、local では弱い作業向け

3. Codex / Claude Code / cloud AI

   - performance-critical work 用
   - 大きな実装、難しい coding、広い repo 理解が必要な作業向け
   - origin-agent はこれらを完全代替しない

Near-term UX candidates:

- slash command suggestions / command palette
- `/` 入力時の command 候補表示
- command description display
- state-aware next-action hints
- chat panel UI
- user / assistant / system / error message layout polish
- provider/model/routing information that does not crowd the chat experience
- generated preview / pending write state visibility
- normal Agent wording around save/create/update/cancel instead of requiring users to think in `/diff` / `/check` / `/apply`
- `/status` that explains why the current provider/model is selected
- startup intro that explains what to do next
- visible privacy / cost / quota / performance tradeoffs for local / OpenAI / cloud routing

Intended normal Agent path:

```text
User prompt
↓
origin selects a reasonable plan, or asks the user to choose when multiple next actions exist
↓
plan/action selection uses `1 / 2 / 3`, with intended `Up` / `Down` plus `Enter` support
↓
generate the content/change
↓
show the generated result, preview, or summary inline
↓
ask whether to save/create/update it with `y / n`
↓
after `y`, run internal safety checks and write only if safe
↓
show created / updated / cancelled / blocked / ready
```

Plan/action selection is not file write approval. It should include `Other / custom instruction` when presented choices may be wrong, and `Cancel` should be last. Execution approval is only for external-state-changing actions such as create file, update file, delete file, write, or future mutating command execution.

After any shortcut key or `Enter` confirmation, the UI should visibly change immediately. Future Agent UX should show non-frozen progress states such as `thinking...`, `generating...`, `checking...`, `writing...`, `created`, `updated`, `cancelled`, `blocked`, and `ready`.

`/diff`, `/check`, and `/apply` remain explicit fallback / direct / debug workflow commands. They are still important for manual inspection and precise control, but they should not be the primary mental model for normal Agent UX.

After v1.2.0, Phase 23.4 corrected Agent mode state and Ask-vs-Agent contract behavior, and Phase 23.5.1–23.5.6 delivered the visual hierarchy / workbench convergence. These form a coherent user-facing milestone, released as `v1.3.0` (later superseded as latest by `v1.4.0`):

- non-blocking TUI chat so the interface stays responsive while waiting for the model
- focused composer and command palette interaction, with de-bordering and a shared left-rail visual grammar
- IME / Japanese input polish
- structured message, action, result, and error block rendering, plus a reskinned message history
- latest-output-first behavior for small terminal heights
- `/models` route picker with numbering + numeric buffer
- curated command surface (daily `/` palette, planned `/resume` slot, `/sessions` demotion)
- startup mascot landing
- Agent / Proposal / Result panels using background/accent styling rather than marker-heavy or border-heavy UI

Shipped in `v1.4.0` (was the confirmed post-v1.3 sequencing): Phase 24 ORIGIN.md workspace instructions, Phase 25 Agent intent classification (model-driven action planner), and Phase 26 explicit model selection (no silent auto-select). Post-v1.4: small follow-up polish continues; agent memory / cross-session context stays a direction-only backlog item; deferred: the Agent / Proposal input-model unification (removing instant `y` / `n`).

Do not prioritize before daily-use UX:

- auto-apply
- repo-wide autonomous agent behavior
- MCP as a near-term default
- full Codex / Claude Code replacement
- advanced resume as the next default direction

Do prioritize higher-ceiling UX where it preserves explicit scope and human approval:

- better composer and chat pane interaction
- clearer normal Agent flow from plan/action selection to generated preview to execution approval
- clearer manual fallback visibility for `/diff`, `/check`, and `/apply`
- explicit context selection instead of repo-wide automatic context
- safe multi-file workflows
- local-first / OpenAI-compatible model routing that can benefit from stronger models

この節は「現在の最新タスク」を固定する場所ではない。
最新の実装状態は README.md / CHANGELOG.md を参照する。
