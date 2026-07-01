use origin_config::load_user_config;
use origin_protocol::model::ModelInfo;
use origin_protocol::provider::ProviderPreset;
use origin_protocol::session::SessionEvent;
use origin_provider::{builtin_registry, ModelProvider, ProviderRegistry};
use origin_session::{list_sessions, redact_sensitive_text, SessionSummary, SessionWriter};
use origin_tools::{
    changed_files_from_diff, discover_git_repo, git_apply, git_apply_check, git_dirty_status,
    parse_approval_decision, ApprovalDecision, GitRepo,
};
use std::io::{self, Write};

use crate::chat_workflow;
use crate::edit_workflow::{pending_proposal_snapshot, PendingCheckState, PendingProposal};
use crate::edit_workflow::{
    run_propose_preview, ProposePreviewMode, ProposePreviewRequest, ProposePreviewResult,
};
use crate::runtime::{auth_label, first_error_line, ProviderSwitchResult, RuntimeState};
use crate::status_snapshot::{PendingProposalSnapshot, StatusSnapshot, PENDING_STATUS_PENDING};

const PENDING_CREATED_MESSAGE: &str =
    "pending proposal created. Use /diff to review, /check to validate, /apply to apply, or /clear to discard.";
const PENDING_READY_MESSAGE: &str =
    "pending proposal is ready to apply. Run /apply to approve and apply.";
const PENDING_KEPT_MESSAGE: &str = "pending proposal kept.";
const PENDING_CLEARED_MESSAGE: &str = "pending proposal cleared.";
const NO_PENDING_PROPOSAL_MESSAGE: &str =
    "No pending proposal.\nRun `/propose <file> <instruction...>` first.";

const SHELL_HELP: &str = "\
Shell commands:
  /help
      Show this help.

State:
  /status
      Show provider, model, config, sessions, cwd, git, and pending proposal state.
  /sessions
      List recent session transcripts.

Model / Provider:
  /model
      Show the current shell model.
  /model list
      List models for the current provider.
  /model set <model-id>
      Set the model for this shell session only. Does not rewrite config.
  /provider
      Show the current shell provider.
  /provider list
      List configured provider presets.
  /provider set <provider-id>
      Set the provider for this shell session only. Does not rewrite config.

Editing:
  /propose <file> <instruction...>
      Preview a single-file diff proposal.
  /propose <file1> <file2> ... -- <instruction...>
      Preview a multi-file diff proposal. Multiple files require --.
  /diff
      Show the latest pending proposal diff again.
  /check
      Run git apply --check for the latest pending proposal.
  /apply
      Apply the latest pending proposal after git check and y/yes approval.
  /clear
      Clear the latest pending proposal without applying.

Chat:
  any non-slash input
      Send a chat prompt.

Exit:
  /exit
      Exit the shell.
  /quit
      Exit the shell.

Patch apply always requires y/yes approval.
";

#[derive(Debug, Eq, PartialEq)]
enum ShellInput {
    Ignore,
    Help,
    Exit,
    Status,
    Model,
    ModelSet(String),
    ModelList,
    Provider,
    ProviderSet(String),
    ProviderList,
    Propose(ShellProposeArgs),
    ProposeUsage(Option<String>),
    Diff,
    Check,
    Apply,
    Clear,
    Sessions,
    Chat(String),
    Unknown(String),
}

struct ShellState {
    runtime: RuntimeState,
    pending_proposal: Option<PendingProposal>,
}

#[derive(Debug, Eq, PartialEq)]
struct ShellProposeArgs {
    paths: Vec<String>,
    instruction: String,
}

impl ShellState {
    fn initialize() -> Result<Self, String> {
        Ok(Self {
            runtime: RuntimeState::initialize()?,
            pending_proposal: None,
        })
    }
}

impl std::ops::Deref for ShellState {
    type Target = RuntimeState;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

impl std::ops::DerefMut for ShellState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
}

pub fn run() -> Result<(), String> {
    let mut state = ShellState::initialize()?;
    print!("{}", format_shell_startup_intro(&state));

    let stdin = io::stdin();
    loop {
        print!("origin> ");
        io::stdout()
            .flush()
            .map_err(|error| format!("Could not flush shell prompt: {error}"))?;

        let mut line = String::new();
        let bytes = stdin
            .read_line(&mut line)
            .map_err(|error| format!("Could not read shell input: {error}"))?;
        if bytes == 0 {
            println!();
            break;
        }

        match parse_shell_input(&line) {
            ShellInput::Ignore => {}
            ShellInput::Help => println!("{SHELL_HELP}"),
            ShellInput::Exit => break,
            ShellInput::Status => {
                print!(
                    "{}",
                    format_shell_status(&state, state.pending_proposal.as_ref())
                )
            }
            ShellInput::Model => print!("{}", format_shell_model(&state)),
            ShellInput::ModelSet(model_id) => match state.set_model(&model_id) {
                Ok(()) => {
                    print!("{}", format_model_set_success(&state));
                    clear_pending_proposal(&mut state.pending_proposal);
                }
                Err(error) => eprintln!("{error}"),
            },
            ShellInput::Provider => print!("{}", format_shell_provider(&state)),
            ShellInput::ProviderSet(provider_id) => match state.set_provider(&provider_id) {
                Ok(result) => {
                    print!("{}", format_provider_set_success(&state, &result));
                    clear_pending_proposal(&mut state.pending_proposal);
                }
                Err(error) => {
                    eprintln!("{}", format_shell_provider_set_error(&provider_id, &error))
                }
            },
            ShellInput::ModelList => print!("{}", shell_model_list(&state)?),
            ShellInput::ProviderList => print!("{}", shell_provider_list(&state)?),
            ShellInput::Propose(args) => match run_shell_propose(&state.runtime, &args) {
                Ok(proposal) => state.pending_proposal = proposal,
                Err(error) => {
                    state.pending_proposal = None;
                    eprintln!("{}", format_shell_propose_error(&error));
                }
            },
            ShellInput::ProposeUsage(message) => {
                eprintln!("{}", format_shell_propose_usage(message.as_deref()));
            }
            ShellInput::Diff => {
                if let Err(error) = run_shell_diff(state.pending_proposal.as_ref()) {
                    eprintln!("{error}");
                }
            }
            ShellInput::Check => {
                if let Err(error) = run_shell_check(state.pending_proposal.as_mut()) {
                    eprintln!("{error}");
                }
            }
            ShellInput::Apply => match run_shell_apply(state.pending_proposal.as_mut()) {
                Ok(ShellApplyOutcome::Applied) => {
                    clear_pending_proposal(&mut state.pending_proposal);
                }
                Ok(ShellApplyOutcome::Rejected) => {}
                Err(error) => eprintln!("{error}"),
            },
            ShellInput::Clear => clear_pending_proposal_explicit(&mut state.pending_proposal),
            ShellInput::Sessions => print!("{}", shell_sessions()?),
            ShellInput::Unknown(command) => {
                eprintln!("{}", format_unknown_shell_command(&command));
            }
            ShellInput::Chat(prompt) => {
                if let Err(error) = run_shell_chat(&state.runtime, &prompt) {
                    eprintln!("{error}");
                }
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellApplyOutcome {
    Applied,
    Rejected,
}

fn run_shell_chat(state: &RuntimeState, prompt: &str) -> Result<(), String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Ok(());
    }
    let model = state.model_for_chat()?;
    print_field("provider", &state.provider_id);
    print_field("base_url", &state.base_url);
    print_field("model", model);
    print_field("model_source", &state.model_source);
    println!();

    let result = chat_workflow::run_chat(state, prompt)?;
    let _metadata = (
        &result.provider,
        &result.base_url,
        &result.model,
        &result.model_source,
    );
    println!("{}", result.response_text);
    println!();
    println!("session: {}", result.session_path.display());
    Ok(())
}

fn run_shell_propose(
    state: &RuntimeState,
    args: &ShellProposeArgs,
) -> Result<Option<PendingProposal>, String> {
    match run_propose_preview(
        ProposePreviewRequest {
            runtime: state,
            paths: &args.paths,
            context_files: &[],
            instruction: &args.instruction,
            mode: ProposePreviewMode::ExistingFiles,
            retry_invalid: false,
        },
        |prepared| {
            print!(
                "{}",
                format_shell_propose_header(prepared.runtime, prepared.model, prepared.file_count)
            );
        },
    ) {
        Ok(result) => {
            print_shell_propose_result(&result);
            Ok(result.pending)
        }
        Err(error) => {
            if let Some(result) = &error.partial {
                print_shell_propose_result(result);
            } else if !error.session_path.is_empty() {
                print_shell_propose_session(&error.session_path);
            }
            Err(error.error)
        }
    }
}

fn print_shell_propose_result(result: &ProposePreviewResult) {
    if result.pending.is_none() {
        println!("No changes proposed.");
        println!("No files were changed.");
    } else {
        println!("Proposed patch (not applied):");
        println!("{}", result.diff);
        println!("No files were changed.");
        println!("{PENDING_CREATED_MESSAGE}");
    }
    print_shell_propose_session(&result.session_path);
}

fn print_shell_propose_session(session_path: &str) {
    println!();
    println!("session: {session_path}");
}

#[cfg(test)]
fn pending_from_valid_proposal(
    state: &RuntimeState,
    model: &str,
    args: &ShellProposeArgs,
    files: &[origin_tools::ReadFile],
    diff: &str,
) -> PendingProposal {
    PendingProposal::from_valid_diff(state, model, &args.instruction, files, diff)
}

fn run_shell_diff(pending: Option<&PendingProposal>) -> Result<(), String> {
    let Some(pending) = pending else {
        return Err(NO_PENDING_PROPOSAL_MESSAGE.to_string());
    };
    print!("{}", format_pending_diff(pending));
    Ok(())
}

fn format_pending_diff(pending: &PendingProposal) -> String {
    let mut output = String::new();
    output.push_str("Pending proposal:\n");
    push_field(&mut output, "instruction", &pending.instruction);
    push_field(&mut output, "checked", pending.check_state.checked_label());
    if let Some(result) = pending.check_state.check_result() {
        push_field(&mut output, "check_result", result);
    }
    output.push_str("requested:\n");
    push_plain_list(&mut output, &pending.requested_files);
    output.push_str("proposed:\n");
    push_plain_list(&mut output, &pending.proposed_files);
    output.push('\n');
    output.push_str("Diff:\n");
    output.push_str(&pending.diff);
    if !pending.diff.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn run_shell_check(pending: Option<&mut PendingProposal>) -> Result<(), String> {
    let Some(pending) = pending else {
        return Err(NO_PENDING_PROPOSAL_MESSAGE.to_string());
    };
    let mut session = SessionWriter::create("check").map_err(|error| error.to_string())?;
    let result = run_shell_check_with_session(pending, &mut session);
    update_check_state(pending, &result);
    let status = if result.is_ok() { "success" } else { "error" };
    if let Err(error) = &result {
        let _ = session.record(SessionEvent::CommandFailed {
            error: error.clone(),
        });
    }
    let finish_result = session.record(SessionEvent::SessionFinished {
        status: status.to_string(),
    });
    let session_path = session.path().display().to_string();
    println!();
    println!("session: {session_path}");

    match (result, finish_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(error)) => Err(error.to_string()),
        (Err(error), _) => Err(error),
    }
}

fn run_shell_check_with_session(
    pending: &PendingProposal,
    session: &mut SessionWriter,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::UserMessage {
            text: format!("/check {}", pending.instruction),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::ProviderResolved {
            provider: pending.provider.clone(),
            base_url: pending.base_url.clone(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::ModelResolved {
            model: pending.model.clone(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::DiffProposed {
            diff: pending.diff.clone(),
        },
    )?;
    record_session_event(session, SessionEvent::GitApplyCheckStarted)?;

    print_field("mode", "shell check");
    print_field("provider", &pending.provider);
    print_field("base_url", &pending.base_url);
    print_field("model", &pending.model);
    print_field("files", &pending.requested_files.len().to_string());
    println!();

    let repo = discover_git_repo().map_err(|error| error.to_string())?;
    match git_apply_check(&repo, &pending.diff) {
        Ok(()) => {
            record_session_event(session, SessionEvent::GitApplyCheckSucceeded)?;
            println!("git apply --check: ok");
            println!("{PENDING_READY_MESSAGE}");
            println!("No files were changed.");
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            record_session_event(
                session,
                SessionEvent::GitApplyCheckFailed {
                    error: message.clone(),
                },
            )?;
            println!("git apply --check failed.");
            println!("No files were changed.");
            println!("{PENDING_KEPT_MESSAGE}");
            Err(message)
        }
    }
}

fn run_shell_apply(pending: Option<&mut PendingProposal>) -> Result<ShellApplyOutcome, String> {
    let Some(pending) = pending else {
        return Err(NO_PENDING_PROPOSAL_MESSAGE.to_string());
    };
    let mut session = SessionWriter::create("apply").map_err(|error| error.to_string())?;
    let result = run_shell_apply_with_session(pending, &mut session);
    let status = if result.is_ok() { "success" } else { "error" };
    if let Err(error) = &result {
        let _ = session.record(SessionEvent::CommandFailed {
            error: error.clone(),
        });
    }
    let finish_result = session.record(SessionEvent::SessionFinished {
        status: status.to_string(),
    });
    let session_path = session.path().display().to_string();
    println!();
    println!("session: {session_path}");

    match (result, finish_result) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(_), Err(error)) => Err(error.to_string()),
        (Err(error), _) => Err(error),
    }
}

fn run_shell_apply_with_session(
    pending: &mut PendingProposal,
    session: &mut SessionWriter,
) -> Result<ShellApplyOutcome, String> {
    record_pending_proposal_context(session, pending, "/apply")?;

    print_field("mode", "shell apply");
    print_field("provider", &pending.provider);
    print_field("base_url", &pending.base_url);
    print_field("model", &pending.model);
    print_field("files", &pending.requested_files.len().to_string());
    println!();

    let repo = discover_git_repo().map_err(|error| error.to_string())?;
    warn_if_dirty(&repo)?;
    record_session_event(session, SessionEvent::GitApplyCheckStarted)?;
    match git_apply_check(&repo, &pending.diff) {
        Ok(()) => {
            pending.check_state = PendingCheckState::Passed;
            record_session_event(session, SessionEvent::GitApplyCheckSucceeded)?;
            println!("git apply --check: ok");
        }
        Err(error) => {
            let message = error.to_string();
            pending.check_state = PendingCheckState::Failed(first_error_line(&message));
            record_session_event(
                session,
                SessionEvent::GitApplyCheckFailed {
                    error: message.clone(),
                },
            )?;
            println!("git apply --check: failed");
            println!("Nothing was applied.");
            println!("{PENDING_KEPT_MESSAGE}");
            return Err(message);
        }
    }

    record_session_event(session, SessionEvent::ApprovalRequested)?;
    print!("\nApply this patch? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|error| format!("Could not flush approval prompt: {error}"))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| format!("Could not read approval input: {error}"))?;

    match parse_approval_decision(&input) {
        ApprovalDecision::Approved => {
            record_session_event(
                session,
                SessionEvent::ApprovalDecision {
                    decision: "approved".to_string(),
                },
            )?;
            if let Err(error) = git_apply(&repo, &pending.diff) {
                println!("Patch apply failed.");
                println!("{PENDING_KEPT_MESSAGE}");
                return Err(error.to_string());
            }
            println!("Patch applied.");
            let files = changed_files_from_diff(&pending.diff);
            record_session_event(
                session,
                SessionEvent::PatchApplied {
                    files: files.clone(),
                },
            )?;
            if !files.is_empty() {
                println!("Changed files:");
                for file in files {
                    println!("- {file}");
                }
            }
            Ok(ShellApplyOutcome::Applied)
        }
        ApprovalDecision::Rejected => {
            record_session_event(
                session,
                SessionEvent::ApprovalDecision {
                    decision: "rejected".to_string(),
                },
            )?;
            record_session_event(session, SessionEvent::PatchRejected)?;
            println!("Patch rejected. Nothing was applied.");
            println!("{PENDING_KEPT_MESSAGE}");
            Ok(ShellApplyOutcome::Rejected)
        }
    }
}

fn update_check_state(pending: &mut PendingProposal, result: &Result<(), String>) {
    pending.update_check_state(result);
}

fn record_pending_proposal_context(
    session: &mut SessionWriter,
    pending: &PendingProposal,
    command: &str,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::UserMessage {
            text: format!("{command} {}", pending.instruction),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::ProviderResolved {
            provider: pending.provider.clone(),
            base_url: pending.base_url.clone(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::ModelResolved {
            model: pending.model.clone(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::DiffProposed {
            diff: pending.diff.clone(),
        },
    )
}

fn warn_if_dirty(repo: &GitRepo) -> Result<(), String> {
    let status = git_dirty_status(repo).map_err(|error| error.to_string())?;
    if status.is_dirty {
        eprintln!("Warning: working tree has uncommitted changes.");
    }
    Ok(())
}

fn clear_pending_proposal(pending: &mut Option<PendingProposal>) {
    if pending.take().is_some() {
        println!("{PENDING_CLEARED_MESSAGE}");
    }
}

fn clear_pending_proposal_explicit(pending: &mut Option<PendingProposal>) {
    if pending.is_some() {
        clear_pending_proposal(pending);
    } else {
        println!("No pending proposal to clear.");
        println!("Run `/propose <file> <instruction...>` to create one.");
    }
}

fn record_session_event(session: &mut SessionWriter, event: SessionEvent) -> Result<(), String> {
    session.record(event).map_err(|error| error.to_string())
}

fn parse_shell_input(input: &str) -> ShellInput {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return ShellInput::Ignore;
    }

    let Some(command) = trimmed.strip_prefix('/') else {
        return ShellInput::Chat(trimmed.to_string());
    };
    let parts = command.split_whitespace().collect::<Vec<_>>();

    match parts.as_slice() {
        ["help"] => ShellInput::Help,
        ["exit"] | ["quit"] => ShellInput::Exit,
        ["status"] => ShellInput::Status,
        ["model"] => ShellInput::Model,
        ["model", "set", rest @ ..] => ShellInput::ModelSet(rest.join(" ")),
        ["model", "list"] => ShellInput::ModelList,
        ["provider"] => ShellInput::Provider,
        ["provider", "set", rest @ ..] => ShellInput::ProviderSet(rest.join(" ")),
        ["provider", "list"] => ShellInput::ProviderList,
        ["propose", rest @ ..] => parse_shell_propose(rest),
        ["diff"] => ShellInput::Diff,
        ["check"] => ShellInput::Check,
        ["apply"] => ShellInput::Apply,
        ["clear"] => ShellInput::Clear,
        ["sessions"] => ShellInput::Sessions,
        _ => ShellInput::Unknown(trimmed.to_string()),
    }
}

fn parse_shell_propose(parts: &[&str]) -> ShellInput {
    match parse_shell_propose_args(parts) {
        Ok(args) => ShellInput::Propose(args),
        Err(message) => ShellInput::ProposeUsage(message),
    }
}

fn parse_shell_propose_args(parts: &[&str]) -> Result<ShellProposeArgs, Option<String>> {
    if parts.is_empty() {
        return Err(None);
    }

    if let Some(separator_index) = parts.iter().position(|part| *part == "--") {
        let file_parts = &parts[..separator_index];
        let instruction_parts = &parts[separator_index + 1..];
        if file_parts.is_empty() || instruction_parts.is_empty() {
            return Err(None);
        }
        return Ok(ShellProposeArgs {
            paths: file_parts.iter().map(|path| (*path).to_string()).collect(),
            instruction: instruction_parts.join(" "),
        });
    }

    if parts.len() < 2 {
        return Err(None);
    }

    let path = parts[0].to_string();
    let instruction_parts = &parts[1..];
    if instruction_parts
        .first()
        .is_some_and(|token| looks_like_file_path(token))
    {
        return Err(Some(
            "Multiple files require `--`: /propose README.md CHANGELOG.md -- fix typo".to_string(),
        ));
    }

    Ok(ShellProposeArgs {
        paths: vec![path],
        instruction: instruction_parts.join(" "),
    })
}

fn looks_like_file_path(token: &str) -> bool {
    token.contains('/') || token.contains('\\') || token.contains('.')
}

fn propose_usage() -> &'static str {
    "Usage:\n  /propose <file> <instruction...>\n  /propose <file1> <file2> ... -- <instruction...>"
}

fn format_shell_propose_usage(message: Option<&str>) -> String {
    let mut output = String::new();
    if let Some(message) = message {
        output.push_str(message);
        output.push('\n');
    }
    output.push_str(propose_usage());
    output
}

fn format_unknown_shell_command(command: &str) -> String {
    format!("Unknown slash command `{command}`.\nType `/help` for available commands.")
}

fn format_shell_provider_set_error(provider_id: &str, error: &str) -> String {
    if error.contains("Unknown provider") {
        return format!(
            "Unknown provider `{provider_id}`.\nRun `/provider list` to see available providers."
        );
    }

    error.to_string()
}

fn format_shell_propose_error(error: &str) -> String {
    if is_diff_rejection_error(error) {
        let reason = error_reason(error).unwrap_or_else(|| first_error_line(error));
        return format!(
            "Proposed diff was rejected before preview.\nReason: {reason}\nNo files were changed."
        );
    }

    error.to_string()
}

fn is_diff_rejection_error(error: &str) -> bool {
    error.contains("Model proposed changes outside the requested file scope")
        || error.contains("Model returned an invalid unified diff")
}

fn error_reason(error: &str) -> Option<String> {
    error.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Reason:")
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty())
    })
}

fn format_shell_status(state: &ShellState, pending: Option<&PendingProposal>) -> String {
    format_status_snapshot(&StatusSnapshot::from_runtime(
        &state.runtime,
        pending_proposal_snapshot(pending),
    ))
}

fn format_status_snapshot(snapshot: &StatusSnapshot) -> String {
    let mut output = String::new();
    output.push_str("origin-agent shell status\n\n");

    output.push_str("Runtime:\n");
    push_indented_field(&mut output, "mode", &snapshot.mode);
    push_indented_field(&mut output, "command", &snapshot.command);
    push_indented_field(&mut output, "cwd", &snapshot.cwd);
    push_indented_field(&mut output, "git", &snapshot.git_state);

    output.push('\n');
    output.push_str("Provider:\n");
    push_indented_field(&mut output, "id", &snapshot.provider_id);
    push_indented_field(&mut output, "kind", &snapshot.provider_kind);
    push_indented_field(&mut output, "base_url", &snapshot.base_url);
    push_indented_field(&mut output, "auth", &snapshot.auth_label);

    output.push('\n');
    output.push_str("Model:\n");
    push_indented_field(
        &mut output,
        "id",
        snapshot.model_id.as_deref().unwrap_or("unresolved"),
    );
    push_indented_field(&mut output, "source", &snapshot.model_source);
    if let Some(error) = &snapshot.model_error {
        push_indented_field(&mut output, "error", error);
    }

    output.push('\n');
    output.push_str("Config:\n");
    push_indented_field(&mut output, "path", &snapshot.config_path);
    push_indented_field(&mut output, "sessions", &snapshot.sessions_root);

    push_pending_proposal_status(&mut output, &snapshot.pending);
    output
}

fn format_shell_startup_intro(state: &ShellState) -> String {
    let mut output = String::new();
    output.push_str("origin-agent interactive shell\n\n");
    push_field(
        &mut output,
        "Provider",
        &redact_sensitive_text(&state.provider_id),
    );
    push_field(
        &mut output,
        "Model",
        &redact_sensitive_text(state.model_id.as_deref().unwrap_or("unresolved")),
    );
    push_field(&mut output, "Git", &redact_sensitive_text(&state.git_state));
    output.push('\n');
    output.push_str("Type /help for commands.\n");
    output.push_str("Try /status, /model, /provider, /sessions.\n");
    output.push_str("Editing flow: /propose -> /diff -> /check -> /apply\n");
    output.push_str("Type /exit to quit.\n\n");
    output
}

fn format_shell_model(state: &ShellState) -> String {
    let mut output = String::new();
    output.push_str("Current model:\n\n");
    output.push_str("Provider:\n");
    push_indented_field(&mut output, "id", &state.provider_id);

    output.push('\n');
    output.push_str("Model:\n");
    push_indented_field(
        &mut output,
        "id",
        state.model_id.as_deref().unwrap_or("unresolved"),
    );
    push_indented_field(&mut output, "source", &state.model_source);
    push_indented_field(&mut output, "fallback", shell_model_fallback_label(state));
    if let Some(error) = &state.model_error {
        push_indented_field(&mut output, "error", error);
    }

    output.push('\n');
    output.push_str("Hints:\n");
    output.push_str("  Use /model list to see available models.\n");
    output.push_str("  Use /model set <model-id> to switch for this shell session.\n");
    output
}

fn format_shell_provider(state: &ShellState) -> String {
    let mut output = String::new();
    output.push_str("Current provider:\n\n");
    output.push_str("Provider:\n");
    push_indented_field(&mut output, "id", &state.provider_id);
    push_indented_field(&mut output, "kind", &state.provider_kind);
    push_indented_field(&mut output, "base_url", &state.base_url);
    push_indented_field(&mut output, "auth", &state.auth_label);

    output.push('\n');
    output.push_str("Hints:\n");
    output.push_str("  Use /provider list to see available providers.\n");
    output.push_str("  Use /provider set <provider-id> to switch for this shell session.\n");
    output
}

fn format_model_set_success(state: &ShellState) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "model set to {}\n",
        state.model_id.as_deref().unwrap_or("unresolved")
    ));
    push_field(&mut output, "provider", &state.provider_id);
    push_field(&mut output, "model_source", &state.model_source);
    output
}

fn format_provider_set_success(state: &ShellState, result: &ProviderSwitchResult) -> String {
    let mut output = String::new();
    output.push_str(&format!("provider set to {}\n", result.provider_id));
    push_field(&mut output, "base_url", &state.base_url);
    push_field(&mut output, "auth", &state.auth_label);
    match result.model_id.as_deref() {
        Some(model_id) => output.push_str(&format!("model re-resolved to {model_id}\n")),
        None => output.push_str("model unresolved\n"),
    }
    push_field(&mut output, "model_source", &result.model_source);
    if let Some(error) = &state.model_error {
        push_field(&mut output, "model_error", error);
    }
    output
}

fn format_shell_propose_header(state: &RuntimeState, model: &str, file_count: usize) -> String {
    let mut output = String::new();
    push_field(&mut output, "mode", "shell propose preview");
    push_field(&mut output, "provider", &state.provider_id);
    push_field(&mut output, "base_url", &state.base_url);
    push_field(&mut output, "model", model);
    push_field(&mut output, "model_source", &state.model_source);
    push_field(&mut output, "files", &file_count.to_string());
    output.push('\n');
    output
}

fn shell_model_list(state: &ShellState) -> Result<String, String> {
    let models = state
        .provider
        .list_models()
        .map_err(|error| error.user_message())?;

    Ok(format_shell_model_list(state, &models))
}

fn format_shell_model_list(state: &ShellState, models: &[ModelInfo]) -> String {
    let mut output = String::new();
    output.push_str(&format!("Models for provider: {}\n", state.provider_id));
    output.push_str(&format!("base_url: {}\n", state.base_url));
    output.push_str(&format!(
        "current: {}\n",
        state.model_id.as_deref().unwrap_or("unresolved")
    ));
    output.push('\n');
    output.push_str(&format!(
        "{:<72}  {:<8}  fallback_candidate\n",
        "model_id", "current"
    ));
    for model in models {
        let current = if Some(model.id.as_str()) == state.model_id.as_deref() {
            "yes"
        } else {
            "no"
        };
        output.push_str(&format!(
            "{:<72}  {:<8}  {}\n",
            model.id,
            current,
            fallback_candidate_label(model)
        ));
    }
    output
}

fn shell_provider_list(state: &ShellState) -> Result<String, String> {
    let registry = builtin_registry();
    let config = load_user_config().map_err(|error| error.to_string())?;

    Ok(format_shell_provider_list(
        state,
        &registry,
        config.as_ref().map(|loaded| &loaded.config),
    ))
}

fn shell_sessions() -> Result<String, String> {
    let sessions = list_sessions().map_err(|error| error.to_string())?;
    Ok(format_shell_sessions(&sessions))
}

fn format_shell_sessions(sessions: &[SessionSummary]) -> String {
    let mut output = String::new();
    output.push_str("Recent sessions:\n\n");

    if sessions.is_empty() {
        output.push_str("No sessions found.\n");
        return output;
    }

    output.push_str(&format!(
        "{:<7}  {:<25}  {:<8}  {:<10}  {:<30}  status\n",
        "latest", "session_id", "command", "provider", "model"
    ));
    for (index, summary) in sessions.iter().take(10).enumerate() {
        output.push_str(&format_shell_session_row(summary, index == 0));
    }
    if sessions.len() > 10 {
        output.push_str(&format!("\nShowing 10 of {} sessions.\n", sessions.len()));
    }

    output.push_str("\nTip:\n");
    output.push_str("  Use plain CLI `origin sessions show <session-id>` to inspect details.\n");
    output.push_str("  Use `origin sessions show --latest` to inspect the latest session.\n");
    output
}

fn format_shell_session_row(summary: &SessionSummary, latest: bool) -> String {
    format!(
        "{:<7}  {:<25}  {:<8}  {:<10}  {:<30}  {}\n",
        if latest { "yes" } else { "" },
        redact_sensitive_text(&summary.session_id),
        redact_sensitive_text(&summary.command),
        redact_sensitive_text(&summary.provider),
        shell_truncate_middle(&redact_sensitive_text(&summary.model), 30),
        redact_sensitive_text(&summary.status)
    )
}

fn format_shell_provider_list(
    state: &ShellState,
    registry: &ProviderRegistry,
    config: Option<&origin_config::UserConfig>,
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "{:<12}  {:<8}  {:<20}  {:<8}  base_url\n",
        "provider", "current", "kind", "auth"
    ));
    for preset in registry.presets() {
        let provider_config = config.and_then(|config| config.provider(preset.id.as_str()));
        let base_url = provider_config
            .and_then(|provider| provider.base_url.as_deref())
            .unwrap_or(&preset.base_url);
        output.push_str(&format!(
            "{:<12}  {:<8}  {:<20}  {:<8}  {}\n",
            preset.id.as_str(),
            current_marker(state, preset),
            preset.kind.as_str(),
            auth_label(
                &preset.auth,
                provider_config.and_then(|provider| provider.api_key_env.as_deref())
            ),
            base_url
        ));
    }
    output
}

fn current_marker(state: &ShellState, preset: &ProviderPreset) -> &'static str {
    if state.provider_id == preset.id.as_str() {
        "yes"
    } else {
        ""
    }
}

fn fallback_candidate_label(model: &ModelInfo) -> &'static str {
    if model.is_fallback_candidate() {
        "yes"
    } else {
        "no"
    }
}

fn shell_model_fallback_label(state: &ShellState) -> &'static str {
    if state.model_source == "provider_fallback" {
        "yes"
    } else {
        "no"
    }
}

fn shell_truncate_middle(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let keep = width - 3;
    let prefix_len = keep / 2;
    let suffix_len = keep - prefix_len;
    let prefix = value.chars().take(prefix_len).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}...{suffix}")
}

fn push_field(output: &mut String, label: &str, value: &str) {
    output.push_str(&format!("{:<13} {}\n", format!("{label}:"), value));
}

fn push_pending_proposal_status(output: &mut String, pending: &PendingProposalSnapshot) {
    output.push('\n');
    output.push_str("Pending proposal:\n");
    if pending.status != PENDING_STATUS_PENDING {
        push_indented_field(output, "status", &pending.status);
        return;
    }

    push_indented_field(output, "status", &pending.status);
    if let Some(checked) = &pending.checked {
        push_indented_field(output, "checked", checked);
    }
    if let Some(result) = &pending.check_result {
        push_indented_field(output, "check_result", result);
    }
    if let Some(instruction) = &pending.instruction {
        push_indented_field(output, "instruction", instruction);
    }
    output.push_str("  requested:\n");
    push_indented_list(output, &pending.requested_files);
    output.push_str("  proposed:\n");
    push_indented_list(output, &pending.proposed_files);
    if let Some(provider) = &pending.provider {
        push_indented_field(output, "provider", provider);
    }
    if let Some(model) = &pending.model {
        push_indented_field(output, "model", model);
    }
}

fn push_indented_field(output: &mut String, label: &str, value: &str) {
    output.push_str(&format!("  {:<13} {}\n", format!("{label}:"), value));
}

fn push_indented_list(output: &mut String, values: &[String]) {
    if values.is_empty() {
        output.push_str("    - none\n");
        return;
    }
    for value in values {
        output.push_str(&format!("    - {value}\n"));
    }
}

fn push_plain_list(output: &mut String, values: &[String]) {
    if values.is_empty() {
        output.push_str("- none\n");
        return;
    }
    for value in values {
        output.push_str(&format!("- {value}\n"));
    }
}

fn print_field(label: &str, value: &str) {
    println!("{:<13} {}", format!("{label}:"), value);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use origin_config::{ProviderConfig, UserConfig};
    use origin_protocol::model::ModelInfo;
    use origin_protocol::provider::{ProviderId, ProviderKind};
    use origin_provider::builtin_registry;
    use origin_session::SessionSummary;

    use super::{
        clear_pending_proposal, clear_pending_proposal_explicit, format_model_set_success,
        format_pending_diff, format_provider_set_success, format_shell_model,
        format_shell_model_list, format_shell_propose_error, format_shell_propose_header,
        format_shell_propose_usage, format_shell_provider, format_shell_provider_list,
        format_shell_provider_set_error, format_shell_sessions, format_shell_startup_intro,
        format_shell_status, format_status_snapshot, parse_shell_input,
        pending_from_valid_proposal, pending_proposal_snapshot, run_shell_apply, run_shell_check,
        run_shell_diff, PendingCheckState, PendingProposal, ShellApplyOutcome, ShellInput,
        ShellProposeArgs, ShellState, NO_PENDING_PROPOSAL_MESSAGE, PENDING_CLEARED_MESSAGE,
        PENDING_CREATED_MESSAGE, PENDING_KEPT_MESSAGE, PENDING_READY_MESSAGE, SHELL_HELP,
    };
    use crate::runtime::{
        ensure_auth_available, resolve_runtime_model, RuntimeModel, RuntimeState,
    };
    use crate::status_snapshot::{
        PendingProposalSnapshot, StatusSnapshot, PENDING_STATUS_NONE, PENDING_STATUS_PENDING,
    };
    use origin_protocol::provider::ProviderAuth;
    use origin_provider::openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleProvider};
    use origin_tools::ReadFile;

    #[test]
    fn parses_empty_input_as_ignore() {
        assert_eq!(parse_shell_input(""), ShellInput::Ignore);
        assert_eq!(parse_shell_input("  \n"), ShellInput::Ignore);
    }

    #[test]
    fn parses_help_command() {
        assert_eq!(parse_shell_input("/help"), ShellInput::Help);
    }

    #[test]
    fn shell_help_mentions_edit_loop_commands() {
        assert!(SHELL_HELP.contains("State:"));
        assert!(SHELL_HELP.contains("Model / Provider:"));
        assert!(SHELL_HELP.contains("Editing:"));
        assert!(SHELL_HELP.contains("Chat:"));
        assert!(SHELL_HELP.contains("Exit:"));
        assert!(SHELL_HELP.contains("/propose <file> <instruction...>"));
        assert!(SHELL_HELP.contains("/propose <file1> <file2> ... -- <instruction...>"));
        assert!(SHELL_HELP.contains("/diff"));
        assert!(SHELL_HELP.contains("/check"));
        assert!(SHELL_HELP.contains("/apply"));
        assert!(SHELL_HELP.contains("/clear"));
        assert!(SHELL_HELP.contains("any non-slash input"));
        assert!(SHELL_HELP.contains("Does not rewrite config."));
        assert!(SHELL_HELP.contains("y/yes approval"));
        assert!(!SHELL_HELP.contains("Authorization"));
        assert!(!SHELL_HELP.contains("Bearer"));
    }

    #[test]
    fn parses_exit_commands() {
        assert_eq!(parse_shell_input("/exit"), ShellInput::Exit);
        assert_eq!(parse_shell_input("/quit"), ShellInput::Exit);
    }

    #[test]
    fn parses_status_model_provider_and_sessions_commands() {
        assert_eq!(parse_shell_input("/status"), ShellInput::Status);
        assert_eq!(parse_shell_input("/model"), ShellInput::Model);
        assert_eq!(
            parse_shell_input("/model set google/gemma-4-31b"),
            ShellInput::ModelSet("google/gemma-4-31b".to_string())
        );
        assert_eq!(
            parse_shell_input("/model set"),
            ShellInput::ModelSet(String::new())
        );
        assert_eq!(parse_shell_input("/model list"), ShellInput::ModelList);
        assert_eq!(parse_shell_input("/provider"), ShellInput::Provider);
        assert_eq!(
            parse_shell_input("/provider set openai"),
            ShellInput::ProviderSet("openai".to_string())
        );
        assert_eq!(
            parse_shell_input("/provider set"),
            ShellInput::ProviderSet(String::new())
        );
        assert_eq!(
            parse_shell_input("/provider list"),
            ShellInput::ProviderList
        );
        assert_eq!(
            parse_shell_input("/propose README.md fix typo"),
            ShellInput::Propose(ShellProposeArgs {
                paths: vec!["README.md".to_string()],
                instruction: "fix typo".to_string()
            })
        );
        assert_eq!(
            parse_shell_input("/propose"),
            ShellInput::ProposeUsage(None)
        );
        assert_eq!(
            parse_shell_input("/propose README.md"),
            ShellInput::ProposeUsage(None)
        );
        assert_eq!(
            parse_shell_input("/propose README.md -- fix typo"),
            ShellInput::Propose(ShellProposeArgs {
                paths: vec!["README.md".to_string()],
                instruction: "fix typo".to_string()
            })
        );
        assert_eq!(
            parse_shell_input("/propose README.md CHANGELOG.md -- align wording"),
            ShellInput::Propose(ShellProposeArgs {
                paths: vec!["README.md".to_string(), "CHANGELOG.md".to_string()],
                instruction: "align wording".to_string()
            })
        );
        assert_eq!(
            parse_shell_input("/propose -- fix typo"),
            ShellInput::ProposeUsage(None)
        );
        assert_eq!(
            parse_shell_input("/propose README.md --"),
            ShellInput::ProposeUsage(None)
        );
        assert_eq!(
            parse_shell_input("/propose README.md CHANGELOG.md fix typo"),
            ShellInput::ProposeUsage(Some(
                "Multiple files require `--`: /propose README.md CHANGELOG.md -- fix typo"
                    .to_string()
            ))
        );
        assert_eq!(parse_shell_input("/diff"), ShellInput::Diff);
        assert_eq!(parse_shell_input("/check"), ShellInput::Check);
        assert_eq!(parse_shell_input("/apply"), ShellInput::Apply);
        assert_eq!(parse_shell_input("/clear"), ShellInput::Clear);
        assert_eq!(parse_shell_input("/sessions"), ShellInput::Sessions);
    }

    #[test]
    fn parses_unknown_slash_command() {
        assert_eq!(
            parse_shell_input("/unknown"),
            ShellInput::Unknown("/unknown".to_string())
        );
    }

    #[test]
    fn formats_unknown_slash_command_with_help_hint() {
        let output = super::format_unknown_shell_command("/unknown");

        assert_eq!(
            output,
            "Unknown slash command `/unknown`.\nType `/help` for available commands."
        );
    }

    #[test]
    fn formats_propose_usage_as_multiline_help() {
        let output = format_shell_propose_usage(Some(
            "Multiple files require `--`: /propose README.md CHANGELOG.md -- fix typo",
        ));

        assert!(output.contains("Multiple files require `--`"));
        assert!(output.contains("Usage:"));
        assert!(output.contains("/propose <file> <instruction...>"));
        assert!(output.contains("/propose <file1> <file2> ... -- <instruction...>"));
    }

    #[test]
    fn formats_shell_propose_validation_failure_with_next_state() {
        let output = format_shell_propose_error(
            "Model proposed changes outside the requested file scope.\nReason: model proposed changes outside the requested file scope\nNothing was applied.",
        );

        assert!(output.contains("Proposed diff was rejected before preview."));
        assert!(output.contains("Reason: model proposed changes outside the requested file scope"));
        assert!(output.contains("No files were changed."));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn formats_unknown_provider_with_list_hint() {
        let output = format_shell_provider_set_error(
            "unknown-provider",
            "Unknown provider `unknown-provider`.",
        );

        assert_eq!(
            output,
            "Unknown provider `unknown-provider`.\nRun `/provider list` to see available providers."
        );
    }

    #[test]
    fn parses_normal_text_as_chat() {
        assert_eq!(
            parse_shell_input("hello there"),
            ShellInput::Chat("hello there".to_string())
        );
    }

    #[test]
    fn formats_startup_intro_with_current_state_without_secret_terms() {
        let state = test_state();
        let output = format_shell_startup_intro(&state);

        assert!(output.contains("origin-agent interactive shell"));
        assert!(output.contains("Provider:"));
        assert!(output.contains("lmstudio"));
        assert!(output.contains("Model:"));
        assert!(output.contains("shell-model"));
        assert!(output.contains("Git:"));
        assert!(output.contains("clean"));
        assert!(output.contains("Type /help for commands."));
        assert!(output.contains("Try /status, /model, /provider, /sessions."));
        assert!(output.contains("Editing flow: /propose -> /diff -> /check -> /apply"));
        assert!(output.contains("Type /exit to quit."));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
        assert!(!output.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn formats_startup_intro_with_unresolved_model() {
        let mut state = test_state();
        state.model_id = None;
        state.model_source = "unresolved".to_string();
        state.model_error = Some("provider unavailable".to_string());

        let output = format_shell_startup_intro(&state);

        assert!(output.contains("Model:"));
        assert!(output.contains("unresolved"));
        assert!(output.contains("Provider:"));
        assert!(output.contains("lmstudio"));
    }

    #[test]
    fn formats_status_from_shell_state_without_secret_terms() {
        let state = test_state();
        let output = format_shell_status(&state, None);

        assert!(output.contains("origin-agent shell status"));
        assert!(output.contains("Runtime:"));
        assert!(output.contains("Provider:"));
        assert!(output.contains("Model:"));
        assert!(output.contains("Config:"));
        assert!(output.contains("Pending proposal:"));
        assert!(output.contains("mode:"));
        assert!(output.contains("interactive"));
        assert!(output.contains("command:"));
        assert!(output.contains("origin"));
        assert!(output.contains("id:           lmstudio"));
        assert!(output.contains("kind:         openai-compatible"));
        assert!(output.contains("auth:         none"));
        assert!(output.contains("source:       config"));
        assert!(output.contains("path:         /home/user/.origin/config.toml"));
        assert!(output.contains("status:       none"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
        assert!(!output.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn builds_status_snapshot_for_shell_status_without_pending() {
        let state = test_state();
        let snapshot =
            StatusSnapshot::from_runtime(&state.runtime, pending_proposal_snapshot(None));

        assert_eq!(snapshot.mode, "interactive");
        assert_eq!(snapshot.command, "origin");
        assert_eq!(snapshot.provider_id, "lmstudio");
        assert_eq!(snapshot.provider_kind, "openai-compatible");
        assert_eq!(snapshot.model_id.as_deref(), Some("shell-model"));
        assert_eq!(snapshot.model_source, "config");
        assert_eq!(snapshot.pending.status, PENDING_STATUS_NONE);
        assert_eq!(snapshot.pending.checked, None);
    }

    #[test]
    fn formats_status_snapshot_like_shell_status() {
        let state = test_state();
        let snapshot =
            StatusSnapshot::from_runtime(&state.runtime, PendingProposalSnapshot::none());
        let output = format_status_snapshot(&snapshot);

        assert!(output.contains("origin-agent shell status"));
        assert!(output.contains("Runtime:"));
        assert!(output.contains("Provider:"));
        assert!(output.contains("Model:"));
        assert!(output.contains("Config:"));
        assert!(output.contains("Pending proposal:"));
        assert!(output.contains("status:       none"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn formats_status_with_pending_proposal_not_checked() {
        let state = test_state();
        let pending = test_pending_proposal();
        let output = format_shell_status(&state, Some(&pending));
        let snapshot = pending_proposal_snapshot(Some(&pending));

        assert!(output.contains("Pending proposal:"));
        assert!(output.contains("status:       pending"));
        assert!(output.contains("checked:      no"));
        assert!(output.contains("instruction:  fix typo"));
        assert!(output.contains("requested:"));
        assert!(output.contains("- README.md"));
        assert!(output.contains("proposed:"));
        assert!(output.contains("provider:     lmstudio"));
        assert!(output.contains("model:        shell-model"));
        assert!(!output.contains("check_result:"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
        assert_eq!(snapshot.status, PENDING_STATUS_PENDING);
        assert_eq!(snapshot.checked.as_deref(), Some("no"));
        assert_eq!(snapshot.instruction.as_deref(), Some("fix typo"));
        assert_eq!(snapshot.requested_files, vec!["README.md"]);
        assert_eq!(snapshot.proposed_files, vec!["README.md"]);
    }

    #[test]
    fn formats_status_with_successful_pending_check() {
        let state = test_state();
        let mut pending = test_pending_proposal();
        pending.check_state = PendingCheckState::Passed;
        let output = format_shell_status(&state, Some(&pending));
        let snapshot = pending_proposal_snapshot(Some(&pending));

        assert!(output.contains("checked:      yes"));
        assert!(output.contains("check_result: ok"));
        assert_eq!(snapshot.checked.as_deref(), Some("yes"));
        assert_eq!(snapshot.check_result.as_deref(), Some("ok"));
    }

    #[test]
    fn formats_status_with_failed_pending_check() {
        let state = test_state();
        let mut pending = test_pending_proposal();
        pending.check_state = PendingCheckState::Failed("git apply --check failed".to_string());
        let output = format_shell_status(&state, Some(&pending));
        let snapshot = pending_proposal_snapshot(Some(&pending));

        assert!(output.contains("checked:      failed"));
        assert!(output.contains("check_result: git apply --check failed"));
        assert_eq!(snapshot.checked.as_deref(), Some("failed"));
        assert_eq!(
            snapshot.check_result.as_deref(),
            Some("git apply --check failed")
        );
    }

    #[test]
    fn formats_model_from_shell_state() {
        let state = test_state();
        let output = format_shell_model(&state);

        assert!(output.contains("Current model:"));
        assert!(output.contains("Provider:"));
        assert!(output.contains("Model:"));
        assert!(output.contains("Hints:"));
        assert!(output.contains("id:           lmstudio"));
        assert!(output.contains("id:           shell-model"));
        assert!(output.contains("source:       config"));
        assert!(output.contains("fallback:     no"));
        assert!(output.contains("Use /model list"));
        assert!(output.contains("Use /model set <model-id>"));
        assert!(output.contains("shell-model"));
        assert!(output.contains("config"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn formats_provider_from_shell_state() {
        let state = test_state();
        let output = format_shell_provider(&state);

        assert!(output.contains("Current provider:"));
        assert!(output.contains("Provider:"));
        assert!(output.contains("Hints:"));
        assert!(output.contains("id:           lmstudio"));
        assert!(output.contains("kind:         openai-compatible"));
        assert!(output.contains("base_url:     http://localhost:1234/v1"));
        assert!(output.contains("auth:         none"));
        assert!(output.contains("Use /provider list"));
        assert!(output.contains("Use /provider set <provider-id>"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn formats_shell_propose_header_as_preview_without_apply_terms() {
        let state = test_state();
        let output = format_shell_propose_header(&state, "shell-model", 1);

        assert!(output.contains("mode:"));
        assert!(output.contains("shell propose preview"));
        assert!(output.contains("provider:"));
        assert!(output.contains("lmstudio"));
        assert!(output.contains("model:"));
        assert!(output.contains("shell-model"));
        assert!(output.contains("model_source:"));
        assert!(output.contains("config"));
        assert!(output.contains("files:"));
        assert!(!output.contains("git apply"));
        assert!(!output.contains("approval"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn builds_pending_proposal_from_valid_diff() {
        let state = test_state();
        let args = ShellProposeArgs {
            paths: vec!["README.md".to_string()],
            instruction: "fix typo".to_string(),
        };
        let files = vec![ReadFile {
            relative_path: "README.md".to_string(),
            contents: "old\n".to_string(),
        }];
        let diff = "\
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new
";

        let pending = pending_from_valid_proposal(&state, "shell-model", &args, &files, diff);

        assert_eq!(pending.diff, diff);
        assert_eq!(pending.requested_files, vec!["README.md"]);
        assert_eq!(pending.proposed_files, vec!["README.md"]);
        assert_eq!(pending.instruction, "fix typo");
        assert_eq!(pending.provider, "lmstudio");
        assert_eq!(pending.base_url, "http://localhost:1234/v1");
        assert_eq!(pending.model, "shell-model");
    }

    #[test]
    fn builds_pending_proposal_from_multiple_files() {
        let state = test_state();
        let args = ShellProposeArgs {
            paths: vec!["README.md".to_string(), "CHANGELOG.md".to_string()],
            instruction: "align wording".to_string(),
        };
        let files = vec![
            ReadFile {
                relative_path: "README.md".to_string(),
                contents: "old readme\n".to_string(),
            },
            ReadFile {
                relative_path: "CHANGELOG.md".to_string(),
                contents: "old changelog\n".to_string(),
            },
        ];
        let diff = "\
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old readme
+new readme
--- a/CHANGELOG.md
+++ b/CHANGELOG.md
@@ -1 +1 @@
-old changelog
+new changelog
";

        let pending = pending_from_valid_proposal(&state, "shell-model", &args, &files, diff);
        let status = format_shell_status(&state, Some(&pending));
        let pending_diff = format_pending_diff(&pending);

        assert_eq!(
            pending.requested_files,
            vec!["README.md".to_string(), "CHANGELOG.md".to_string()]
        );
        assert_eq!(
            pending.proposed_files,
            vec!["README.md".to_string(), "CHANGELOG.md".to_string()]
        );
        assert!(status.contains("- README.md"));
        assert!(status.contains("- CHANGELOG.md"));
        assert!(pending_diff.contains("--- a/README.md"));
        assert!(pending_diff.contains("--- a/CHANGELOG.md"));
    }

    #[test]
    fn check_without_pending_proposal_is_clear_error() {
        let err = run_shell_check(None).unwrap_err();

        assert_eq!(err, NO_PENDING_PROPOSAL_MESSAGE);
    }

    #[test]
    fn diff_without_pending_proposal_is_clear_error() {
        let err = run_shell_diff(None).unwrap_err();

        assert_eq!(err, NO_PENDING_PROPOSAL_MESSAGE);
    }

    #[test]
    fn formats_pending_diff_without_secret_terms() {
        let pending = test_pending_proposal();
        let output = format_pending_diff(&pending);

        assert!(output.contains("Pending proposal:"));
        assert!(output.contains("instruction:  fix typo"));
        assert!(output.contains("checked:      no"));
        assert!(output.contains("requested:"));
        assert!(output.contains("- README.md"));
        assert!(output.contains("proposed:"));
        assert!(output.contains("Diff:"));
        assert!(output.contains("--- a/README.md"));
        assert!(!output.contains("git apply --check"));
        assert!(!output.contains("Apply this patch"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn formats_pending_diff_with_check_result() {
        let mut pending = test_pending_proposal();
        pending.check_state = PendingCheckState::Passed;
        let output = format_pending_diff(&pending);

        assert!(output.contains("checked:      yes"));
        assert!(output.contains("check_result: ok"));
    }

    #[test]
    fn apply_without_pending_proposal_is_clear_error() {
        let err = run_shell_apply(None).unwrap_err();

        assert_eq!(err, NO_PENDING_PROPOSAL_MESSAGE);
    }

    #[test]
    fn clearing_pending_proposal_drops_existing_diff() {
        let mut pending = Some(test_pending_proposal());

        clear_pending_proposal(&mut pending);

        assert_eq!(pending, None);
    }

    #[test]
    fn explicit_clear_without_pending_is_noop() {
        let mut pending = None;

        clear_pending_proposal_explicit(&mut pending);

        assert_eq!(pending, None);
    }

    #[test]
    fn shell_apply_outcome_marks_only_successful_apply_for_clear() {
        assert_eq!(ShellApplyOutcome::Applied, ShellApplyOutcome::Applied);
        assert_ne!(ShellApplyOutcome::Applied, ShellApplyOutcome::Rejected);
    }

    #[test]
    fn pending_status_messages_describe_state_transitions() {
        assert!(PENDING_CREATED_MESSAGE.contains("pending proposal created"));
        assert!(PENDING_READY_MESSAGE.contains("ready to apply"));
        assert!(PENDING_KEPT_MESSAGE.contains("kept"));
        assert!(PENDING_CLEARED_MESSAGE.contains("cleared"));
    }

    #[test]
    fn formats_model_list_from_shell_state_provider_and_current_model() {
        let state = test_state();
        let output = format_shell_model_list(
            &state,
            &[
                ModelInfo::new("shell-model"),
                ModelInfo::new("text-embedding-nomic-embed-text-v1.5"),
            ],
        );

        assert!(output.contains("Models for provider: lmstudio"));
        assert!(output.contains("base_url: http://localhost:1234/v1"));
        assert!(output.contains("current: shell-model"));
        assert!(output.contains("model_id"));
        assert!(output.contains("fallback_candidate"));
        assert!(output.contains("current"));
        assert!(output.contains("shell-model"));
        assert!(output.contains("yes"));
        assert!(output.contains("text-embedding-nomic-embed-text-v1.5"));
        assert!(output.contains("no"));
    }

    #[test]
    fn formats_provider_list_with_current_marker() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                base_url: Some("https://custom.example/api/v1".to_string()),
                api_key_env: Some("CUSTOM_OPENAI_KEY".to_string()),
                default_model: None,
            },
        );
        let config = UserConfig {
            providers,
            ..UserConfig::default()
        };
        let state = test_state();
        let registry = builtin_registry();
        let output = format_shell_provider_list(&state, &registry, Some(&config));

        assert!(output.contains("provider"));
        assert!(output.contains("current"));
        assert!(output.contains("lmstudio      yes"));
        assert!(output.contains("openai"));
        assert!(output.contains("https://custom.example/api/v1"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
        assert!(!output.contains("CUSTOM_OPENAI_KEY"));
    }

    #[test]
    fn formats_shell_sessions_as_recent_list_without_secret_terms() {
        let sessions = vec![
            test_session_summary(
                "20260506-112745-93b507",
                "chat",
                "lmstudio",
                "qwen3.5-35b-a3b-uncensored-hauhaucs-aggressive",
                "success",
            ),
            test_session_summary(
                "20260506-111000-abcdef",
                "propose",
                "lmstudio",
                "short-model",
                "error",
            ),
        ];

        let output = format_shell_sessions(&sessions);

        assert!(output.contains("Recent sessions:"));
        assert!(output.contains("latest"));
        assert!(output.contains("session_id"));
        assert!(output.contains("command"));
        assert!(output.contains("provider"));
        assert!(output.contains("model"));
        assert!(output.contains("status"));
        assert!(output.contains("yes      20260506-112745-93b507"));
        assert!(output.contains("chat"));
        assert!(output.contains("propose"));
        assert!(output.contains("lmstudio"));
        assert!(output.contains("success"));
        assert!(output.contains("error"));
        assert!(output.contains("origin sessions show <session-id>"));
        assert!(output.contains("origin sessions show --latest"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
        assert!(!output.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn formats_empty_shell_sessions() {
        let output = format_shell_sessions(&[]);

        assert!(output.contains("Recent sessions:"));
        assert!(output.contains("No sessions found."));
    }

    #[test]
    fn shell_model_uses_config_before_provider_fallback() {
        let provider = test_provider();
        let model = resolve_runtime_model(Some("config-model".to_string()), &provider);

        assert_eq!(model.id.as_deref(), Some("config-model"));
        assert_eq!(model.source, "config");
    }

    #[test]
    fn model_set_updates_shell_state_for_chat() {
        let mut state = test_state();
        state.set_model("google/gemma-4-31b").unwrap();

        assert_eq!(state.model_id.as_deref(), Some("google/gemma-4-31b"));
        assert_eq!(state.model_source, "shell");
        assert_eq!(state.model_error, None);
        assert_eq!(state.model_for_chat().unwrap(), "google/gemma-4-31b");
    }

    #[test]
    fn formats_model_set_success_with_provider_and_source() {
        let mut state = test_state();
        state.set_model("google/gemma-4-31b").unwrap();
        let output = format_model_set_success(&state);

        assert!(output.contains("model set to google/gemma-4-31b"));
        assert!(output.contains("provider:"));
        assert!(output.contains("lmstudio"));
        assert!(output.contains("model_source:"));
        assert!(output.contains("shell"));
    }

    #[test]
    fn model_set_rejects_empty_model() {
        let mut state = test_state();
        let err = state.set_model(" ").unwrap_err();

        assert_eq!(
            err,
            "Model id cannot be empty.\nUsage: /model set <model-id>"
        );
        assert_eq!(state.model_id.as_deref(), Some("shell-model"));
    }

    #[test]
    fn provider_apply_updates_shell_state() {
        let mut state = test_state();
        let result = state.apply_provider(
            test_provider_with_id("openai", "https://api.openai.com/v1"),
            "env".to_string(),
            RuntimeModel {
                id: Some("zai-org-glm-5-1".to_string()),
                source: "config".to_string(),
                error: None,
            },
        );

        assert_eq!(result.provider_id, "openai");
        assert_eq!(result.model_id.as_deref(), Some("zai-org-glm-5-1"));
        assert_eq!(state.provider_id, "openai");
        assert_eq!(state.base_url, "https://api.openai.com/v1");
        assert_eq!(state.auth_label, "env");
        assert_eq!(state.model_for_chat().unwrap(), "zai-org-glm-5-1");
    }

    #[test]
    fn formats_provider_set_success_with_base_url_auth_and_model_source() {
        let mut state = test_state();
        let result = state.apply_provider(
            test_provider_with_id("openai", "https://api.openai.com/v1"),
            "env".to_string(),
            RuntimeModel {
                id: Some("zai-org-glm-5-1".to_string()),
                source: "config".to_string(),
                error: None,
            },
        );
        let output = format_provider_set_success(&state, &result);

        assert!(output.contains("provider set to openai"));
        assert!(output.contains("base_url:"));
        assert!(output.contains("https://api.openai.com/v1"));
        assert!(output.contains("auth:"));
        assert!(output.contains("env"));
        assert!(output.contains("model re-resolved to zai-org-glm-5-1"));
        assert!(output.contains("model_source:"));
        assert!(output.contains("config"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
    }

    #[test]
    fn provider_set_rejects_unknown_provider() {
        let mut state = test_state();
        let err = state.set_provider("unknown-provider").unwrap_err();

        assert!(err.contains("Unknown provider `unknown-provider`."));
        assert_eq!(state.provider_id, "lmstudio");
    }

    #[test]
    fn auth_check_reports_missing_key_without_secret_terms() {
        let auth = ProviderAuth::BearerEnv {
            api_key_env: "ORIGIN_TEST_MISSING_KEY_PHASE_11_2".to_string(),
        };
        let err = ensure_auth_available("openai", &auth, None).unwrap_err();

        assert!(err.contains("Provider `openai` requires an API key."));
        assert!(err.contains("ORIGIN_TEST_MISSING_KEY_PHASE_11_2"));
        assert!(err.contains("Do not print the API key value."));
        assert!(!err.contains("Authorization"));
        assert!(!err.contains("Bearer"));
    }

    #[test]
    fn unresolved_model_keeps_error_for_chat_guard() {
        let mut state = test_state();
        state.model_id = None;
        state.model_source = "unresolved".to_string();
        state.model_error = Some("provider unavailable".to_string());

        let err = state.model_for_chat().unwrap_err();

        assert!(err.contains("provider unavailable"));
    }

    fn test_state() -> ShellState {
        ShellState {
            runtime: RuntimeState {
                provider: test_provider(),
                provider_id: "lmstudio".to_string(),
                provider_kind: ProviderKind::OpenAiCompatible.as_str().to_string(),
                base_url: "http://localhost:1234/v1".to_string(),
                auth_label: "none".to_string(),
                model_id: Some("shell-model".to_string()),
                model_source: "config".to_string(),
                model_error: None,
                config_path: "/home/user/.origin/config.toml".to_string(),
                sessions_root: "/home/user/.origin/sessions".to_string(),
                cwd: "/repo".to_string(),
                git_state: "clean".to_string(),
            },
            pending_proposal: None,
        }
    }

    fn test_pending_proposal() -> PendingProposal {
        PendingProposal {
            diff: "\
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new
"
            .to_string(),
            requested_files: vec!["README.md".to_string()],
            proposed_files: vec!["README.md".to_string()],
            instruction: "fix typo".to_string(),
            provider: "lmstudio".to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
            model: "shell-model".to_string(),
            check_state: PendingCheckState::NotChecked,
        }
    }

    fn test_session_summary(
        session_id: &str,
        command: &str,
        provider: &str,
        model: &str,
        status: &str,
    ) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            created_at: "2026-05-06T11:27:45Z".to_string(),
            command: command.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            status: status.to_string(),
            path: PathBuf::from(format!("/tmp/{session_id}.jsonl")),
        }
    }

    fn test_provider() -> OpenAiCompatibleProvider {
        test_provider_with_id("lmstudio", "http://localhost:1234/v1")
    }

    fn test_provider_with_id(id: &str, base_url: &str) -> OpenAiCompatibleProvider {
        OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
            id: ProviderId::new(id),
            base_url: base_url.to_string(),
            auth: ProviderAuth::None,
        })
    }
}
