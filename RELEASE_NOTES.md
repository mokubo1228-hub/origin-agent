# Release Notes: v0.6.0

Status: Unreleased

## Summary

v0.6.0 is the TUI prototype release for `origin-agent`.

It adds a `ratatui` / `crossterm` based terminal UI that can show runtime status, display chat messages, accept input, and send chat prompts through the existing chat workflow.

The TUI is intentionally still a prototype. It does not yet include file editing, provider/model switching, sessions browsing, diff viewing, streaming, or a multiline editor.

## Added

- `origin tui` command.
- TUI prototype using `ratatui` and `crossterm`.
- Status pane.
- Chat pane.
- Input area.
- Footer.
- TUI chat input.
- TUI chat transcript logging.

## Changed

- TUI can send chat prompts through the existing chat workflow.
- TUI shows user, assistant, system, and error messages.
- TUI exits safely with `q`, `Esc`, or `Ctrl+C`.
- Runtime state and chat workflow are now reusable by TUI.

## Safety Model

- TUI does not run propose/check/apply yet.
- TUI does not modify files.
- TUI does not display or save API keys.
- TUI uses existing transcript safety behavior.
- Terminal raw mode and alternate screen cleanup are handled.

## Known Limitations

- TUI file edit loop is not implemented.
- TUI model/provider switching is not implemented.
- TUI sessions viewer is not implemented.
- TUI diff viewer is not implemented.
- Streaming responses are not implemented.
- TUI multiline editor is not implemented.
- MCP integration is not implemented.
- Repo-wide context selection is not implemented.
- File picker is not implemented.

## Checks Before Tagging

```sh
git status --short
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo run -p origin-cli -- --help
cargo install --path crates/origin-cli
origin --help
origin status
origin tui
git diff --check
```

Suggested tag command after committing release prep:

```sh
git tag -a v0.6.0 -m "v0.6.0 TUI prototype"
```
