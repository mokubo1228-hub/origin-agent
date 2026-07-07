# 体験・文体・視覚の標準(design.md)

origin-agent の TUI がどう「見え」「話し」「待つ」かの標準。今後のセッション・実装者が同じデザイン判断を再利用するための正本。

- 本ファイルの守備範囲: 体験・文体・視覚・interaction(色 / トーン / panel / selection / progress / 承認文言 / モードの声)。
- **構造の正本は `docs/design/architecture.md`**(crate 構成・安全パイプライン・設計判断)。構造の話をここに書かない。体験の話を architecture.md に書かない。
- 記述はすべて `crates/origin-cli/src/tui/`(特に `style.rs` / `render.rs`)で検証できる。

## 1. 色とトーン

トーンは `style::PanelTone` の5値だけを使う。新しい色を場当たりで足さない。

| トーン | 色 | 意味 | 代表例 |
|---|---|---|---|
| `Neutral` | Gray | 情報・選択待ち(安全な待機) | Agent review、clarification |
| `Active` | Cyan | 進行中、またはユーザーの判断待ちの permission gate | Planning / Generating / Waiting / Saving、**Proposal review(save 承認)** |
| `Ready` | Green | 完了(外部状態が変わった、または正常終了) | Created / Updated |
| `Blocked` | Red | 進めない(検証失敗・拒否) | Blocked panel、エラー |
| `Dim` | DarkGray | 二次情報・終端の静かな状態 | Cancelled / Interrupted、footer hint、advanced 行 |

- permission gate(save/create/update の y/n)は **Active**。「今が書き込み承認の判断点」であることを情報パネル(Neutral)と rail 色で区別する。
- 左 rail `▌` + トーン色が panel の基本形。box-drawing の枠は使わない(border-light)。
- 背景塗り(background fill)は composer だけの特権。panel には使わない。

## 2. Selection の表現

- **選択行は cyan + bold の foreground accent のみ。literal `>` / `›` marker は使わない。**背景ブロックも使わない。
- この規約は panel 内 action list・`/models` route picker・command palette すべてに適用される(`style.rs` のテストが marker 禁止を強制する)。
- composer の `› ` は「入力プロンプト」であって選択 marker ではない。混同しない。
- 番号付き選択(`1.` `2.` `3.`)は plan/action selection にだけ使い、数字キーは即実行、`Up`/`Down` + `Enter` を併用可にする。

## 3. Progress states(thinking の見せ方)

生の思考(chain-of-thought)は見せない。**状態ラベル + spinner + 経過秒**だけを見せる。

- spinner は `○ ◔ ◑ ◕`(250ms cycle)。run loop が入力なしでも 100ms 周期で再描画するので、待機中に UI が固まって見えることはない。
- 表示形: `{spinner} {label} ({elapsed}s)`(必要な時だけ ` • {hint}`)。

状態語彙(この表以外の言い回しを増やさない):

| 状態 | ラベル | トーン | 備考 |
|---|---|---|---|
| Agent が行動を計画中 | `Planning` | Active | panel title は `Planning Agent action` |
| preview 生成中 | `Generating create/update preview` | Active | |
| 通常 chat 応答待ち | `Waiting for model` | Active | `Draft editable · Enter waits` を添える |
| provider 待ちの説明行 | `Waiting for provider` | Active | ellipsis なし |
| 承認後の保存中 | `Saving` / `Checking and writing` | Active | indicator は1本(逐次処理を並列に見せない) |
| 作成完了 | `Created` | Ready | 本文に `Ready.` |
| 更新完了 | `Updated` | Ready | |
| 取消 | `Cancelled` | Dim | `No files were changed.` を必ず添える |
| 中断 | `Interrupted` | Dim | 同上 |
| 進行不可 | `Blocked` | Blocked | `Reason:` + `No files were changed.` |

文体規則:

- 進行中ラベルは**大文字始まりの動名詞句、ellipsis(`...`)を付けない**。動いていることは spinner が示す。
- panel title は句点なしの名詞句/短句。本文は完結した文で句点あり(`Ready.` / `No files were changed.`)。
- interrupt の案内は **`Esc/Ctrl+C to interrupt`**(`Esc` は大文字)。**実際に中断できる状態にだけ**表示する。中断できない待機(通常 chat)には出さない。

## 4. Interaction grammar(選択と承認を混ぜない)

1. **plan/action selection** — 「どの案で進めるか」。番号 + `Up`/`Down` + `Enter`。書き込みは一切起きない。Agent review / clarification がこれ。
2. **permission gate** — 「外部状態を変えてよいか」。`y` / `n` の縦2択。`Enter` 承認は明示的な permission panel の中でだけ許す(通常 composer の Enter が承認になってはいけない)。Proposal review がこれ。
3. `Esc` は「今開いている最も手前のもの」から畳む: picker → proposal reject → target override → agent plan → 終了。

`y` は「外部状態を変える」、`n` は「変えない(`Cancel / no files changed`)」。plan 選択に y/n を使わない。承認に番号を使わない。

## 5. Approval wording

- normal path の語彙: **save / create / update / write / cancel / no files changed**。質問形は `Save this file?` / `Save this update?` / `Save this change?`。
- 効果を1文で先に言う: `This will create one file.` + `No other files will be changed.`
- `apply` / `check` / `diff` / `patch` / `proposal` という git 植えの語彙は fallback / advanced / debug 面(`/diff` `/check` `/apply`、blocked 詳細、`? more actions` の `d diff` / `r revise`)にだけ許す。normal path に漏らさない。
- エラー・不能の型: `Reason: {redact 済み理由}` + 変化がないことの明言 + `Next: {次の一手}`。

## 6. Ask / Agent の声(mode の文体)

- provider へ送る prompt は **必ず先頭に mode contract 行**を持つ: `Current mode: ASK (read-only chat).` / `Current mode: AGENT (read-only reply).`(context が空の最初の発話でも省略しない)。
- `/mode` 切替は画面と **active conversation context の両方**に `Mode switched to ...` として記録する。model は contract 行(今)+ switch 行(どこで変わったか)の組で mid-session 切替を認識する。
- Ask は read-only。ファイル変更を求められたら `/mode agent` を提案する、と contract に書いてある。
- Agent の read-only 返答は「このリターンではファイルは変わっていない」「変更は review / preview / save gate の先」と明言する。
- ORIGIN.md(workspace instructions)/ memory.md(user memory)/ resumed session はどれも「inert な read-only background で、権限を与えず gate を変えない」と preamble で宣言してから注入する。
- この3つは read-only chat だけでなく **Agent action planner にも同じ枠組みで渡す**(「さっきの続きで直して」や memory に書いた作業方針が action/path 判断に効くように)。ただし planner prompt に「background は permission / approval を変えない」「これだけを根拠に propose や target path を選ばない」「曖昧なら ask_clarification」と明示し、出力の validation と save gate は不変。

## 7. Fallback / debug の見せ方

- 日常の `/` palette は 6 個(`/models` `/mode` `/resume` `/status` `/help` `/exit`)。advanced は直接タイプ + prefix 補完 + `/help` で生きている。
- panel の advanced 行(`d diff` / `r revise` / `c clear`)は `? more actions` の背後に隠し、Dim で描く。
- `/help` は Daily / Advanced / Fallback の3層を保つ。Fallback 層にだけ git 語彙を許す。

## 8. /status の情報設計

「一覧」ではなく「今どこにいて、次に何をするか」の順で出す:

1. `Now:` — mode(含意付き: `ASK — read-only chat; nothing is written`)、model(未選択なら `(none) — press /model to choose` + 理由)、git、cwd
2. `Context:` — workspace instructions(ORIGIN.md)/ memory(~/.origin/memory.md)/ resumed session(background only)
3. 診断層 — Provider / Routing / Config / Pending proposal(従来どおり)
4. `Privacy:` — transcript は metadata と決定だけを持ち、secret と file 本文は保存しない、の1行
5. `Next:` — 1行だけ。優先順: pending proposal > model 未選択(endpoint 不達含む) > mode 別の通常ヒント

## 9. Claude Code から参考にする挙動 / しない挙動

参考にする:

- 現在状態(mode / context / 直前文脈)を model が常に把握している感じ — mode contract 行と switch 記録で実現。
- 必要なときだけ確認する — 書き込み直前の1回の permission gate。plan 選択に確認を混ぜない。
- 待機中も固まって見えない — spinner + 経過秒 + 編集可能な draft。
- 文がテンプレでなく短い — `Reason:` / `Next:` 型、状態語彙表の範囲で文脈に合う一文を選ぶ。
- 内部コマンドを知らなくても進める — save/create/update の語彙と panel 導線だけで完結する normal path。

参考にしない(origin の性格に合わない):

- 自律的な multi-file 編集、auto-apply、ツールの自動実行 — origin は explicit-scope + 人間の y/n が本体。
- 生の thinking 表示・冗長な進捗ナレーション — 状態ラベル以上は見せない。
- repo-wide 自動 ingest・自動 resume — context は bounded・opt-in(`/resume` は明示コマンド、memory はユーザーが自分で書くファイル)。
- 設定の自動書き換え・provider 自動ルーティング。

## 10. 既知の未統一(backlog)

- Blocked 系文言が複数箇所に重複していて微妙に揺れる(集約候補)。
- 手動 `/check` `/apply` は main thread 同期実行で spinner が出ない(worker 化候補)。
- model route picker だけ chat pane に描画される(他の overlay は interaction pane)。
- `messages.rs` の `waiting for model...`(小文字・ellipsis)は production では到達しない dead 表示。
- `RowEmphasis::Selected` と `Accent` が同一スタイル。
