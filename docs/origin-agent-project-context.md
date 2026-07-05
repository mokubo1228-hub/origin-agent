# origin-agent Project Context

## 1. Project Overview

Project name: `origin-agent`  
CLI command: `origin`

`origin-agent` is a safety-first AI editing CLI for LM Studio, OpenAI API, and other OpenAI-compatible providers.

Core product thesis:

```text
origin-agent combines aider-like model/provider freedom,
Codex / Gemini CLI-like terminal workbench UX,
and origin-agent's own safety-first, explicit-scope,
human-approved edit workflow.
```

Provider/model flexibility is intentional, not incidental. Terminal AI workbench UX is intentional, not cosmetic. Safety-first remains the default behavior constraint: the goal is not to become an unsafe autonomous coding agent, and the goal is not to stay intentionally weak.

It is not a full replacement for Codex, Claude Code, or Gemini CLI today. Those tools remain the right choice for serious implementation work, large multi-file edits, fast cloud models, and mature coding-agent workflows.

This does not mean origin-agent should stay intentionally weak or degraded. The goal is to improve its UX and workflow toward Codex / Claude Code / Gemini CLI / aider where that fits a safety-first, explicit-scope design. As stronger local or OpenAI-compatible models become available, origin-agent should be able to benefit from them and raise its practical ceiling.

`origin-agent` is a smaller, self-controlled tool for:

- using local LLMs through LM Studio
- using OpenAI / GLM and other OpenAI-compatible APIs
- chatting with selected local/API models
- proposing explicit file edits as unified diffs
- validating diffs before application
- applying patches only after explicit human approval
- preserving session transcripts
- understanding and controlling the structure of an AI coding CLI
- eventually using it in livestreams as part of the origin brand

---

## 2. Brand / Naming

The public / product name is:

```text
origin-agent
```

The installed command is:

```bash
origin
```

Reasoning:

* `origin-agent` connects the tool to the origin brand.
* `origin` is short enough to use naturally as a CLI command.

Naming structure:

```text
Product / repo: origin-agent
Command:        origin
Config dir:     ~/.origin
Session dir:    ~/.origin/sessions
```

---

## 3. Core Positioning

`origin-agent` is a safety-approved AI editing CLI.

The v1.0 manual fallback / direct workflow is:

```text
chat
↓
explicit file propose
↓
diff preview
↓
check
↓
human approval
↓
apply
```

The central safety design principle is:

```text
The model may propose.
The tool validates.
The human approves.
Only then can files change.
```

Safety-first does not mean confirmation-heavy UX. It means origin-agent must not change external state without explicit execution approval, must keep scope bounded, must validate targets and patches before writing, must avoid exposing secrets, and must avoid broad autonomous workspace behavior by default.

The TUI direction is moving from command-first `/propose` usage toward a more natural Ask / Agent workbench:

```text
Ask = read-only questions with explicit safe file context or bounded workspace overview
Agent = natural-language single-file proposal flow, still safety-gated before writes
```

The intended normal Agent path should feel closer to Codex / Claude Code:

```text
natural prompt
↓
origin selects a reasonable plan, or asks the user to choose when multiple next actions exist
↓
generate the content/change
↓
show the generated result, preview, or summary inline
↓
ask whether to save/create/update it
↓
after explicit execution approval, run internal safety checks and write only if safe
```

Plan/action selection and execution approval are separate concepts:

```text
Plan/action selection = choose among possible next actions.
Execution approval = approve an external-state-changing action.
```

Plan/action selection should use numbered choices such as `1 / 2 / 3`, and should also support `Up` / `Down` selection plus `Enter` confirmation. When the presented choices may be wrong, include `Other / custom instruction`; put `Cancel` last. This is not file write approval and should not use `y / n`.

Execution approval should use `y / n` only when origin is about to change external state: create file, update file, delete file, write, or future mutating command execution. `y` means perform that external-state-changing action. `n` means do not perform it.

`/propose`, `/diff`, `/check`, and `/apply` remain available as explicit fallback / direct / debug commands. They should not dominate the normal Agent UX or require users to understand internal patch mechanics.

---

## 4. Post-v1 Direction

The v1.0.0 baseline is complete. The next priority is not adding large autonomous features.

Post-v1, origin-agent should become the daily entry point for lightweight local-first AI work:

```text
Use origin first for light, private, routine, file-scoped work.
Use larger cloud coding agents only when the task actually needs them.
```

The intended routing ladder is:

1. Local LLM through LM Studio

   * strongest privacy
   * uses owned local compute
   * does not spend cloud AI quota
   * best for lightweight tasks, docs, small edits, private notes, routine work

2. OpenAI / OpenAI-compatible external API

   * middle option when local models are not enough
   * can preserve Codex / Claude Code usage budget
   * useful for moderately hard work that does not require a full coding agent

3. Codex / Claude Code / high-performance cloud AI

   * reserved for performance-critical work
   * best for large implementation, difficult coding, broad repo understanding
   * not something origin-agent tries to fully replace

The main post-v1 friction is daily-use UX, not missing core safety functionality.

Important post-v1 UX directions:

* slash command suggestions / command palette
* command descriptions while typing `/`
* state-aware next-action hints
* chat panel and message layout polish
* clearer user / assistant / system / error separation
* provider routing UX
* status views that explain why the current provider/model is selected
* visible local / OpenAI / cloud tradeoffs for privacy, cost, quota, and performance
* pending proposal state that is easy to understand at a glance

Current post-v1 work has shifted from generic daily-use polish toward TUI visual convergence:

* keep the interface local-first / OpenAI-compatible / safety-first
* make the TUI feel more like a terminal AI workbench instead of a debug/status dashboard
* keep the header thin, the message area readable, and the composer central
* use structured message/action blocks and latest-output-first rendering
* keep command palette and composer interactions discoverable without changing command semantics
* make natural-language prompts useful for lightweight read-only work before requiring slash commands

The current directory should be treated as a generic workspace, not only as a software repository. It may be:

* code repository
* documentation folder
* handoff docs folder
* business notes folder
* Markdown / Obsidian-style folder
* specs folder
* mixed docs/code workspace
* unknown workspace

Ask mode may build bounded read-only context from that workspace, but it must not recursively ingest the whole workspace or assume the git root is the context scope.

Still avoid by default:

* auto-apply
* repo-wide autonomous agent behavior
* MCP as a near-term default
* config auto-writing
* advanced resume / state restoration
* treating origin-agent as a full Codex / Claude Code replacement today

Important distinction:

```text
Safety constraints are default behavior constraints, not a low capability ceiling.
```

The limitation is not "keep the tool weak." The limitation is "do not default to unsafe autonomous behavior without explicit safety design."

Post-v1 improvements should raise usability and capability through:

* better composer and chat pane UX
* clearer command flow
* clearer generated preview, execution approval, checking, writing, done, blocked, and cancelled states
* explicit context selection
* Ask mode read-only context from explicit files and bounded workspace overview
* Agent mode natural-language proposal flow that reuses existing safety gates
* safe multi-file workflows
* local-first / OpenAI-compatible model routing
* workflows that scale upward when stronger models are available

---

## 5. v1.0 Definition

`origin-agent` reaches v1.0 when users can launch a TUI, chat with a selected local/API model, switch provider/model, propose edits for explicit files, inspect diffs, run checks, and apply changes only after human approval.

Japanese definition:

```text
origin-agent は、TUI上でローカル/APIモデルを切り替えながらチャットし、
明示指定したファイルに対して diff 提案・確認・承認 apply まで
安全に実行できる状態を v1.0 とする。
```

v1.0 is not a full Codex / Claude Code replacement today.

That positioning should not be read as an intentional low ceiling. origin-agent can and should become more capable as UX, explicit context selection, safe file workflows, and available local/API models improve.

v1.0 does not require:

* autonomous agent behavior
* repo-wide automatic context selection
* MCP
* full resume / state restoration
* plugin system
* auto-apply
* new/delete/rename patch support
* provider-specific parameter deep support

---

## 6. Session / Resume Definitions

### session

A `session` is an execution record.

It is saved as JSONL under:

```text
~/.origin/sessions/YYYY/MM/<session-id>.jsonl
```

A session may record:

* command
* provider
* model
* user message / instruction
* file path / bytes / sha256
* Ask mode file/workspace context metadata
* model response
* diff proposal
* validation result
* check result
* approval decision
* apply result
* error result

A session must not store:

* API key values
* Authorization headers
* `OPENAI_API_KEY` value
* full file contents
* full auto-discovered workspace tree contents

Short definition:

```text
session = record / log / note
```

### resume

`resume` means using a previous session as background context for a later chat or propose operation.

It does not restore previous state.

It does not automatically reuse old diffs.

It does not automatically restore a pending proposal.

It does not apply anything.

Short definition:

```text
resume = use a previous session as background context
```

### advanced resume

Advanced resume would mean restoring actual working state, such as:

* previous pending proposal
* previous provider/model/shell state
* previous opened file set
* old diff reuse
* continuing from old check/apply state

This is intentionally out of scope for v1.0.

Reason:

```text
Past session state may not match current file state.
Old diffs may be unsafe against current files.
```

Advanced resume is a v1.1+ candidate.

---

## 7. Release History

Detailed release history is maintained in `CHANGELOG.md`.

Major milestones:

- v0.1.0: usable MVP
- v0.3.0: interactive shell prototype
- v0.4.0: shell file edit loop
- v0.5.0: shell workflow polish
- v0.6.0: TUI prototype
- v0.7.0: TUI slash commands and session-local provider/model switching
- v0.8.0: TUI file edit loop
- v0.9.0: TUI release-candidate polish
- v1.0.0: first stable safety-approved TUI file edit workflow

This section is intentionally high-level. Check `CHANGELOG.md` for the latest version.

---

## 8. Current Capabilities

See `README.md` and `CHANGELOG.md` for the latest command list.

At a high level, origin-agent supports:

- plain CLI usage
- interactive shell usage
- TUI workflow usage
- provider/model switching
- selectable TUI model route picker via `/models`
- session-local TUI Ask / Agent mode via `/mode`
- read-only Ask mode explicit safe file-path context
- read-only Ask mode bounded workspace overview context
- Agent mode natural-language single-file proposal review
- Agent mode immediate prior in-session context
- Agent mode explicit Ask-vs-Agent provider context for read-only fallback
- Agent mode short confirmations can continue or clarify prior Agent tasks when appropriate
- Agent mode new-file proposal using model body output with local unified-diff construction
- dedicated Agent review and Proposal review panels
- safe single new-file apply in non-git workspaces
- explicit file propose/check/apply workflow
- session transcript logging
- lightweight resume

---

## 9. Safety Model

Safety rules:

* default does not modify files
* safety-first is not confirmation-heavy UX
* external-state-changing execution requires explicit approval before create/update/delete/write
* plan/action selection is not execution approval and should not use `y / n` in the intended Agent UX
* `/propose` previews only
* `/check` runs `git apply --check` only
* `/apply` re-runs `git apply --check`
* `/apply` requires `y` / `yes`
* Agent review uses fixed action selection for preview generation / adjustment / cancellation; `y / n` is not the Agent review action path
* Proposal review `y` is the approval point for actual write/apply
* Proposal review may support `Up` / `Down` plus `Enter` only inside an explicit permission panel
* non-git apply is limited to safe single new-file proposals
* Enter / `n` / `no` rejects
* failed `git apply --check` blocks apply
* out-of-scope diff targets are rejected
* invalid unified diffs are rejected
* broad new/delete/rename patch support is not implemented; Agent new-file creation is the narrow safe exception
* API keys are not displayed or saved
* Authorization headers are not displayed or saved
* full file contents are not saved to transcript
* full auto-discovered workspace tree contents are not saved to transcript
* config is not rewritten automatically
* model/provider switching is session-local unless explicitly changed in config
* Ask mode context is read-only and is not an edit target
* workspace overview context is bounded and does not recursively ingest the whole workspace

---

## 10. Release Policy

Phases can be small.

Tags should be coarser and based on meaningful user-facing milestones.

Commit-level changes:

* small implementation changes
* internal refactors
* display tweaks
* docs updates

Patch release examples:

* bug fixes
* CI / release maintenance
* documentation updates
* small UX improvements

Minor release examples:

* clearly new user-facing workflows
* interactive shell
* shell edit loop
* TUI prototype
* coherent multi-file workflow

Do not create a tag for every small Phase.

---

## 11. Current Architecture Notes

Important modules:

* `runtime.rs`

  * shared runtime provider/model/config/git/session state
* `chat_workflow.rs`

  * shared chat execution and transcript recording
* `status_snapshot.rs`

  * display-oriented status snapshot for shell/TUI
* `shell.rs`

  * interactive shell loop, slash command handling, shell-specific formatting
* `tui/mod.rs`

  * TUI workflow, slash command handling, chat, provider/model switching, Agent mode, and review state
  * this module is large; Agent review/proposal review code is a likely future split target
* `tui/edit_flow.rs`

  * TUI proposal/check/apply helpers, Proposal review apply behavior, and safe non-git new-file apply
* `commands.rs`

  * plain CLI orchestration
* `origin-provider`

  * OpenAI-compatible provider layer
* `origin-session`

  * transcript writing / reading
* `origin-tools`

  * file read, workspace safety, diff extraction, patch validation, git apply helpers

Recent refactors:

* `RuntimeState` extracted from shell
* `chat_workflow` extracted from shell chat
* `StatusSnapshot` extracted for shell/TUI status display
* TUI introduced after these refactors
* TUI composer and edit-flow code split out of the original monolithic TUI file

---

## 11.1 Current Agent Mode Direction

Phase 23.3 established the first Agent review workflow. Phase 23.4 (Ask / Agent mode-state correction) and Phase 23.5.1–23.5.6 are implemented and folded into the released `## v1.3.0` entry. Phase 24 (ORIGIN.md workspace instructions), Phase 25 (model-driven Agent action planner / intent classification), and Phase 26 (explicit model selection) then shipped in `v1.4.0`, now the latest released version.

Accepted Phase 23.4 interpretation:

* The main completed change is Ask / Agent mode-state correction, not merely UI polish.
* Agent mode understanding/state behavior is resolved enough for now.
* Remaining UI design concerns are deferred to later visual hierarchy work.

Implemented:

* Agent mode can create safe single new files through natural-language prompts.
* Agent mode can use immediate prior in-session context.
* Agent file picker was removed.
* Agent review uses a fixed 3-choice action menu before provider proposal generation.
* `y / n` is no longer Agent review action selection.
* Proposal review panel appears after proposal generation and before file writes.
* Proposal review remains the write/apply permission gate.
* Proposal review can support `Up` / `Down` plus `Enter` only inside an explicit permission panel.
* Agent preview generation runs in a background worker.
* `Esc` / `Ctrl+C` can interrupt Agent preview generation.
* Late provider results after cancellation are ignored.
* Normal provider waiting has clearer running-state UI.
* Ask / Agent mode contract is explicit in provider context.
* Agent read-only fallback uses an Agent-mode preface instead of Ask-mode-like wording.
* Short confirmations such as `はい`, `yes`, `ok`, `お願い`, `進めて`, `continue`, and `go ahead` can continue or clarify prior Agent tasks when appropriate.
* Ask mode and Agent mode can treat the same short input differently when Agent context exists.
* New-file Agent proposal asks the model for body content only; origin constructs the unified diff locally.
* Safe single new-file apply is supported outside Git repositories.
* `/diff`, `/check`, and `/apply` remain available as manual fallback commands.

Future visual hierarchy work should align the Agent / Proposal / Result UI toward:

* reuse `/models` route picker selection style where practical.
* prefer background/accent styling over literal `>` markers and heavy borders.
* keep the primary decision visually dominant.
* keep advanced/debug actions such as `d`, `r`, `/diff`, `/check`, and `/apply` secondary.
* keep normal wording such as `save`, `create`, `update`, `write`, `cancel`, and `no files changed`.
* consider numbered selection for `/models`, but use a numeric input buffer plus `Enter` confirmation so `12` is not accidentally treated as `1`.

Do not broaden the next visual hierarchy phase into multi-file Agent edits, resume, provider auto-routing, config auto-writing, MCP, auto-apply, or large module splitting.

Phase 23.5.1 through 23.5.6 are implemented and folded into v1.3.0: a shared TUI style/component layer (`tui/style.rs`), left-rail `render_panel` live panels with reduced box-drawing, the `? more actions` reveal, input-buffer + Enter Agent review selection, `/models` numbering + numeric buffer, composer / command palette de-bordering, command-surface cleanup with the planned `/resume` slot and `/sessions` demotion, the message-history reskin, and the startup mascot landing. These shipped in `v1.3.0` (superseded as latest by `v1.4.0`). The Phase 23.5 split is tracked in section 14.

---

## 12. v1.0 Baseline

The v1.0.0 baseline is complete.

At v1.0.0, origin-agent supports:

* TUI chat
* session-local provider/model switching
* explicit single-file TUI propose
* diff inspection
* `git apply --check`
* human-approved apply
* session transcript logging
* transcript redaction before JSONL write

The historical roadmap below records how that baseline was reached.

### v0.5.x

Shell UX polish.

Examples:

* `/sessions` display polish
* shell startup intro
* shell error wording
* shell help categories
* pending proposal visibility
* shell as TUI prototype semantics

### v0.6.x

TUI prototype.

Goal:

* TUI starts
* status pane
* chat pane
* input area
* chat works
* transcript saved

### v0.7.x

TUI provider/model switching.

Goal:

* TUI `/model list`
* TUI `/model set <model-id>`
* TUI `/provider list`
* TUI `/provider set <provider-id>`
* selected provider/model reflected in TUI chat
* config not rewritten

### v0.8.x

TUI file edit loop.

Goal:

* explicit files only
* propose
* diff view
* check
* approval apply
* pending proposal display
* clear

### v0.9.x

Release candidate polish.

Goal:

* demo flow fixed
* README / help cleaned
* provider failure handling improved
* transcript safety checked
* release candidate stabilized

### v1.0

Completed baseline:

```text
TUI上で chat + provider/model switching + explicit file edit loop が安全に通る。
```

---

## 13. Post-v1 Non-goals and Deferred Work

The following are still deferred after v1.0.0 unless explicitly scoped:

* advanced resume
* restoring pending proposal from old session
* reusing old diffs
* unbounded repo-wide / workspace-wide full ingest
* autonomous planning
* MCP
* deep provider-specific parameter support
* full new/delete/rename patch support
* config auto-writing
* plugin system
* auto-apply
* full Codex / Claude Code replacement

---

## 14. Near-term Roadmap Direction

Standing direction after v1.0.0:

```text
v1.0.1 bugfix / wording / safety fixes
↓
v1.1+ daily-use UX polish for local-first work
↓
Only then consider larger features
```

Good v1.1+ candidates include:

* slash command suggestions / command palette
* state-aware next-action hints
* chat panel and message layout polish
* provider routing UX
* local / OpenAI / cloud tradeoff visibility
* TUI sessions viewer
* multi-file TUI propose
* targeted TUI diff/navigation polish
* small provider failure handling improvements

Completed Phase 23.5 split (visual hierarchy / interaction grammar / command surface unification), shipped in v1.3.0:

* 23.5.1 + 23.5.2 — shared TUI style/component layer, left-rail live panels, `? more actions` reveal, input-buffer + Enter Agent review selection (plus a Proposal review footer microcopy fix). Implemented.
* 23.5.3 — `/models` selection style / numbering / numeric buffer. Implemented.
* 23.5.4 — composer / command palette de-bordering. Implemented.
* 23.5.5 — command-surface cleanup, planned `/resume` slot, `/sessions` demotion to advanced/direct, and message-history reskin. Implemented.
* 23.5.6 — startup mascot landing (compact block-art mascot, `origin-agent — local-first AI workbench` wordmark, `Type a message to start.` hint; the old status block and edit-loop pipeline removed from the empty state). Implemented.

All of the above are implemented and gathered into the released `## v1.3.0` CHANGELOG entry; the latest released tag is now `v1.4.0` (see below).

Shipped in `v1.4.0` (was the confirmed post-v1.3 sequencing):

* Phase 24 ORIGIN.md workspace instructions — `<cwd>/ORIGIN.md` is loaded once at startup and passed to Ask / Agent as read-only workspace background, cached for the session and surfaced in `/status`; the instruction body is never written to transcript and there is no safety/approval/routing change.
* Phase 25 Agent intent classification — Agent mode uses a model-driven action planner (read-only answer, clarification, one existing-file update, one new-file create, cancel, or no action), validated by origin before any review opens; unsafe, invalid, or multi-file output falls back to clarification.
* Phase 26 explicit model selection — the TUI/CLI no longer silently auto-selects a discovered model; the landing shows `No model selected — press /model to choose` and blocks sending until one is chosen, and non-interactive `origin chat` / `propose` prints an explicit `auto-selected` note instead of a silent pick.

Post-v1.4 direction:

* small follow-up polish continues — command palette / composer `›` marker consistency and header wording.
* future backlog (direction only, not scheduled) — agent memory / cross-session persistent context; no committed next phase until the product owner decides.
* deferred — the Agent / Proposal input-model unification (removing instant `y` / `n`) once labeled "23.5.6".

The existing safety model is unchanged: no auto-apply, write/apply approval stays the `y` / `n` permission gate, `/resume` restores no state and changes no files, and `/diff` / `/check` / `/apply` remain manual fallback commands.

Phase 23.5 should preserve the existing safety model while reducing normal Agent UX exposure to internal patch mechanics:

* no automatic file changes
* explicit execution approval before create/update/delete/write
* scope validation before writing
* target and patch validation before writing
* `/diff`, `/check`, and `/apply` remain fallback / direct / debug commands
* plan/action selection is not execution approval and should not use `y / n`
* no multi-file Agent edits yet
* no auto-apply
* no config auto-writing
* no provider auto-routing
* no MCP

Still avoid by default:

* repo-wide agent behavior
* unbounded workspace ingest
* MCP as a near-term requirement
* auto-apply
* config auto-writing
* advanced resume / state restoration

This section is intentionally not a live task tracker.
Use `README.md`, `CHANGELOG.md`, and recent git history to determine the current checkout's exact state.
