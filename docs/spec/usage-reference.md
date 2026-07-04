# origin-agent 利用リファレンス(完全版 / English)

> このファイルは CLI / interactive shell / TUI の **全コマンドと安全挙動を網羅した詳細リファレンス**。
> portfolio / クイックスタートの入口は [README](../../README.md) を参照。
> 内容は開発者・パワーユーザー向けに英語で維持している(AI / 開発 doc layer)。

---

## Install

Install the CLI locally:

```sh
cargo install --path crates/origin-cli
```

Then run:

```sh
origin --help
```

For development, running through Cargo is still supported:

```sh
cargo run -p origin-cli -- --help
```

## Quick Start

Show help:

```sh
origin --help
```

List providers and models:

```sh
origin status
origin provider
origin model --model explicit-test-model
origin providers
origin models
```

Send a single chat prompt:

```sh
origin chat "hello"
```

Start the TUI workbench:

```sh
origin
```

Start the TUI workbench explicitly:

```sh
origin tui
```

Start the old interactive shell explicitly:

```sh
origin shell
```

Continue with background from the latest saved session:

```sh
origin chat --resume latest "continue from previous context"
```

Preview an edit without changing files:

```sh
origin propose "fix typo" README.md
```

Inspect saved transcripts:

```sh
origin sessions
origin sessions show --latest
```

## Explicit Interactive Shell

Run `origin shell` to start the old interactive shell explicitly:

```text
origin-agent interactive shell

Provider: lmstudio
Model:    qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive
Git:      clean

Type /help for commands.
Try /status, /model, /provider, /sessions.
Editing flow: /propose -> /diff -> /check -> /apply
Type /exit to quit.

origin> /status
origin> /model list
origin> /model set qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive
origin> /provider list
origin> /provider set lmstudio
origin> hello
origin> /propose README.md fix typo
origin> /propose README.md CHANGELOG.md -- align release wording
origin> /diff
origin> /check
origin> /apply
origin> /clear
origin> /exit
```

Available shell commands:

- `/help`
- `/exit`
- `/quit`
- `/status`
- `/model`
- `/model list`
- `/model set <model-id>`
- `/provider`
- `/provider list`
- `/provider set <provider-id>`
- `/propose <file> <instruction...>`
- `/propose <file1> <file2> ... -- <instruction...>`
- `/diff`
- `/check`
- `/apply`
- `/clear`
- `/sessions`

Text without a leading slash is sent as a chat prompt.

Shell state keeps the current provider, model, base URL, config path, session root, current working directory, and git state for the duration of the shell session.

`/model set` and `/provider set` are session-local. They do not rewrite `~/.origin/config.toml`, and they are not persisted after the shell exits. If a pending proposal exists, changing the model or provider clears it.

Shell chat and shell edit commands are saved to the normal session transcript directory.

Shell edit loop:

```text
origin> /propose README.md fix typo
origin> /propose README.md CHANGELOG.md -- align release wording
origin> /diff
origin> /check
origin> /apply
```

Shell edit behavior:

- `/propose <file> <instruction...>` reads the current file and previews a file-scoped diff.
- `/propose <file1> <file2> ... -- <instruction...>` reads multiple current files and previews a file-scoped diff.
- Multiple files require `--` before the instruction to avoid ambiguous parsing.
- `/propose` only targets the files explicitly passed to the command.
- A successful `/propose` creates a pending proposal.
- `/status` shows runtime, provider, model, config, and pending proposal state.
- `/model`, `/provider`, and `/sessions` are formatted for quick shell inspection.
- `/diff` re-displays the pending proposal diff and does not run `git apply`.
- `/check` runs `git apply --check` against the pending proposal and does not change files.
- `/apply` reruns `git apply --check` even if `/check` already succeeded.
- `/apply` asks for approval and only `y` or `yes` applies the patch.
- Pressing Enter at the `/apply` prompt rejects the patch.
- Rejecting `/apply` keeps the pending proposal so you can run `/apply` again.
- A successful `/apply` clears the pending proposal.
- `/clear` discards the pending proposal without changing files.
- Changing model or provider clears the pending proposal to avoid applying a diff created under a different shell state.

## TUI Workflow

Run the TUI workbench with:

```sh
origin
```

The explicit TUI route is also available:

```sh
origin tui
```

The current TUI includes:

- thin workbench header
- startup landing with a compact block-art origin mascot, a `origin-agent — local-first AI workbench` wordmark, and a `Type a message to start.` hint (falls back to text on short terminals)
- chat pane with rail-less user/assistant rows and left-rail action, waiting, and error blocks
- border-light visual grammar: left-rail live panels, a blue/cyan selected-row accent shared across panels, `/models`, and the command palette, and no box-drawing around the composer or command palette
- light composer input area with a subtle background fill and blank background spacer rows above and below, plus a keyboard-owner affordance that dims the composer when a menu/panel owns input
- attached slash command palette
- multiline input composer
- state-aware footer
- chat input
- `Enter` accepts the selected slash suggestion when the command palette is active; otherwise it submits the composer
- `Ctrl+J` inserts a newline in the composer
- `Left` / `Right` move the cursor within the composer
- `Up` / `Down` move vertically across logical composer lines when slash suggestions are not active
- `Up` / `Down` navigate slash command suggestions when the command palette is active
- insertion and `Backspace` work at the current cursor position
- user, assistant, action, waiting, and error messages
- chat transcript logging
- daily slash palette: `/models`, `/mode`, `/resume`, `/status`, `/help`, and `/exit`
- advanced/direct commands: `/mode ask`, `/mode agent`, `/model`, `/model list`, `/model set <model-id>`, `/provider`, `/provider list`, `/provider set <provider-id>`, `/sessions`, and `/quit`
- `/resume` planned slot, which reports that TUI resume is not implemented yet and points to `origin sessions` / `origin chat --resume latest`
- fallback/direct edit commands: `/propose <file> <instruction...>`, `/diff`, `/check`, `/apply`, and `/clear`
- `/mode` toggles Ask / Agent, while `/mode ask` and `/mode agent` set the mode explicitly
- `/models`, `/model list`, `/model set <provider>/<model-id>`, and `/model set <model-id>`
- `/provider list` and `/provider set <provider-id>`
- state-aware footer next-action hints and Agent review panels near the composer
- slash command suggestions while typing `/`
- selectable slash command suggestions attached above the composer
- composer placeholder, prompt marker, continuation lines, focused input cell, and tail display for longer input
- muted submitted-input rows and inline assistant reply markers for readable chat history
- latest output/result/error stays visible when terminal height is limited
- `/status` provider routing/tradeoff hints for local, OpenAI, and OpenAI-compatible providers
- safe exit with `q`, `Esc`, or `Ctrl+C`

The TUI does not call a provider when it starts. It resolves and calls the provider only when you send a chat prompt.

TUI interaction mode is session-local and defaults to Ask. `/mode` toggles between Ask and Agent; `/mode ask` and `/mode agent` set the mode explicitly:

```text
> /mode
> /mode ask
> /mode agent
```

Ask mode is read-only chat. When a normal non-slash prompt explicitly mentions safe file paths such as `README.md`, `CHANGELOG.md`, `docs/START_HERE.md`, or `crates/origin-cli/src/tui.rs`, the TUI reads those existing files and includes their contents as context for that one provider request. Missing or unsafe paths are skipped with a visible diagnostic. Ask mode never creates a pending edit proposal.

Ask mode can also answer bounded workspace overview prompts such as `このプロジェクトを軽く読んで`, `このフォルダを見て`, or `summarize this folder`. For these prompts, the TUI treats the current working directory as a generic workspace, builds a shallow bounded tree/list, and includes selected safe overview files such as README, START_HERE, CHANGELOG, docs Markdown files, Japanese handoff/spec notes, and common manifest files. It does not assume the workspace is a code repository, does not automatically expand to the git root, and does not read every file.

Ask mode skips hidden files, heavy directories such as `.git/`, `target/`, `node_modules/`, `dist/`, `build/`, and `vendor/`, binary files, too-large files, `.env` files, and secret/token/key/credential-like files. Full file contents and auto-discovered tree contents are not saved to transcripts; transcripts store metadata such as paths, status, byte counts, hashes, and overview counts.

The TUI keeps a small in-memory active conversation context for the current run. Recent user prompts, assistant replies, Agent review status, and Agent diagnostics are included as bounded read-only context for follow-up provider requests, so short prompts such as `どう？`, `それで`, or `今の内容で` are not sent without the immediate prior topic. This is session-local only: it is not restored from old transcripts, does not implement advanced resume, and does not store full file contents or full workspace trees.

Agent mode is the natural-language entry point for read-only chat or explicit single-file diff proposal. For every normal non-slash prompt, Agent mode first asks the selected model for one structured action from a closed allowlist: answer read-only, ask for clarification, propose one existing-file update, propose one new-file create, cancel, or no action. origin validates the model's action and path before showing any review. It does not open a file target picker.

When the planner selects a read-only answer, Agent mode uses the Ask-style read-only path. Agent mode may include explicitly mentioned safe files or the same bounded workspace overview context, calls the provider for chat output, does not create a pending proposal, and does not open an edit target picker.

When the planner proposes one safe existing file update, Agent mode opens a dedicated Agent review panel near the composer before any proposal provider call. The panel names the file to edit, the context being used, may show the planner rationale, and has numbered actions: `1.` generate an update preview, `2.` return to the composer for other/custom instruction, and `3.` cancel. Pressing a valid number runs that action immediately; `Up` / `Down` moves the highlight, `Enter` confirms the highlighted action, and `Esc` cancels. No files are changed at Agent review time. If the planner action is ambiguous, invalid, unsafe, or missing a required target, Agent mode opens a focused clarification panel instead of silently answering read-only or fabricating a proposal.

When the planner proposes one new file create, Agent mode validates the proposed target strictly. The target must stay inside the current working directory, have an existing parent directory, not already exist, and not be hidden, `.env`-like, secret-like, absolute, or parent-traversing. Missing or invalid create paths ask for clarification; Agent mode does not infer a default output path in this phase.

For a valid new-file create, Agent mode shows a dedicated Agent review panel with `Create: <path>`, the context being used, may show the planner rationale, and has numbered actions: `1.` generate a create preview for the proposed path, `2.` edit the output path, and `3.` cancel. Pressing a valid number runs that action immediately; `Up` / `Down` moves the highlight, `Enter` confirms the highlighted action, and `Esc` cancels. After choosing `2`, type another safe relative path and press Enter to update the review, then choose `1` to generate the preview. Agent mode does not show a picker or generic candidate list.

Agent follow-up chat uses the active in-session context above as planner input. Short confirmations such as `はい`, `yes`, `ok`, `お願い`, `進めて`, `continue`, or `go ahead` are interpreted by the model with that context instead of by a Rust verb table. Ask mode remains read-only, so the same short input can stay normal chat in Ask mode. Cross-run resume, persistent Agent plans, and automatic target carryover remain deferred to future resume/context work.

External paths are outside the Agent edit boundary in this phase. Agent mode reports explicit external references instead of silently using them; external edits are rejected, and external read-only context confirmation is not implemented yet.

After proposal generation succeeds, Agent mode opens a dedicated Proposal review panel instead of making slash commands the normal path. The panel uses save/create/update wording, shows the target, and asks for explicit save approval with vertical `y` / `n` actions: `y` saves through the safe check/apply path when available, and `n` cancels with no files changed. `Up` / `Down` can move the highlighted permission action and Enter confirms only inside that explicit permission panel; normal composer Enter is not a save/apply approval. The `y` / `n` permission prompt has no numeric selection. Advanced/debug actions are hidden behind a `? more actions` reveal in the normal path: pressing `?` toggles them, and `d` still shows the full diff and `r` still enters revision feedback mode as secondary controls.

```text
> /mode agent
> README.md の誤字を直して
> 1
```

Agent mode does not auto-apply. `/diff`, `/check`, and `/apply` remain available as manual fallback commands for pending proposals; in Git repositories `/apply` still reruns `git apply --check` and requires explicit `y` or `yes`. In a non-git workspace, Proposal review `y` can safely create exactly one new file when the pending proposal is a validated `/dev/null -> <target>` diff, the target is inside the current directory, the parent exists, the target does not already exist, and the content is non-empty. Non-git existing-file edits remain blocked with a Blocked panel. Agent mode does not support multi-file natural-language edits, external edits, delete diffs, or rename diffs yet; use one safe target or the direct `/propose <file> <instruction...>` command.

Near-term Agent UX direction after Phase 23.3 is to reduce the need to understand `/diff`, `/check`, and `/apply` in the normal path while keeping those commands as manual fallback / direct / debug controls. Future normal Agent UX should distinguish plan/action selection from execution approval: plan/action selection should use numbered choices such as `1 / 2 / 3` with intended `Up` / `Down` plus `Enter` support, while `y / n` should be reserved for external-state-changing execution approval such as save, create, update, delete, or write. This is direction, not a replacement for the current command behavior documented above.

Agent proposal generation returns only a pending diff. For Agent new-file proposals, origin asks the model for the target file body and constructs the `/dev/null` unified diff locally, so hunk header counts are not trusted to free-form model output. For Agent existing-file proposals up to 64 KiB, origin asks the model for the complete updated file body and constructs the single-target unified diff locally from the current file plus that body. Larger existing files do not fall back to raw model diffs; Agent asks you to narrow the instruction, choose a smaller target, or cancel. Manual `/propose` / `/diff` keep the model-generated unified-diff path as the direct fallback/debug route. If a model-generated diff path returns an invalid unified diff, Agent mode does not store it and retries once with validation feedback for the same confirmed target path. If the retry also fails, no pending proposal is stored and no files are changed.

During Agent preview generation, `Esc` or `Ctrl+C` interrupts the running provider request logically, returns to the normal composer, and ignores any late provider response. No pending proposal is stored and no files are changed.

Empty new-file proposals are rejected. A model response that creates a file with no meaningful added content, or only blank added lines, does not become a pending proposal.

TUI chat requests are non-blocking from the interface perspective:

- The provider request runs in a background worker while the TUI keeps rendering and polling input.
- You can continue typing a draft while waiting for the model response.
- Pressing `Enter` while a chat response is in flight does not submit a second prompt.
- Ask/Agent read-only waits show a current-state `Waiting for model` panel with the same lightweight running indicator style as Agent preview generation, without claiming interrupt support.
- Agent read-only fallback sends an Agent-mode provider context preface rather than Ask-mode wording; it states that no files have changed and that file changes stay behind Agent review, preview generation, and save permission gates.
- The waiting warning is shown once per in-flight request instead of repeating on every `Enter`.

The TUI composer is intentionally small:

- `Enter` submits the current input.
- `Ctrl+J` inserts a newline at the cursor.
- `Left` / `Right` move the cursor within the composer.
- `Up` / `Down` move vertically across logical composer lines when slash suggestions are not active.
- `Up` / `Down` still navigate slash command suggestions when the command palette is active.
- Text insertion and `Backspace` operate at the current cursor position.
- Multiline chat prompts are sent as one prompt.
- Multiline slash commands are not supported yet and show a clear error.
- When the composer is taller than the visible area, it keeps the latest lines visible.
- Japanese/full-width display width is handled for composer rendering and cursor placement.
- The terminal cursor is anchored inside the composer so Japanese IME preedit text appears at the input cell.
- `PageUp` / `PageDown` scroll the message history while the composer stays fixed.
- Mouse capture is disabled so native terminal text selection and copy work normally; wheel/trackpad scrolling is left to the terminal.
- Home / End, word-wise movement, composer selection, soft-wrap-aware vertical movement, and full grapheme-cluster editing are not implemented yet.

TUI `/status` includes a short routing/tradeoff hint for the current provider:

- LM Studio is treated as the local route for private, lightweight, local-compute work.
- OpenAI is treated as an external OpenAI-compatible API for work that needs more capability than local models without opening a full coding agent.
- Custom OpenAI-compatible providers are shown as manually selected provider-specific routes.

TUI model route switching is session-local:

```text
origin tui

> /models
> /model set lmstudio/qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive
> /model set openai/zai-org-glm-5-1
> /status
```

`/models` opens the primary selectable model route list. It shows known provider routes from current, configured/default, and live model-list sources where available, with unavailable providers kept visible using safe redacted diagnostics. Selectable routes are numbered `1.`, `2.`, `3.`, … and the selected route is shown with a blue/cyan accent (no `>` marker). `Up` / `Down` moves the selection; typing a number highlights the matching route via a numeric input buffer (so `12` is not treated as `1`), and a digit alone does not switch; `Enter` uses the highlighted route, `Backspace` edits the buffer, and `Esc` cancels. `/model list` is kept as a legacy-compatible alias for `/models`.

`/model set <provider>/<model-id>` is the direct manual fallback and switches provider and model together for the current TUI session. `/model set <model-id>` keeps the current provider and changes only the model. These changes do not rewrite `~/.origin/config.toml`, and the selection is not persisted after the TUI exits.

`/provider list` and `/provider set <provider-id>` remain available for diagnostics and advanced use, but the daily path is to choose a model route with `/models`.

`/sessions` remains available as an advanced/history command. It is direct-typeable, prefix-completable, and listed in `/help`, but it is no longer shown in the daily root `/` palette.

`/resume` is currently a planned TUI slot only. It does not restore sessions, call a provider, create or modify files, or change TUI state; for now, use `origin sessions` to browse saved sessions or `origin chat --resume latest "<your prompt>"` from the plain CLI.

At TUI startup, origin looks for exactly `ORIGIN.md` in the current working directory. If it exists, is a regular non-empty UTF-8 file, is not a symlink, and is within the size limit, Ask mode and Agent read-only chat include it as read-only workspace instructions for provider context. The file is inert background only: it cannot trigger commands, choose routes, create proposals, write files, or bypass approval. `/status` shows workspace instructions as loaded, skipped with a reason, or none. Session transcripts store only metadata for loaded or skipped files and never store the `ORIGIN.md` body; absent files write no transcript event.

Canonical TUI demo flow:

```text
origin tui

> /status
> /models
> /model set lmstudio/qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive
> hello
> /propose README.md fix typo
> /diff
> /check
> /apply
Apply decision: press Enter, n, no, or anything else to reject.
Apply decision: type y or yes only when the patch is safe.
> /clear
> /exit
```

The demo flow is intentionally explicit: inspect status, choose model/provider only for the current TUI session when needed, preview a diff, check it, then approve only after review.

TUI edit loop:

```text
origin tui

> /propose README.md fix typo
> /diff
> /check
> /apply
```

TUI edit behavior:

- `/propose <file> <instruction...>` reads the current file and previews a file-scoped diff.
- Normal Ask-mode prompts with mentioned files are read-only and do not create pending proposals.
- TUI `/propose` is single-file only for now.
- `/diff` re-displays the pending proposal diff and does not run `git apply`.
- `/check` runs `git apply --check` against the pending proposal and does not change files.
- `/apply` reruns `git apply --check` even if `/check` already succeeded.
- `/apply` asks for approval and only `y` or `yes` applies the patch.
- Pressing Enter, `n`, `no`, or anything else at the `/apply` prompt rejects the patch.
- Rejecting `/apply` keeps the pending proposal.
- A successful `/apply` clears the pending proposal.
- `/clear` clears visible TUI messages and discards the pending proposal without changing files.

The current TUI does not implement:

- multi-file TUI `/propose`
- multi-file Agent edits from normal prompts
- TUI sessions viewer
- streaming
- file picker
- PageUp / PageDown as composer editing keys
- Home / End
- word-wise movement
- selection
- soft-wrap-aware vertical movement
- full grapheme-cluster editing
- unbounded repo-wide context selection
- MCP integration
- auto-apply
- config auto-writing

Safety behavior:

- TUI `/propose` previews only.
- TUI `/check` does not modify files.
- TUI `/apply` reruns `git apply --check`.
- TUI `/apply` requires `y` / `yes` approval.
- Rejected apply keeps the pending proposal.
- Successful apply clears the pending proposal.
- TUI does not display or save API keys.
- TUI uses the existing transcript safety behavior.
- Terminal raw mode and alternate screen cleanup are handled.

## LM Studio Setup

LM Studio is the default provider preset.

- Start the LM Studio Local Server before running `models`, `chat`, or `propose`.
- The default base URL is `http://localhost:1234/v1`.
- Load a model in LM Studio before sending prompts.
- The current local target model is `qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive`.
- Very large models may fail to load or respond slowly depending on local hardware.

Useful commands:

```sh
origin providers
origin models
origin chat "hello"
```

## Config File

User config is read from:

```text
~/.origin/config.toml
```

For tests or temporary runs, override the path with `ORIGIN_CONFIG`:

```sh
ORIGIN_CONFIG=/path/to/config.toml origin models
```

Example:

```toml
default_provider = "lmstudio"
default_model = "qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive"

[providers.lmstudio]
base_url = "http://localhost:1234/v1"
default_model = "qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive"

[providers.openai]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
default_model = "zai-org-glm-5-1"
```

Do not put API key values in config. Only store the environment variable name, such as `OPENAI_API_KEY`.

Resolution order:

1. CLI args
2. Environment variables
3. User config
4. Built-in defaults or provider fallback

## OpenAI Setup

OpenAI is also treated as an OpenAI-compatible provider preset.

Set `OPENAI_API_KEY` before using it:

```sh
export OPENAI_API_KEY='...'
```

Do not paste API key values into README, logs, transcripts, or config files.

Usage:

```sh
origin models --provider openai
origin chat --provider openai --model zai-org-glm-5-1 "hello"
origin propose --provider openai --model zai-org-glm-5-1 "fix typo" README.md
```

## Propose Usage

Preview a diff without changing files:

```sh
origin propose "fix typo" README.md
```

Preview a diff with background from a previous session:

```sh
origin propose --resume latest "fix typo" README.md
origin propose --resume <session-id> "fix typo" README.md
```

`--resume` adds a summarized previous session as background context only. `propose` still reads the current files you pass on the command line and must use those current file contents as the source of truth.

Run `git apply --check` without changing files:

```sh
origin propose --check "fix typo" README.md
```

Ask for approval before applying:

```sh
origin propose --apply "fix typo" README.md
```

Retry once when the model returns a diff rejected by target or integrity validation:

```sh
origin propose --retry-invalid "fix typo" README.md
origin propose --retry-invalid --check "fix typo" README.md
```

`--retry-invalid` is opt-in. The default `propose` command does not retry. Retry feedback is based on the CLI validation error, and the retried diff must pass the same scope and integrity validation before it can be displayed, checked, or applied.

Retry once when the diff passes CLI validation but fails `git apply --check`:

```sh
origin propose --retry-apply-check "fix typo" README.md
origin propose --retry-apply-check --check "fix typo" README.md
origin propose --retry-apply-check --apply "fix typo" README.md
```

`--retry-apply-check` is opt-in and only reacts to `git apply --check` failure. `--retry-invalid` is for target/integrity validation failure; `--retry-apply-check` is for patch application check failure. If both are specified, each retry path can run at most once, so there is no unbounded retry loop.

Safety behavior:

- Default `propose` only displays the diff.
- `--check` runs `git apply --check` and does not apply.
- `--apply` still asks for confirmation.
- Only `y` or `yes` applies the patch.
- Pressing Enter rejects the patch.
- Proposed diffs are rejected if they touch files outside the requested file list.
- Delete file, rename, absolute path, and `..` diff targets are rejected in this MVP. New-file creation is supported only through the narrow validated new-file proposal path described above.
- Invalid unified diffs are rejected before `git apply --check`.
- If `git apply --check` fails, the patch is not applied.
- `git apply --check` failure retry is opt-in.
- Retry still runs target validation, integrity validation, and `git apply --check`.
- Retry does not bypass human approval.

## Sessions

List sessions:

```sh
origin sessions
```

Show the latest session:

```sh
origin sessions show --latest
```

Show a specific session:

```sh
origin sessions show <session-id>
```

Transcripts are saved as JSONL:

```text
~/.origin/sessions/YYYY/MM/<session-id>.jsonl
```

Saved:

- provider id
- base URL
- model id
- command type
- user prompt or instruction
- file path, byte count, and SHA-256
- model response
- diff proposal
- diff validation result
- retry-invalid result when used
- retry-apply-check result when used
- check/apply/reject/error result

Not saved:

- API key values
- Authorization headers
- `OPENAI_API_KEY` values
- full file contents

## Lightweight Resume

Resume is intentionally lightweight.

```sh
origin chat --resume latest "continue from previous context"
origin chat --resume <session-id> "continue from that session"
origin propose --resume latest "fix typo" README.md
```

What resume does:

- Reads an existing session transcript.
- Builds a short background context block.
- Adds that context to the next `chat` or `propose` prompt.
- Records a `resumed_from` event in the new session transcript.

What resume does not do:

- It does not fully restore a conversation.
- It does not restore old file contents.
- It does not assume current files are unchanged.
- It does not apply old diffs.
- It does not change patch approval behavior.

## Safety Principles

- No automatic apply by default.
- Human approval is required for `--apply` and shell/TUI `/apply`.
- `y` or `yes` is required to approve.
- Enter rejects.
- Workspace-outside paths are rejected.
- Directories are rejected.
- Binary files are rejected.
- Files above the configured MVP size limit are rejected.
- API keys and Authorization headers are not saved.
- Diff target validation and diff integrity validation run before `git apply --check`.
- `--retry-invalid` is explicit and retries at most once.
- `--retry-apply-check` is explicit, retries at most once, and does not bypass approval.

## Current Limitations

- Full conversation resume is not implemented.
- Shell-side editing is limited to the minimal `/propose` -> `/check` -> `/apply` file-scoped loop.
- TUI supports chat, session-local model/provider switching, Ask/Agent mode, and a single-file edit loop.
- Multi-file TUI propose is not implemented.
- TUI model/provider switching is session-local and does not rewrite config.
- TUI sessions viewer is not implemented.
- TUI file picker is not implemented.
- MCP is not implemented.
- Repo-wide context selection is not implemented.
- Streaming responses are not implemented.
- TUI composer PageUp / PageDown editing behavior, Home / End, word-wise movement, selection, soft-wrap-aware vertical movement, and full grapheme-cluster editing are not implemented. PageUp / PageDown are reserved for message history scrolling.
- Auto-apply is not implemented.
- Config auto-writing is not implemented.
- Broad delete file and rename patch support are intentionally deferred.
- Agent mode is still single-file; broad multi-file Agent edits are deferred.

## Development

GitHub Actions runs Rust CI on push and pull request:

- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test --workspace`

The CI does not call LM Studio localhost, OpenAI API, or any command that requires `OPENAI_API_KEY`.

Check the workspace:

```sh
cargo check --workspace
```

Run tests:

```sh
cargo test --workspace
```

Run the development binary:

```sh
cargo run -p origin-cli -- --help
```

## Reference Codebases

`ai-cli-research/`(codex / aider / gemini-cli)は、設計比較のためローカルで参照した read-only の先行実装。**このリポジトリには含まれない**(`.gitignore` で除外)。
