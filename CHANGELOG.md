# Changelog

## Unreleased

- TUI Ask and Agent read-only provider context now includes `<cwd>/ORIGIN.md` as read-only workspace instructions when that file exists and passes validation. The file is loaded once at TUI startup, cached for the session, and shown in `/status` as loaded, skipped, or none.
- ORIGIN.md transcript handling records metadata only (`ORIGIN.md`, bytes, SHA-256, loaded/skipped status, and skip reason) and never saves the instruction body. Absent `ORIGIN.md` remains silent and writes no transcript event.
- Safety and approval behavior is unchanged: ORIGIN.md cannot trigger commands, routing changes, proposal generation, writes, auto-apply, or approval bypass.
- Agent mode now uses a model-driven action planner instead of a Rust verb/confirmation lexicon. The planner may only return read-only answer, clarification, one existing-file update, one new-file create, cancel, or no action.
- Agent planner output is validated by origin before any review opens. Unsafe, invalid, missing, malformed, or multi-file proposals fall back to clarification; create requests no longer infer a default output path.
- Agent planner rationale can appear in Agent review/clarification panels, and transcripts record planner action/path metadata only. Proposal review and the `y/n` write gate are unchanged.
- Agent review and destination clarification number keys now run their selected action immediately; `Up` / `Down` plus `Enter` still works, and Proposal review `y/n` approval is unchanged.
- TUI mouse capture is disabled so native terminal selection/copy works normally. Message history scrolling is on `PageUp` / `PageDown`, with older/newer indicators updated accordingly.
- Agent existing-file previews for files up to 64 KiB now ask the model for the complete updated file body and construct the single-target unified diff locally. Larger files ask for a narrower target instead of falling back to raw model diffs.

## v1.3.0 - 2026-06-07

Local-first TUI workbench release. This consolidates the post-v1.2.0 Phase 23.3–23.5.6 work into one user-facing milestone: a responsive non-blocking TUI, natural-language Ask / Agent modes with safety-gated proposal review, a selectable model route picker, a unified left-rail visual grammar, a curated command surface, and a mascot startup landing. The safety model, approval semantics, and `/diff` / `/check` / `/apply` behavior are unchanged.

### Responsiveness

- TUI chat requests now run on a background worker thread, so the event loop keeps rendering and polling input while waiting for the model; the draft input stays editable while a response is in flight.
- Pressing `Enter` while a response is in flight no longer submits a second prompt, and the "chat response is still in progress" warning is shown once per in-flight request instead of repeating.
- Ask / Agent read-only waits and Agent preview / save / check / write progress render as current-state running panels with a compact spinner-style indicator and elapsed seconds.
- The message pane supports mouse-wheel / trackpad scrolling with subtle older/newer indicators while the composer stays fixed; long output preserves block headers and tail-displays the latest useful result or error when terminal height is limited.
- Provider timeout wording now describes waiting for a provider response rather than a connection-only timeout, and OpenAI-compatible parsing reports reasoning-only or empty assistant responses as invalid instead of showing a blank message.

### Ask / Agent workflow

- Session-local Ask / Agent interaction mode via `/mode`, with `/mode ask` and `/mode agent` for explicit selection (default Ask).
- Ask mode is read-only: it can include explicitly mentioned safe files and a bounded read-only workspace overview, treating the current directory as a generic workspace (not always a git repository) and never creating a pending proposal.
- Agent mode classifies each natural-language prompt and routes to read-only chat, one explicit existing-file edit target, or a single suggested `.md` new-file path — it does not open a file target picker or scan generic candidates.
- A dedicated Agent review panel precedes any proposal provider call, with numbered actions (generate preview / adjust or other instruction / cancel) using input-buffer + `Enter` confirmation; a digit highlights an action and `Up` / `Down` moves the highlight, but a bare digit does not execute.
- A dedicated Proposal review panel is the write/apply permission gate after generation, using save / create / update wording with vertical `y` / `n`, and keeps `d` diff and `r` revise behind a `? more actions` reveal.
- New-file proposals ask the model for the file body and construct the `/dev/null` unified diff locally instead of trusting model hunk headers; invalid diffs retry once with validation feedback, empty new-file proposals are rejected, and a single validated new file can be applied safely in non-git workspaces.
- A bounded, in-memory in-session context lets short follow-ups such as `どう？`, `はい`, `ok`, `お願い`, `進めて`, `continue`, or `go ahead` continue or clarify a prior Agent task; cross-run resume, persistent Agent plans, and automatic target carryover remain deferred.
- Agent preview generation runs in the background and can be interrupted with `Esc` / `Ctrl+C`, ignoring any late provider result with no proposal stored and no files changed.

### Model route selection

- `/models` is the primary selectable provider/model route picker, showing current, configured/default, and live known-provider routes while keeping unavailable providers visible with safe redacted diagnostics.
- Routes are numbered `1.`, `2.`, `3.`, … with a numeric input buffer so `12` is not treated as `1`; `Up` / `Down` moves the highlight, `Enter` confirms, `Backspace` edits the buffer, `Esc` cancels, and there is no `>` marker.
- `/model list` stays as a legacy-compatible alias; `/model set <provider>/<model-id>`, `/model set <model-id>`, `/provider list`, and `/provider set` remain direct fallback/advanced routes. Route switching is session-local and never rewrites `~/.origin/config.toml`.

### Visual hierarchy / TUI polish

- A shared TUI style/component layer (`crates/origin-cli/src/tui/style.rs`) provides a single rail-aware row primitive, panel/action-list views, and selection/footer helpers, so live Agent / Proposal / Result / waiting panels render through one path.
- Live panels use left-rail tone styling with reduced box-drawing; the canonical selected-row token is a blue/cyan accent (no background block, no `>` / `›` marker) shared across panels, `/models`, and the command palette.
- The command palette and composer drop box-drawing borders; the composer is a light input area with a subtle background fill, a bright `›` prompt, and blank spacer padding, and it dims with a keyboard-owner affordance when a menu/panel owns input.
- Message history is reskinned to rail-less `you` / assistant rows plus left-rail `action` / `waiting` / `error` system blocks, replacing the old submitted-input background block and uppercase log headers.
- Composer rendering handles Japanese/full-width display width and anchors the IME cursor at the input cell, and supports `Left` / `Right` and logical-line `Up` / `Down` cursor movement, insertion, and `Backspace`.

### Command surface cleanup

- Running `origin` with no subcommand now starts the TUI workbench; the old interactive shell remains available as `origin shell`.
- The daily root `/` palette is curated to `/models`, `/mode`, `/resume`, `/status`, `/help`, and `/exit`; advanced and fallback commands remain direct-typeable and prefix-completable.
- `/help` groups commands into Daily, Advanced / direct, and Fallback / direct edit sections.
- `/resume` is a planned slot that prints a coming-soon message pointing to `origin sessions` / `origin chat --resume latest`, without restoring state or changing files.
- `/sessions` is demoted from the daily palette to an advanced/history command, still fully available by direct typing, prefix completion, and `/help`.

### Startup landing

- The startup empty state shows a compact block-art origin mascot with a `origin-agent — local-first AI workbench` wordmark and a single `Type a message to start.` hint.
- The old duplicated provider/model/git status block, the `/provider list` hint, and the `/propose -> /diff -> /check -> /apply` pipeline cheatsheet are removed from the empty state — they remain available by direct typing and in `/help` — and the landing falls back to wordmark + hint on short terminals.

### Safety / Unchanged guarantees

- No auto-apply. Approval semantics are unchanged: plan/action selection is not write approval, and Proposal review `y` / `n` remains the write/create/update permission gate, with `Enter` approval accepted only inside an explicit permission panel.
- `/diff`, `/check`, and `/apply` remain manual fallback / direct / debug commands; `/check` runs `git apply --check` only, and `/apply` reruns `git apply --check` and requires explicit `y` / `yes`.
- Non-git apply is limited to safe single new-file proposals; existing-file edits still require Git-backed apply. Out-of-scope, delete, rename, and invalid diffs are rejected, and multi-file Agent edits remain unsupported.
- `/resume` restores no sessions, reuses no old diffs, restores no pending proposal, calls no provider, and changes no files.
- Auto-apply, provider auto-routing, config auto-writing, MCP, and streaming responses remain unimplemented.
- API keys, Authorization headers, Bearer tokens, and `OPENAI_API_KEY` values are not displayed or saved; full file contents and auto-discovered workspace trees are not written to transcripts.

### Notes

- This release consolidates Phase 23.3 through Phase 23.5.6.
- Phase 23.5.6 is the startup mascot landing. An earlier draft used the "23.5.6" label for an Agent / Proposal input-model unification (removing instant `y` / `n` and routing both panels through the composer); that work is not part of v1.3.0 and is deferred to post-v1.3 follow-up. Proposal review continues to handle `y` / `n` directly, with `Enter` as a non-approving hint.
- Internal-only TUI module and edit-flow refactors are included with no behavior change.
- Documentation clarified the core product thesis (aider-like provider/model freedom, Codex / Gemini CLI-like terminal workbench UX, safety-first explicit-scope approval) and the post-Phase 23.3 Ask / Agent UX direction.

## v1.2.0

TUI input composer UX release for daily-use local-first work.

### Changed

- TUI composer now supports `Ctrl+J` newline input while keeping `Enter` as submit.
- Multiline chat prompts are submitted as one prompt.
- TUI composer now uses clearer placeholder text, prompt marker, continuation-line spacing, and tail display for longer input.
- TUI help and footer mention `Enter` submit and `Ctrl+J` newline behavior.
- Multiline slash commands are rejected with a clear unsupported message instead of being parsed ambiguously.

### Security / Safety

- TUI `/apply` approval semantics are unchanged.
- `y` / `yes` is still required to apply a patch.
- `Enter`, `n`, `no`, or any other approval input still rejects.
- `Ctrl+J` does not approve `/apply`.
- Auto-apply is not implemented.
- Provider auto-routing is not implemented.
- Config auto-writing is not implemented.
- API keys, Authorization headers, Bearer tokens, and secret values are not displayed or saved.

### Limitations

- Multi-file TUI `/propose` is not implemented.
- TUI sessions viewer is not implemented.
- Streaming responses are not implemented.
- TUI file picker is not implemented.
- Full TUI multiline editor features such as cursor movement within the buffer are not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Auto-apply is not implemented.
- Config auto-writing is not implemented.
- Advanced resume is not implemented.
- New/delete/rename patch support remains unsupported.

## v1.1.0

Daily-use TUI UX release after the v1.0.0 baseline.

### Changed

- TUI footer now shows state-aware next-action hints.
- TUI input now shows slash command suggestions while typing `/`.
- TUI chat pane uses clearer role labels and spacing for user, assistant, system, and error messages.
- TUI `/status` now shows provider routing/tradeoff hints for local, OpenAI, and custom OpenAI-compatible providers.

### Security / Safety

- TUI `/apply` semantics are unchanged.
- Human approval requirements are unchanged.
- Auto-apply is not implemented.
- Provider auto-routing is not implemented.
- Config auto-writing is not implemented.
- API keys, Authorization headers, Bearer tokens, and secret values are not displayed or saved.

### Limitations

- Multi-file TUI `/propose` is not implemented.
- TUI sessions viewer is not implemented.
- Streaming responses are not implemented.
- TUI file picker is not implemented.
- TUI multiline editor is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Auto-apply is not implemented.
- Config auto-writing is not implemented.
- Advanced resume is not implemented.
- New/delete/rename patch support remains unsupported.

## v1.0.0

First stable release of the safety-approved TUI file edit workflow.

### Added

- TUI chat with the selected local/API model.
- Session-local provider/model switching in the TUI.
- TUI `/propose <file> <instruction...>` for explicit single-file diff proposals.
- TUI `/diff` for pending diff inspection.
- TUI `/check` with `git apply --check`.
- TUI `/apply` with `git apply --check` rerun and explicit `y` / `yes` approval.
- Session transcript logging for TUI chat and edit-loop operations.

### Security / Safety

- TUI `/propose` previews only.
- TUI `/check` never modifies files.
- TUI `/apply` reruns `git apply --check` before approval.
- TUI `/apply` applies only after explicit `y` / `yes` approval.
- Enter, `n`, `no`, or any other approval input rejects the patch.
- Rejected apply keeps the pending proposal.
- Successful apply clears the pending proposal.
- Transcript events are redacted before being written to JSONL.
- API keys, Authorization headers, Bearer tokens, and `OPENAI_API_KEY` values are not displayed or saved.
- Full file contents are not saved to transcript.

### Limitations

- v1.0.0 is not a full Codex / Claude Code replacement.
- Multi-file TUI `/propose` is not implemented.
- TUI sessions viewer is not implemented.
- Streaming responses are not implemented.
- TUI file picker is not implemented.
- TUI multiline editor is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Auto-apply is not implemented.
- Config auto-writing is not implemented.
- Advanced resume is not implemented.
- New/delete/rename patch support remains unsupported.

## v0.9.0

Release-candidate polish before v1.0.

### Changed

- Polished the canonical TUI demo flow in the README.
- Polished TUI `/help` wording for the current chat, model/provider, and edit-loop commands.
- Improved TUI error wording so common failures explain what happened and what to do next.
- Hardened transcript writing so sensitive text values are redacted before storage.
- Added and updated tests for TUI error wording and transcript safety.

### Security / Safety

- Transcript events are redacted before being written to JSONL.
- TUI chat, propose, check, and apply error displays continue to redact sensitive text.
- API keys, Authorization headers, Bearer tokens, and `OPENAI_API_KEY` values are not displayed or saved.
- Full file contents are not saved to transcript.
- TUI `/apply` safety semantics are unchanged: `git apply --check` is rerun and only explicit `y` / `yes` approval applies a patch.

### Limitations

- Multi-file TUI `/propose` is not implemented.
- TUI sessions viewer is not implemented.
- Streaming responses are not implemented.
- TUI file picker is not implemented.
- TUI multiline editor is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Auto-apply is not implemented.
- Config auto-writing is not implemented.
- New/delete/rename patch support remains unsupported.

## v0.8.0

TUI file edit loop release.

### Added

- TUI `/propose <file> <instruction...>` for preview-only single-file diff proposals.
- TUI `/diff` to redisplay the current pending proposal.
- TUI `/clear` to discard the current pending proposal and clear visible TUI messages.
- TUI `/check` to run `git apply --check` against the pending proposal without modifying files.
- TUI `/apply` with `git apply --check` rerun and explicit `y` / `yes` approval.
- Pending proposal state for the TUI edit loop.

### Changed

- TUI `/propose` uses the shared propose preview workflow.
- TUI `/check` and `/apply` record session transcript events for check, approval, apply, reject, and errors.
- TUI model/provider changes clear pending proposals to avoid applying a diff created under a different session state.

### Security / Safety

- TUI `/propose` previews only.
- TUI `/diff` never reads files, calls providers, runs checks, or applies patches.
- TUI `/check` only runs `git apply --check` and never modifies files.
- TUI `/apply` reruns `git apply --check` before approval.
- TUI `/apply` applies only after explicit `y` / `yes` approval.
- Enter, `n`, `no`, or any other approval input rejects the patch.
- Rejected apply keeps the pending proposal.
- Successful apply clears the pending proposal.
- API keys and Authorization headers are not displayed or saved.

### Limitations

- Multi-file TUI `/propose` is not implemented.
- TUI sessions viewer is not implemented.
- Streaming responses are not implemented.
- TUI file picker is not implemented.
- TUI multiline editor is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Auto-apply is not implemented.
- Config auto-writing is not implemented.
- New/delete/rename patch support remains unsupported.

## v0.7.0

TUI slash commands and session-local TUI provider/model switching release.

### Added

- TUI slash command handling.
- TUI `/help`, `/status`, `/model`, `/provider`, `/sessions`, `/clear`, `/exit`, and `/quit`.
- TUI `/model list` and `/provider list`.
- TUI `/model set <model-id>` and `/provider set <provider-id>` for session-local switching.

### Changed

- TUI slash input is treated as command input; non-slash input remains chat.
- TUI chat uses the switched provider/model for the current TUI session.
- TUI model/provider switching does not rewrite config.
- Project docs now delegate fast-changing usage details to `README.md` and release history to this changelog.

### Security / Safety

- TUI model/provider switching is session-local.
- TUI still does not run propose/check/apply.
- TUI still does not modify files.
- API keys and Authorization headers are not displayed or saved.

### Limitations

- TUI file edit loop is not implemented.
- TUI sessions viewer is not implemented.
- TUI diff viewer is not implemented.
- Streaming responses are not implemented.
- TUI file picker is not implemented.
- TUI multiline editor is not implemented.

## v0.6.0

TUI prototype release.

### Added

- `origin tui` command.
- TUI prototype using `ratatui` and `crossterm`.
- TUI status pane.
- TUI chat pane.
- TUI input area.
- TUI footer.
- TUI chat input.
- TUI chat transcript logging.

### Changed

- TUI can send chat prompts through the existing chat workflow.
- TUI shows user, assistant, system, and error messages.
- TUI exits safely with `q`, `Esc`, or `Ctrl+C`.
- Runtime state and chat workflow are now reusable by TUI.

### Security / Safety

- TUI does not run propose/check/apply yet.
- TUI does not modify files.
- TUI does not display or save API keys.
- TUI uses existing transcript safety behavior.
- Terminal raw mode and alternate screen cleanup are handled.

### Limitations

- TUI file edit loop is not implemented.
- TUI model/provider switching is not implemented.
- TUI sessions viewer is not implemented.
- TUI diff viewer is not implemented.
- Streaming responses are not implemented.
- TUI multiline editor is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.

## v0.5.0

Shell workflow polish and multi-file shell propose release.

### Added

- Shell `/propose` now supports multiple files with `--` separator.
- Example: `/propose README.md CHANGELOG.md -- align release wording`.

### Changed

- Shell `/help` is organized by category.
- Shell `/status` uses sectioned display for runtime/provider/model/config/pending proposal.
- Shell `/model` and `/provider` displays are easier to read.
- Shell `/sessions` shows recent sessions with a latest marker.
- Shell startup intro now shows provider/model/git state and common next commands.
- Shell error messages now explain what happened and what to do next.
- Shell edit loop UX is clearer:
  - `/propose`
  - `/diff`
  - `/check`
  - `/apply`
  - `/clear`

### Security / Safety

- Shell `/propose` still previews only.
- Shell `/check` still does not modify files.
- Shell `/apply` still requires `y` / `yes` approval.
- Multi-file proposal still validates requested file scope.
- API keys and Authorization headers are not displayed or saved.

### Limitations

- TUI is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Streaming responses are not implemented.
- New/delete/rename patch support remains unsupported.

## v0.4.1

Shell edit loop UX improvement release.

### Added

- Shell `/diff` to redisplay the current pending proposal.
- Shell `/clear` to discard the current pending proposal.

### Changed

- README and shell help now document `/diff` and `/clear`.
- Shell edit loop is clearer:
  - `/propose`
  - `/diff`
  - `/check`
  - `/apply`
  - `/clear`

### Security / Safety

- Shell `/diff` never changes files.
- Shell `/clear` never applies changes.
- Pending proposal state is explicit and discardable.
- API keys and Authorization headers are not displayed or saved.

### Limitations

- TUI is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Streaming responses are not implemented.
- New/delete/rename patch support remains unsupported.

## v0.4.0

Interactive shell file edit loop release.

### Added

- Shell `/propose <file> <instruction...>`.
- Shell `/check`.
- Shell `/apply`.
- Pending proposal state in the interactive shell.
- Shell edit loop: `/propose` -> `/check` -> `/apply`.
- Transcript logging for shell propose/check/apply.
- Shell help updates for the edit workflow.

### Changed

- Shell UX now explains pending proposal created / kept / cleared states.
- `/model set` and `/provider set` clear pending proposals.
- README documents the shell editing workflow.

### Security / Safety

- Shell `/propose` previews only.
- Shell `/check` never changes files.
- Shell `/apply` re-runs `git apply --check`.
- Shell `/apply` requires `y`/`yes` approval.
- Reject keeps the pending proposal.
- Apply success clears the pending proposal.
- API keys and Authorization headers are not displayed or saved.

### Limitations

- Shell-side new/delete/rename patch support remains unsupported.
- TUI is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Streaming responses are not implemented.

## v0.3.1

Patch release for GitHub Actions CI.

### Added

- GitHub Actions Rust CI workflow.
- CI runs on `push` and `pull_request`.
- CI checks:
  - `cargo fmt --all --check`
  - `cargo check --workspace`
  - `cargo test --workspace`

### Notes

- CI does not call LM Studio localhost.
- CI does not call OpenAI API.
- CI does not require `OPENAI_API_KEY`.
- `actions/checkout@v4` currently emits a Node.js 20 deprecation warning, but the workflow passes.

## v0.3.0

Interactive shell prototype release.

### Added

- Interactive shell prototype via `origin` with no subcommand.
- Shell state for provider, model, base URL, config path, session root, current working directory, and git state.
- Shell commands:
  - `/help`
  - `/status`
  - `/model`
  - `/model list`
  - `/model set <model-id>`
  - `/provider`
  - `/provider list`
  - `/provider set <provider-id>`
  - `/sessions`
  - `/exit`
  - `/quit`
- Chat from normal shell input.
- Session transcript logging for shell chat.

### Changed

- `origin` without a subcommand now starts the prototype interactive shell.
- Shell model and provider switching is session-local.

### Security / Safety

- Shell model/provider switching does not rewrite config files.
- API keys and Authorization headers are not displayed or saved.
- Shell chat uses the existing transcript safety model.

### Limitations

- Shell-side `propose`, `check`, and `apply` are not implemented.
- TUI is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Streaming responses are not implemented.

## v0.2.1

Small usability release after the v0.2.0 rename.

### Added

- `origin status` to show current provider, model, config, session, cwd, and git state.
- `origin model` to show the currently resolved model.
- `origin model list` as the command-group equivalent of `origin models`.
- `origin provider` to show the currently resolved provider.
- `origin provider list` as the command-group equivalent of `origin providers`.
- `origin propose --retry-apply-check` for one explicit retry when a diff passes CLI validation but fails `git apply --check`.

### Changed

- Status output now summarizes provider/model/config/session/git state.
- Model and provider commands expose current resolution state before moving toward interactive model/provider selection.

### Security / Safety

- `--retry-apply-check` is opt-in.
- Apply-check retry still runs target validation, integrity validation, and `git apply --check`.
- Apply-check retry does not bypass human approval.

### Limitations

- Interactive shell is not implemented.
- Slash commands are not implemented.
- TUI is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.

## v0.1.0

Usable MVP for a lightweight, file-scoped AI editing CLI built around OpenAI-compatible providers.

### Added

- LM Studio provider preset.
- OpenAI provider preset.
- OpenAI-compatible `/v1/chat/completions` support.
- `origin chat` command for single prompt chat.
- `origin propose` command for file-scoped unified diff proposals.
- `origin propose --check` for `git apply --check` without applying.
- `origin propose --apply` with human approval before `git apply`.
- Session transcript logging to JSONL.
- `origin sessions` list command.
- `origin sessions show --latest` and `origin sessions show <session-id>`.
- Lightweight `--resume <session-id|latest>` context for `chat` and `propose`.
- User config loader for `~/.origin/config.toml`.
- `ORIGIN_CONFIG` override for temporary config paths.
- Provider/model/base URL resolution across CLI args, env, config, and built-in defaults.
- Model fallback safety to avoid embedding-like models by default.
- Diff target validation for requested file scope.
- Diff integrity validation before `git apply --check`.
- `--retry-invalid` for one explicit retry when model output fails diff validation.
- Example config files for LM Studio and LM Studio + OpenAI.

### Security / Safety

- API key values are not stored in config examples or session transcripts.
- Authorization headers are not logged.
- `OPENAI_API_KEY` is treated as an environment variable name, not a stored secret value.
- Default `propose` mode never applies patches.
- `--apply` requires explicit human approval.
- Only `y` or `yes` approves patch application.
- Enter rejects patch application.
- Workspace-outside paths are rejected.
- Directory inputs are rejected.
- Binary files are rejected.
- Large files are rejected by the MVP file size limit.
- Diff targets outside the requested file scope are rejected.
- New file, delete file, and rename diffs are rejected in this MVP.
- Invalid unified diffs are rejected before `git apply --check`.

### Limitations

- Full conversation resume is not implemented.
- TUI is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- Streaming responses are not implemented.
- New file, delete file, and rename patch support are not implemented.
- Multi-file edits are limited to explicitly requested files.
