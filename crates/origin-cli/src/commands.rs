use origin_protocol::chat::{ChatMessage, ChatRequest};
use origin_protocol::model::{select_fallback_model, ModelInfo};
use origin_protocol::patch::DiffProposal;
use origin_protocol::provider::ProviderAuth;
use origin_protocol::session::SessionEvent;

use origin_config::{load_user_config, resolve_config, resolve_config_path, CliOverrides};
use origin_provider::openai_compatible::OpenAiCompatibleProvider;
use origin_provider::openai_compatible::PROPOSE_CHAT_TIMEOUT;
use origin_provider::{builtin_registry, ModelProvider, ProviderError, ProviderRegistry};
use origin_session::{
    build_resume_context_excluding, default_sessions_root, format_event, list_sessions,
    read_latest_session, read_session_by_id, sha256_hex, ResumeContext, SessionRecord,
    SessionSummary, SessionWriter,
};
use origin_tools::{
    changed_files_from_diff, discover_git_repo, extract_diff_proposal, git_apply, git_apply_check,
    git_dirty_status, parse_approval_decision, read_workspace_files, validate_diff_integrity,
    validate_diff_targets, validate_diff_targets_allowing_new_files, ApprovalDecision,
    DiffIntegrityError, DiffValidationError, ReadFile, Workspace,
};

use std::env;
use std::io::{self, Write};

use crate::args::{Args, Command, HelpTopic, ModelAction, ProviderAction, SessionsAction};
use crate::chat_workflow;

const ORIGIN_MODEL_ENV: &str = "ORIGIN_MODEL";
const PROPOSE_MAX_TOKENS: u32 = 4096;
const INVALID_DIFF_RETRY_ATTEMPT: u8 = 1;
const APPLY_CHECK_RETRY_ATTEMPT: u8 = 1;

pub fn run(args: Args) -> Result<(), String> {
    match args.command {
        Command::Help => help(args.help_topic),
        Command::Version => version(),
        Command::Shell => crate::shell::run(),
        Command::Tui => crate::tui::run(),
        Command::Status => status(args),
        Command::Providers => providers(),
        Command::Models => models(args),
        Command::Provider => provider(args),
        Command::Model => model(args),
        Command::Chat => run_with_session("chat", |session| chat(args, session)),
        Command::Propose => run_with_session("propose", |session| propose(args, session)),
        Command::Sessions => sessions(args),
    }
}

fn version() -> Result<(), String> {
    println!("origin-agent {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn run_with_session(
    command: &str,
    execute: impl FnOnce(&mut SessionWriter) -> Result<(), String>,
) -> Result<(), String> {
    let mut session = SessionWriter::create(command).map_err(|error| error.to_string())?;
    let result = execute(&mut session);
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

fn providers() -> Result<(), String> {
    let registry = builtin_registry();
    let config = load_user_config().map_err(|error| error.to_string())?;

    println!(
        "{:<12}  {:<20}  {:<18}  base_url",
        "provider", "kind", "auth"
    );
    for preset in registry.presets() {
        let provider_config = config
            .as_ref()
            .and_then(|loaded| loaded.config.provider(preset.id.as_str()));
        let base_url = config
            .as_ref()
            .and_then(|loaded| {
                loaded
                    .config
                    .provider(preset.id.as_str())
                    .and_then(|provider| provider.base_url.as_deref())
            })
            .unwrap_or(&preset.base_url);
        let auth = provider_auth_label(
            &preset.auth,
            provider_config.and_then(|provider| provider.api_key_env.as_deref()),
        );
        println!(
            "{:<12}  {:<20}  {:<18}  {}",
            preset.id.as_str(),
            preset.kind.as_str(),
            auth,
            base_url
        );
    }

    Ok(())
}

fn provider(args: Args) -> Result<(), String> {
    match args.provider_action {
        ProviderAction::Show => {
            let selected = resolve_selected_provider(args.provider, args.base_url)?;
            print_selected_provider(&selected);
            Ok(())
        }
        ProviderAction::List => providers(),
    }
}

fn models(args: Args) -> Result<(), String> {
    let selected = resolve_selected_provider(args.provider, args.base_url)?;

    print_field("provider", selected.provider.id().as_str());
    print_field("base_url", selected.provider.base_url());
    println!();

    let models = selected
        .provider
        .list_models()
        .map_err(|error| error.user_message())?;

    println!("{:<72}  fallback_candidate", "model_id");
    for model in models {
        println!("{:<72}  {}", model.id, fallback_candidate_label(&model));
    }

    Ok(())
}

fn model(args: Args) -> Result<(), String> {
    match args.model_action {
        ModelAction::Show => {
            let selected = resolve_selected_provider(args.provider, args.base_url)?;
            let model = resolve_model(
                args.model,
                selected.default_model.clone(),
                &selected.provider,
            )?;
            print_selected_model(&selected, &model);
            Ok(())
        }
        ModelAction::List => models(args),
    }
}

fn status(args: Args) -> Result<(), String> {
    let selected = resolve_selected_provider(args.provider, args.base_url)?;
    let model = match resolve_model(
        args.model,
        selected.default_model.clone(),
        &selected.provider,
    ) {
        Ok(model) => StatusModel {
            id: model.id,
            source: model.source.as_str().to_string(),
        },
        Err(error) => StatusModel {
            id: "unresolved".to_string(),
            source: format!("error ({})", first_error_line(&error)),
        },
    };
    let config_path = display_config_path(selected.config_path.as_ref());
    let sessions = default_sessions_root()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("unknown ({error})"));
    let cwd = env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("unknown ({error})"));
    let workspace_instructions = env::current_dir()
        .map(chat_workflow::WorkspaceInstructions::load_from_cwd)
        .unwrap_or(chat_workflow::WorkspaceInstructions::None);
    let memory = chat_workflow::UserMemory::load_default();
    let policy = env::current_dir()
        .map(|path| origin_tools::policy::load_policy_from_env(&path))
        .unwrap_or_else(|_| origin_tools::policy::PolicySet::empty());
    let policy_label = if let Some(warning) = policy.warnings.first() {
        format!("ignored ({warning})")
    } else if policy.workspace_rules == 0 && policy.user_rules == 0 {
        "none".to_string()
    } else {
        let mut parts = Vec::new();
        if policy.workspace_rules > 0 {
            parts.push(format!("workspace {} rules", policy.workspace_rules));
        }
        if policy.user_rules > 0 {
            parts.push(format!("user {} rules", policy.user_rules));
        }
        parts.join(" · ")
    };

    print!(
        "{}",
        format_status_output(StatusOutput {
            provider: selected.provider.id().as_str(),
            model: &model.id,
            model_source: &model.source,
            base_url: selected.provider.base_url(),
            config: &config_path,
            sessions: &sessions,
            cwd: &cwd,
            git: &git_status_label(),
            command: "origin",
            workspace_instructions: &instruction_file_status_label(workspace_instructions.record()),
            memory: &instruction_file_status_label(memory.record()),
            policy: &policy_label,
        })
    );
    Ok(())
}

/// ORIGIN.md / memory.md の CLI status 用1行ラベル。
/// loaded はサイズ、skipped は理由を添え、無ければ "none"(本文は出さない)。
fn instruction_file_status_label(record: Option<chat_workflow::ChatFileContextRecord>) -> String {
    match record {
        None => "none".to_string(),
        Some(record) => match record.status.as_str() {
            "loaded" => format!(
                "{} (loaded, {} bytes)",
                record.path,
                record.bytes.unwrap_or(0)
            ),
            _ => format!(
                "{} (skipped, {})",
                record.path,
                record.reason.unwrap_or_else(|| "unknown".to_string())
            ),
        },
    }
}

struct StatusModel {
    id: String,
    source: String,
}

struct StatusOutput<'a> {
    provider: &'a str,
    model: &'a str,
    model_source: &'a str,
    base_url: &'a str,
    config: &'a str,
    sessions: &'a str,
    cwd: &'a str,
    git: &'a str,
    command: &'a str,
    workspace_instructions: &'a str,
    memory: &'a str,
    policy: &'a str,
}

fn format_status_output(status: StatusOutput<'_>) -> String {
    let mut output = String::new();
    output.push_str("origin-agent status\n\n");
    push_field(&mut output, "provider", status.provider);
    push_field(&mut output, "model", status.model);
    push_field(&mut output, "model_source", status.model_source);
    push_field(&mut output, "base_url", status.base_url);
    push_field(&mut output, "config", status.config);
    push_field(&mut output, "sessions", status.sessions);
    push_field(&mut output, "cwd", status.cwd);
    push_field(&mut output, "git", status.git);
    push_field(&mut output, "instructions", status.workspace_instructions);
    push_field(&mut output, "memory", status.memory);
    push_field(&mut output, "policy", status.policy);
    push_field(&mut output, "command", status.command);
    output
}

fn push_field(output: &mut String, label: &str, value: &str) {
    output.push_str(&format!("{:<13} {}\n", format!("{label}:"), value));
}

struct SelectedProvider {
    provider: OpenAiCompatibleProvider,
    kind: String,
    auth_label: String,
    config_path: Option<std::path::PathBuf>,
    default_model: Option<String>,
}

fn resolve_selected_provider(
    provider: Option<String>,
    base_url: Option<String>,
) -> Result<SelectedProvider, String> {
    let config =
        resolve_config(CliOverrides { provider, base_url }).map_err(|error| error.to_string())?;
    let registry = builtin_registry();
    let preset = registry
        .get_preset(config.provider.as_str())
        .ok_or_else(|| {
            format!(
                "Unknown provider `{}`. Try `origin provider list`.",
                config.provider.as_str()
            )
        })?;
    let kind = preset.kind.as_str().to_string();
    let auth_label = provider_auth_label(&preset.auth, config.api_key_env_override.as_deref());
    let provider = openai_provider(
        &registry,
        config.provider.as_str(),
        config.base_url_override,
        config.api_key_env_override,
    )?;

    Ok(SelectedProvider {
        provider,
        kind,
        auth_label,
        config_path: config.config_path,
        default_model: config.default_model,
    })
}

fn print_selected_provider(selected: &SelectedProvider) {
    print_field("provider", selected.provider.id().as_str());
    print_field("kind", &selected.kind);
    print_field("base_url", selected.provider.base_url());
    print_field("auth", &selected.auth_label);
    print_field(
        "config",
        &display_config_path(selected.config_path.as_ref()),
    );
}

fn print_selected_model(selected: &SelectedProvider, model: &ResolvedModel) {
    print_field("provider", selected.provider.id().as_str());
    print_field("model", &model.id);
    print_field("model_source", model.source.as_str());
    print_field(
        "fallback",
        if model.source == ModelResolutionSource::Fallback {
            "yes"
        } else {
            "no"
        },
    );
    print_field(
        "config",
        if model.source == ModelResolutionSource::Config {
            "yes"
        } else {
            "no"
        },
    );
}

fn display_config_path(config_path: Option<&std::path::PathBuf>) -> String {
    config_path
        .cloned()
        .or_else(resolve_config_path)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_status_label() -> String {
    match discover_git_repo() {
        Ok(repo) => match git_dirty_status(&repo) {
            Ok(status) if status.is_dirty => "dirty".to_string(),
            Ok(_) => "clean".to_string(),
            Err(error) => format!("unknown ({error})"),
        },
        Err(_) => "not a git repository".to_string(),
    }
}

fn first_error_line(error: &str) -> String {
    error.lines().next().unwrap_or(error).to_string()
}

fn sessions(args: Args) -> Result<(), String> {
    match args.sessions_action {
        SessionsAction::List => list_session_summaries(),
        SessionsAction::ShowLatest => {
            show_session(read_latest_session().map_err(|error| error.to_string())?)
        }
        SessionsAction::Show { session_id } => {
            show_session(read_session_by_id(&session_id).map_err(|error| error.to_string())?)
        }
    }
}

fn list_session_summaries() -> Result<(), String> {
    let sessions = list_sessions().map_err(|error| error.to_string())?;
    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!(
        "{:<25}  {:<20}  {:<8}  {:<10}  {:<30}  status",
        "session_id", "created_at", "command", "provider", "model"
    );
    for summary in sessions {
        print_session_summary(&summary);
    }

    Ok(())
}

fn print_session_summary(summary: &SessionSummary) {
    println!(
        "{:<25}  {:<20}  {:<8}  {:<10}  {:<30}  {}",
        summary.session_id,
        compact_timestamp(&summary.created_at),
        summary.command,
        summary.provider,
        truncate_middle(&summary.model, 30),
        summary.status
    );
}

fn show_session(record: SessionRecord) -> Result<(), String> {
    print_field("session", &record.summary.session_id);
    print_field("created", &record.summary.created_at);
    print_field("command", &record.summary.command);
    print_field("provider", &record.summary.provider);
    print_field("model", &record.summary.model);
    print_field("status", &record.summary.status);
    println!();
    println!("Events:");
    for event in &record.events {
        println!("{}", format_event(event));
    }

    Ok(())
}

fn compact_timestamp(timestamp: &str) -> String {
    timestamp
        .strip_suffix(":00Z")
        .unwrap_or(timestamp)
        .to_string()
}

fn print_field(label: &str, value: &str) {
    println!("{:<13} {}", format!("{label}:"), value);
}

fn truncate_middle(value: &str, width: usize) -> String {
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

fn chat(args: Args, session: &mut SessionWriter) -> Result<(), String> {
    let prompt = args
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .ok_or_else(|| "Missing prompt. Usage: origin chat \"hello\"".to_string())?
        .to_string();
    record_session_event(
        session,
        SessionEvent::UserMessage {
            text: prompt.clone(),
        },
    )?;
    let resume_context = load_resume_context(args.resume, session)?;
    let config = resolve_config(CliOverrides {
        provider: args.provider,
        base_url: args.base_url,
    })
    .map_err(|error| error.to_string())?;
    let registry = builtin_registry();
    let provider = openai_provider(
        &registry,
        config.provider.as_str(),
        config.base_url_override,
        config.api_key_env_override,
    )?;
    record_session_event(
        session,
        SessionEvent::ProviderResolved {
            provider: provider.id().to_string(),
            base_url: provider.base_url().to_string(),
        },
    )?;
    let model = resolve_model(args.model, config.default_model, &provider)?;
    print_auto_selected_model_notice(&model);
    record_session_event(
        session,
        SessionEvent::ModelResolved {
            model: model.id.clone(),
        },
    )?;

    print_field("provider", provider.id().as_str());
    print_field("base_url", provider.base_url());
    print_field("model", &model.id);
    print_field("model_source", model.source.as_str());
    if let Some(resume) = &resume_context {
        print_field("resumed_from", &resume.source_session_id);
    }
    println!();

    let prompt = prompt_with_resume_context(&prompt, resume_context.as_ref());
    let response = provider
        .complete_chat(ChatRequest {
            model: model.id,
            messages: vec![ChatMessage::user(prompt)],
            max_tokens: None,
        })
        .map_err(|error| error.user_message())?;
    record_session_event(
        session,
        SessionEvent::ModelResponse {
            text: response.content.clone(),
        },
    )?;

    println!("{}", response.content);

    Ok(())
}

fn propose(args: Args, session: &mut SessionWriter) -> Result<(), String> {
    validate_propose_options(args.check, args.apply)?;
    let instruction = args
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .ok_or_else(|| {
            "Missing instruction. Usage: origin propose \"fix typo\" README.md".to_string()
        })?
        .to_string();
    record_session_event(
        session,
        SessionEvent::UserMessage {
            text: instruction.clone(),
        },
    )?;
    let resume_context = load_resume_context(args.resume, session)?;
    if args.paths.is_empty() {
        return Err("Missing file path. Usage: origin propose \"fix typo\" README.md".to_string());
    }

    let workspace = Workspace::from_current_dir().map_err(|error| error.to_string())?;
    let policy = origin_tools::policy::load_policy_from_env(workspace.root());
    // deny は user の明示 intent より強い(P-4)。読む前・provider に送る前に拒否する。
    for path in &args.paths {
        if let Some(reason) = policy.read_denial_reason(path) {
            return Err(format!("`{path}` was not read — {reason}."));
        }
        if let Some(reason) = policy.write_denial_reason(path) {
            return Err(format!("Cannot propose changes to `{path}` — {reason}."));
        }
    }
    let files = read_workspace_files(&workspace, &args.paths).map_err(|error| error.to_string())?;
    for file in &files {
        record_session_event(
            session,
            SessionEvent::FileRead {
                path: file.relative_path.clone(),
                bytes: file.contents.len() as u64,
                sha256: sha256_hex(file.contents.as_bytes()),
            },
        )?;
    }
    let config = resolve_config(CliOverrides {
        provider: args.provider,
        base_url: args.base_url,
    })
    .map_err(|error| error.to_string())?;
    let registry = builtin_registry();
    let provider = openai_provider(
        &registry,
        config.provider.as_str(),
        config.base_url_override,
        config.api_key_env_override,
    )?;
    record_session_event(
        session,
        SessionEvent::ProviderResolved {
            provider: provider.id().to_string(),
            base_url: provider.base_url().to_string(),
        },
    )?;
    let model = resolve_model(args.model, config.default_model, &provider)?;
    print_auto_selected_model_notice(&model);
    record_session_event(
        session,
        SessionEvent::ModelResolved {
            model: model.id.clone(),
        },
    )?;

    print_field("provider", provider.id().as_str());
    print_field("base_url", provider.base_url());
    print_field("model", &model.id);
    print_field("model_source", model.source.as_str());
    print_field("files", &files.len().to_string());
    print_field("mode", propose_mode_label(args.check, args.apply));
    if args.retry_apply_check {
        print_field("retry_apply_check", "enabled");
    }
    if let Some(resume) = &resume_context {
        print_field("resumed_from", &resume.source_session_id);
    }
    println!();

    let model_id = model.id.clone();
    let mut request = |retry| {
        request_diff_proposal(
            &provider,
            &model_id,
            &instruction,
            &files,
            &[],
            resume_context
                .as_ref()
                .map(|resume| resume.context.as_str()),
            retry,
            false,
        )
    };
    let proposal = request_and_validate_diff_proposal(
        args.retry_invalid,
        &files,
        &policy,
        session,
        &mut request,
    )?;

    if proposal.is_empty() {
        println!("No changes proposed.");
    } else if args.check || args.apply || args.retry_apply_check {
        let (repo, proposal) = prepare_patch_for_git_flow(
            proposal,
            args.retry_apply_check,
            &files,
            &policy,
            session,
            &mut request,
        )?;
        if args.check {
            println!("Proposed patch:");
            println!();
            println!("{}", proposal.diff);
            println!();
            println!("git apply --check: ok");
            println!("No files were changed.");
        } else if args.apply {
            println!("Proposed patch:");
            println!();
            println!("{}", proposal.diff);
            apply_patch_with_approval(&repo, &proposal.diff, session)?;
        } else {
            println!("Proposed patch (not applied):");
            println!();
            println!("{}", proposal.diff);
            println!();
            println!("git apply --check: ok");
            println!("No files were changed.");
        }
    } else {
        println!("Proposed patch (not applied):");
        println!();
        println!("{}", proposal.diff);
    }

    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DiffRetryFeedback {
    Validation(String),
    ApplyCheck(String),
}

impl DiffRetryFeedback {
    fn message(&self) -> &str {
        match self {
            Self::Validation(message) | Self::ApplyCheck(message) => message,
        }
    }
}

fn prepare_patch_for_git_flow<F>(
    proposal: DiffProposal,
    retry_apply_check: bool,
    files: &[ReadFile],
    policy: &origin_tools::policy::PolicySet,
    session: &mut SessionWriter,
    request: &mut F,
) -> Result<(origin_tools::GitRepo, DiffProposal), String>
where
    F: FnMut(Option<DiffRetryFeedback>) -> Result<(String, DiffProposal), String>,
{
    let repo = discover_git_repo().map_err(|error| error.to_string())?;
    warn_if_dirty(&repo)?;
    let proposal = apply_check_with_optional_retry(
        proposal,
        retry_apply_check,
        files,
        policy,
        session,
        request,
        |diff, session| run_git_apply_check(&repo, diff, session),
    )?;
    Ok((repo, proposal))
}

fn apply_check_with_optional_retry<F, C>(
    proposal: DiffProposal,
    retry_apply_check: bool,
    files: &[ReadFile],
    policy: &origin_tools::policy::PolicySet,
    session: &mut SessionWriter,
    request: &mut F,
    mut apply_check: C,
) -> Result<DiffProposal, String>
where
    F: FnMut(Option<DiffRetryFeedback>) -> Result<(String, DiffProposal), String>,
    C: FnMut(&str, &mut SessionWriter) -> Result<(), String>,
{
    match apply_check(&proposal.diff, session) {
        Ok(()) => Ok(proposal),
        Err(error) if !retry_apply_check => Err(error),
        Err(error) => {
            record_session_event(
                session,
                SessionEvent::ApplyCheckRetryRequested {
                    error: error.clone(),
                },
            )?;
            record_session_event(
                session,
                SessionEvent::ApplyCheckRetryStarted {
                    attempt: APPLY_CHECK_RETRY_ATTEMPT,
                },
            )?;
            println!("git apply --check failed. Retrying once with apply-check feedback.");

            let (retry_response, retry_proposal) =
                match request(Some(DiffRetryFeedback::ApplyCheck(error))) {
                    Ok(result) => result,
                    Err(retry_error) => {
                        record_apply_check_retry_failed(session, &retry_error)?;
                        return Err(retry_error);
                    }
                };
            record_model_diff_response(session, &retry_response, &retry_proposal)?;

            if let Err(validation_error) =
                validate_proposed_diff(&retry_proposal.diff, files, policy, session)
            {
                record_apply_check_retry_failed(session, &validation_error)?;
                return Err(validation_error);
            }

            match apply_check(&retry_proposal.diff, session) {
                Ok(()) => {
                    record_session_event(
                        session,
                        SessionEvent::ApplyCheckRetrySucceeded {
                            attempt: APPLY_CHECK_RETRY_ATTEMPT,
                        },
                    )?;
                    Ok(retry_proposal)
                }
                Err(retry_error) => {
                    record_apply_check_retry_failed(session, &retry_error)?;
                    Err(retry_error)
                }
            }
        }
    }
}

fn request_and_validate_diff_proposal<F>(
    retry_invalid: bool,
    files: &[ReadFile],
    policy: &origin_tools::policy::PolicySet,
    session: &mut SessionWriter,
    mut request: F,
) -> Result<DiffProposal, String>
where
    F: FnMut(Option<DiffRetryFeedback>) -> Result<(String, DiffProposal), String>,
{
    let (response, proposal) = request(None)?;
    record_model_diff_response(session, &response, &proposal)?;

    match validate_proposed_diff(&proposal.diff, files, policy, session) {
        Ok(()) => Ok(proposal),
        Err(error) if retry_invalid => {
            record_session_event(
                session,
                SessionEvent::DiffRetryRequested {
                    reason: error.clone(),
                },
            )?;
            record_session_event(
                session,
                SessionEvent::DiffRetryStarted {
                    attempt: INVALID_DIFF_RETRY_ATTEMPT,
                },
            )?;
            println!("Initial diff failed validation. Retrying once with validation feedback.");

            let (retry_response, retry_proposal) =
                match request(Some(DiffRetryFeedback::Validation(error))) {
                    Ok(result) => result,
                    Err(retry_error) => {
                        record_diff_retry_failed(session, &retry_error)?;
                        return Err(retry_error);
                    }
                };
            record_model_diff_response(session, &retry_response, &retry_proposal)?;

            match validate_proposed_diff(&retry_proposal.diff, files, policy, session) {
                Ok(()) => {
                    record_session_event(
                        session,
                        SessionEvent::DiffRetrySucceeded {
                            attempt: INVALID_DIFF_RETRY_ATTEMPT,
                        },
                    )?;
                    Ok(retry_proposal)
                }
                Err(retry_error) => {
                    record_diff_retry_failed(session, &retry_error)?;
                    Err(retry_error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

fn request_diff_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
    context_files: &[ReadFile],
    resume_context: Option<&str>,
    retry_feedback: Option<DiffRetryFeedback>,
    allow_new_files: bool,
) -> Result<(String, DiffProposal), String> {
    let response = provider
        .complete_chat_with_timeout(
            ChatRequest {
                model: model_id.to_string(),
                messages: vec![ChatMessage::user(build_diff_prompt(
                    instruction,
                    files,
                    context_files,
                    resume_context,
                    retry_feedback.as_ref(),
                    allow_new_files,
                ))],
                max_tokens: Some(PROPOSE_MAX_TOKENS),
            },
            PROPOSE_CHAT_TIMEOUT,
        )
        .map_err(|error| propose_provider_error_message(&error))?;
    let proposal = extract_diff_proposal(&response.content).map_err(|error| error.to_string())?;
    Ok((response.content, proposal))
}

pub(crate) fn request_initial_diff_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
) -> Result<(String, DiffProposal), String> {
    request_diff_proposal(
        provider,
        model_id,
        instruction,
        files,
        &[],
        None,
        None,
        false,
    )
}

pub(crate) fn request_initial_new_file_body_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
    context_files: &[ReadFile],
) -> Result<(String, DiffProposal), String> {
    let target = confirmed_new_file_target(files)?;
    let response = provider
        .complete_chat_with_timeout(
            ChatRequest {
                model: model_id.to_string(),
                messages: vec![ChatMessage::user(build_new_file_body_prompt(
                    instruction,
                    target,
                    context_files,
                ))],
                max_tokens: Some(PROPOSE_MAX_TOKENS),
            },
            PROPOSE_CHAT_TIMEOUT,
        )
        .map_err(|error| propose_provider_error_message(&error))?;
    let body = extract_new_file_body_response(&response.content);
    let proposal = DiffProposal::new(construct_new_file_unified_diff(target, &body));
    Ok((response.content, proposal))
}

pub(crate) fn request_initial_existing_file_body_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
) -> Result<(String, DiffProposal), String> {
    request_existing_file_body_proposal(provider, model_id, instruction, files, None)
}

fn request_existing_file_body_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
    validation_error: Option<&str>,
) -> Result<(String, DiffProposal), String> {
    let target = confirmed_existing_file_target(files)?;
    let response = provider
        .complete_chat_with_timeout(
            ChatRequest {
                model: model_id.to_string(),
                messages: vec![ChatMessage::user(build_existing_file_body_prompt(
                    instruction,
                    target,
                    validation_error,
                ))],
                max_tokens: Some(PROPOSE_MAX_TOKENS),
            },
            PROPOSE_CHAT_TIMEOUT,
        )
        .map_err(|error| propose_provider_error_message(&error))?;
    let body = extract_existing_file_body_response(&response.content)?;
    let proposal = DiffProposal::new(construct_existing_file_unified_diff(
        &target.relative_path,
        &target.contents,
        &body,
    ));
    Ok((response.content, proposal))
}

pub(crate) fn request_retry_diff_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
    validation_error: &str,
) -> Result<(String, DiffProposal), String> {
    request_diff_proposal(
        provider,
        model_id,
        instruction,
        files,
        &[],
        None,
        Some(DiffRetryFeedback::Validation(validation_error.to_string())),
        false,
    )
}

pub(crate) fn request_retry_new_file_diff_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
    context_files: &[ReadFile],
    validation_error: &str,
) -> Result<(String, DiffProposal), String> {
    request_diff_proposal(
        provider,
        model_id,
        instruction,
        files,
        context_files,
        None,
        Some(DiffRetryFeedback::Validation(validation_error.to_string())),
        true,
    )
}

pub(crate) fn request_retry_existing_file_body_proposal(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
    model_id: &str,
    instruction: &str,
    files: &[ReadFile],
    validation_error: &str,
) -> Result<(String, DiffProposal), String> {
    request_existing_file_body_proposal(
        provider,
        model_id,
        instruction,
        files,
        Some(validation_error),
    )
}

pub(crate) fn record_constructed_new_file_response(
    session: &mut SessionWriter,
    proposal: &DiffProposal,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::ModelResponse {
            text: "[model returned new-file body; unified diff constructed locally]".to_string(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::DiffProposed {
            diff: proposal.diff.clone(),
        },
    )
}

pub(crate) fn record_constructed_existing_file_response(
    session: &mut SessionWriter,
    proposal: &DiffProposal,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::ModelResponse {
            text: "[model returned existing-file body; unified diff constructed locally]"
                .to_string(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::DiffProposed {
            diff: proposal.diff.clone(),
        },
    )
}

pub(crate) fn record_model_diff_response(
    session: &mut SessionWriter,
    response: &str,
    proposal: &DiffProposal,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::ModelResponse {
            text: response.to_string(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::DiffProposed {
            diff: proposal.diff.clone(),
        },
    )
}

pub(crate) fn validate_proposed_diff(
    diff: &str,
    files: &[ReadFile],
    policy: &origin_tools::policy::PolicySet,
    session: &mut SessionWriter,
) -> Result<(), String> {
    validate_proposed_diff_with_new_file_mode(diff, files, policy, session, false)
}

pub(crate) fn validate_proposed_diff_with_new_file_mode(
    diff: &str,
    files: &[ReadFile],
    policy: &origin_tools::policy::PolicySet,
    session: &mut SessionWriter,
    allow_new_files: bool,
) -> Result<(), String> {
    let requested_files = files
        .iter()
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();
    let validation = if allow_new_files {
        validate_diff_targets_allowing_new_files(diff, &requested_files, policy)
    } else {
        validate_diff_targets(diff, &requested_files, policy)
    };
    match validation {
        Ok(report) => {
            record_session_event(
                session,
                SessionEvent::DiffValidationSucceeded {
                    requested_files: report.requested_files.clone(),
                    proposed_files: report.proposed_files.clone(),
                },
            )?;
        }
        Err(error) => {
            record_diff_validation_failed(session, &error)?;
            return Err(error.to_string());
        }
    }

    match validate_diff_integrity(diff) {
        Ok(report) => {
            record_session_event(
                session,
                SessionEvent::DiffIntegritySucceeded {
                    file_count: report.file_count,
                    hunk_count: report.hunk_count,
                },
            )?;
            Ok(())
        }
        Err(error) => {
            record_diff_integrity_failed(session, &error)?;
            Err(error.to_string())
        }
    }
}

fn record_diff_validation_failed(
    session: &mut SessionWriter,
    error: &DiffValidationError,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::DiffValidationFailed {
            requested_files: error.requested_files.clone(),
            proposed_files: error.proposed_files.clone(),
            reason: error.reason.clone(),
        },
    )
}

fn record_diff_integrity_failed(
    session: &mut SessionWriter,
    error: &DiffIntegrityError,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::DiffIntegrityFailed {
            reason: error.reason.clone(),
        },
    )
}

fn record_diff_retry_failed(session: &mut SessionWriter, reason: &str) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::DiffRetryFailed {
            attempt: INVALID_DIFF_RETRY_ATTEMPT,
            reason: reason.to_string(),
        },
    )
}

fn record_apply_check_retry_failed(
    session: &mut SessionWriter,
    reason: &str,
) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::ApplyCheckRetryFailed {
            attempt: APPLY_CHECK_RETRY_ATTEMPT,
            reason: reason.to_string(),
        },
    )
}

fn propose_provider_error_message(error: &ProviderError) -> String {
    match error {
        ProviderError::Timeout {
            provider, base_url, ..
        } => format!(
            "Timed out while waiting for provider `{provider}` at {base_url} to produce a patch proposal.\n\n\
             The model may be taking too long for the current file scope.\n\
             Try a smaller model, narrow the file list, or increase the propose timeout when configuration support is available."
        ),
        _ => error.user_message(),
    }
}

fn apply_patch_with_approval(
    repo: &origin_tools::GitRepo,
    diff: &str,
    session: &mut SessionWriter,
) -> Result<(), String> {
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
            git_apply(&repo, diff).map_err(|error| error.to_string())?;
            println!("Patch applied.");
            let files = changed_files_from_diff(diff);
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
            Ok(())
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
            Ok(())
        }
    }
}

fn run_git_apply_check(
    repo: &origin_tools::GitRepo,
    diff: &str,
    session: &mut SessionWriter,
) -> Result<(), String> {
    record_session_event(session, SessionEvent::GitApplyCheckStarted)?;
    match git_apply_check(repo, diff) {
        Ok(()) => {
            record_session_event(session, SessionEvent::GitApplyCheckSucceeded)?;
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
            Err(message)
        }
    }
}

fn record_session_event(session: &mut SessionWriter, event: SessionEvent) -> Result<(), String> {
    session.record(event).map_err(|error| error.to_string())
}

fn load_resume_context(
    target: Option<String>,
    session: &mut SessionWriter,
) -> Result<Option<ResumeContext>, String> {
    let Some(target) = target else {
        return Ok(None);
    };
    let resume = build_resume_context_excluding(&target, session.path())
        .map_err(|error| error.to_string())?;
    record_session_event(
        session,
        SessionEvent::ResumedFrom {
            source_session_id: resume.source_session_id.clone(),
            source_path: resume.source_path.display().to_string(),
            context: resume.context.clone(),
        },
    )?;
    Ok(Some(resume))
}

fn prompt_with_resume_context(prompt: &str, resume: Option<&ResumeContext>) -> String {
    match resume {
        Some(resume) => format!(
            "{}\n\nCurrent user request:\n{}",
            resume.context,
            prompt.trim()
        ),
        None => prompt.to_string(),
    }
}

fn warn_if_dirty(repo: &origin_tools::GitRepo) -> Result<(), String> {
    let status = git_dirty_status(repo).map_err(|error| error.to_string())?;
    if status.is_dirty {
        eprintln!("Warning: working tree has existing changes.");
    }
    Ok(())
}

fn validate_propose_options(check: bool, apply: bool) -> Result<(), String> {
    if check && apply {
        Err("Cannot use --check and --apply together.".to_string())
    } else {
        Ok(())
    }
}

fn propose_mode_label(check: bool, apply: bool) -> &'static str {
    if check {
        "check (git apply --check only; no files changed)"
    } else if apply {
        "apply (requires y/yes approval)"
    } else {
        "preview (diff only; no files changed)"
    }
}

fn provider_auth_label(auth: &ProviderAuth, api_key_env_override: Option<&str>) -> String {
    match (auth, api_key_env_override) {
        (ProviderAuth::None, None) => "none".to_string(),
        (ProviderAuth::None, Some(_)) | (ProviderAuth::BearerEnv { .. }, _) => "env".to_string(),
    }
}

fn help(topic: HelpTopic) -> Result<(), String> {
    println!("{}", help_text(topic));
    Ok(())
}

fn help_text(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::Root => ROOT_HELP,
        HelpTopic::Shell => SHELL_HELP,
        HelpTopic::Tui => TUI_HELP,
        HelpTopic::Status => STATUS_HELP,
        HelpTopic::Providers => PROVIDERS_HELP,
        HelpTopic::Models => MODELS_HELP,
        HelpTopic::Provider => PROVIDER_HELP,
        HelpTopic::Model => MODEL_HELP,
        HelpTopic::Chat => CHAT_HELP,
        HelpTopic::Propose => PROPOSE_HELP,
        HelpTopic::Sessions => SESSIONS_HELP,
        HelpTopic::SessionsShow => SESSIONS_SHOW_HELP,
    }
}

const ROOT_HELP: &str = "\
origin-agent

Usage:
  origin
  origin tui
  origin shell
  origin <command> [options]
  origin --help
  origin --version
  origin help [command]

Commands:
  <no args>        Start the TUI workbench.
  tui              Start the TUI workbench explicitly.
  shell            Start the old interactive shell explicitly.
  status           Show current provider, model, config, session, and git state.
  provider         Show the currently selected provider.
  provider list    List configured provider presets.
  model            Show the currently selected model.
  model list       List provider models and fallback eligibility.
  providers        List configured provider presets.
  models           List provider models and fallback eligibility.
  chat             Send a single prompt.
  propose          Ask for a unified diff proposal for files.
  sessions         List saved session transcripts.
  sessions show    Show a saved session transcript.

Common options:
  --provider <id>      Provider preset. Default: lmstudio.
  --base-url <url>     Override provider base URL.
  --model <id>         Override model. Beats ORIGIN_MODEL and config.
  --resume <id|latest> Add a previous session summary as background-only context.
  --version            Show origin-agent version.

Safety:
  propose never changes files unless --apply is used.
  --apply still requires y/yes approval before git apply runs.
";

const SHELL_HELP: &str = "\
Usage:
  origin shell

Starts the old interactive shell explicitly.

For the default TUI workbench, run:
  origin

Shell commands include /status, /model, /provider, /sessions,
and the file edit loop /propose -> /diff -> /check -> /apply.
Safety behavior is unchanged: /apply requires y/yes approval.
";

const TUI_HELP: &str = "\
Usage:
  origin
  origin tui

Starts the TUI workbench.

Current scope:
  - status pane
  - chat pane
  - multiline composer
  - q or Ctrl+C to quit

If no model is configured, the TUI surfaces a no-model state and waits for /model.
Secret values are never displayed.
";

const STATUS_HELP: &str = "\
Usage:
  origin status [--provider <id>] [--base-url <url>] [--model <id>]

Shows current provider, model, config path, session root, cwd, and git state.
Secret values are never displayed.
";

const PROVIDER_HELP: &str = "\
Usage:
  origin provider [--provider <id>] [--base-url <url>]
  origin provider list

Shows the selected provider, or lists configured provider presets.
The auth field shows only the auth source, never secret values.
";

const PROVIDERS_HELP: &str = "\
Usage:
  origin providers

Lists built-in provider presets after config overrides.
The auth column shows only the auth source, never secret values.
";

const MODEL_HELP: &str = "\
Usage:
  origin model [--provider <id>] [--base-url <url>] [--model <id>]
  origin model list [--provider <id>] [--base-url <url>]

Shows the selected model using the normal resolution order, or lists provider models.
Model priority: --model, ORIGIN_MODEL, config default_model, provider fallback for non-interactive commands.
";

const MODELS_HELP: &str = "\
Usage:
  origin models [--provider <id>] [--base-url <url>]

Lists models from the selected OpenAI-compatible provider.
fallback_candidate shows whether non-interactive commands may auto-select that model when no model is configured.
";

const CHAT_HELP: &str = "\
Usage:
  origin chat [--provider <id>] [--base-url <url>] [--model <id>] [--resume <id|latest>] \"prompt\"

Sends one prompt to /v1/chat/completions and prints the response.
--resume adds a summarized previous session as background context only.
Model priority: --model, ORIGIN_MODEL, config default_model, provider fallback for non-interactive commands.
If provider fallback is used, origin prints an auto-selected model notice to stderr.
";

const PROPOSE_HELP: &str = "\
Usage:
  origin propose [options] \"instruction\" <file> [file...]

Options:
  --check      Generate a diff and run git apply --check. Does not apply.
  --apply      Generate a diff, ask for approval, then apply only on y/yes.
  --retry-invalid
               Retry once when the model returns a diff rejected by validation.
  --retry-apply-check
               Run git apply --check and retry once if it fails. Disabled by default.
  --provider <id>
  --base-url <url>
  --model <id>
  --resume <id|latest>

Default mode:
  Prints the proposed unified diff only. No files are changed.

Resume:
  --resume is background context only. The current instruction and current file contents still define the patch scope.

Safety:
  --check and --apply cannot be used together.
  --retry-apply-check never applies without --apply and y/yes approval.
  Enter at the approval prompt rejects the patch.
";

const SESSIONS_HELP: &str = "\
Usage:
  origin sessions
  origin sessions show --latest
  origin sessions show <session-id>

Lists saved JSONL session transcripts from ~/.origin/sessions.
";

const SESSIONS_SHOW_HELP: &str = "\
Usage:
  origin sessions show --latest
  origin sessions show <session-id>

Shows a saved session summary and its recorded events.
Sensitive auth header text is redacted by the session reader.
";

fn build_diff_prompt(
    instruction: &str,
    files: &[ReadFile],
    context_files: &[ReadFile],
    resume_context: Option<&str>,
    retry_feedback: Option<&DiffRetryFeedback>,
    allow_new_files: bool,
) -> String {
    let mut prompt = String::new();
    if allow_new_files {
        prompt.push_str(
            "You are creating exactly one new local file.\n\
         Think as needed internally.\n\
         Final answer must contain only a valid unified diff.\n\
         Do not include markdown fences.\n\
         Do not include prose, reasoning, notes, markdown headings, or commentary outside the diff.\n\
         Use unified diff format.\n\
         Create exactly one file.\n\
         The target path must be exactly the provided file path.\n\
         The created file must use --- /dev/null and +++ b/<target> file headers.\n\
         Use paths relative to the workspace root.\n\
         Create only the explicitly provided new file path.\n\
         Do not modify existing files, delete files, rename files, or add more files.\n\
         Include the complete file content as added lines.\n\
         Every content line must start with +.\n\
         Hunk header line counts must match the number of added content lines.\n\
         Only make changes directly required by the current user instruction.\n\
         If no new file content is clearly needed, return an empty diff.\n\n\
         Required output shape:\n\
         --- /dev/null\n\
         +++ b/path/from/workspace/root\n\
         @@ -0,0 +1 @@\n\
         +new text\n\n",
        );
    } else {
        prompt.push_str(
            "You are editing local files.\n\
         Think as needed internally.\n\
         Final answer must contain only one fenced diff block.\n\
         Do not include prose, reasoning, notes, markdown headings, or commentary outside the diff block.\n\
         Use unified diff format.\n\
         Every changed file must include --- a/<path> and +++ b/<path> file headers.\n\
         Do not return hunks without file headers.\n\
         Use paths relative to the workspace root.\n\
         Do not modify files outside the requested scope.\n\
         Always use the current file contents provided in this request as the source of truth.\n\
         Resume context is background only and is not current file content.\n\
         Only modify files explicitly provided in this request.\n\
         Only make changes directly required by the current user instruction.\n\
         Do not rename the project, restructure documentation, perform general cleanup, or expand scope unless explicitly requested.\n\
         Do not treat project names, product names, command names, file paths, config directory names, or code identifiers as typos unless the user explicitly names the exact text to change.\n\
         For vague requests such as \"fix typo\", only fix clear spelling or grammar mistakes in ordinary prose.\n\
         If no change is clearly needed, return an empty diff block.\n\n\
         Required output shape:\n\
         ```diff\n\
         --- a/path/from/workspace/root\n\
         +++ b/path/from/workspace/root\n\
         @@ -1 +1 @@\n\
         -old text\n\
         +new text\n\
         ```\n\n",
        );
    }
    if let Some(resume_context) = resume_context {
        prompt.push_str(resume_context);
        prompt.push_str("\n\n");
    }
    if let Some(retry_feedback) = retry_feedback {
        match retry_feedback {
            DiffRetryFeedback::Validation(_) => {
                prompt.push_str(
                    "Previous diff attempt was rejected by the CLI validation layer.\n\
                     Validation error:\n",
                );
                prompt.push_str(retry_feedback.message().trim());
                if allow_new_files {
                    let target = files
                        .first()
                        .map(|file| file.relative_path.as_str())
                        .unwrap_or("<target>");
                    prompt.push_str(
                        "\n\nReturn a corrected unified diff only.\n\
                         The invalid unified diff must be replaced completely.\n\
                         Confirmed target path:\n",
                    );
                    prompt.push_str(target);
                    prompt.push_str(
                        "\nRequirements:\n\
                         - Modify/create only the confirmed target path.\n\
                         - Use --- /dev/null and +++ b/<target> exactly for the confirmed target.\n\
                         - Hunk header line counts must match hunk body lines.\n\
                         - Every new file content line must start with +.\n\
                         - Do not broaden scope or modify any other file.\n\
                         - Do not include markdown fences.\n\
                         - Do not include explanations outside the diff.\n\n",
                    );
                } else {
                    prompt.push_str(
                        "\n\nReturn a corrected unified diff only.\n\
                         Keep the current file contents in this request as the source of truth.\n\
                         Do not broaden scope or modify files outside the requested files.\n\n",
                    );
                }
            }
            DiffRetryFeedback::ApplyCheck(_) => {
                prompt.push_str(
                    "Previous diff attempt passed format and scope validation, but git apply --check failed against the current workspace.\n\
                     git apply --check error:\n",
                );
                prompt.push_str(retry_feedback.message().trim());
                prompt.push_str(
                    "\n\nReturn a corrected unified diff only.\n\
                     Keep the current file contents in this request as the source of truth.\n\
                     Do not assume older resume context reflects current file contents.\n\
                     Do not broaden scope or modify files outside the requested files.\n\n",
                );
            }
        }
    }
    prompt.push_str("User instruction:\n");
    prompt.push_str(instruction);
    prompt.push_str("\n\nFiles:\n");

    for file in files {
        prompt.push_str("\nPath: ");
        prompt.push_str(&file.relative_path);
        prompt.push_str("\n```text\n");
        prompt.push_str(&file.contents);
        if !file.contents.ends_with('\n') {
            prompt.push('\n');
        }
        prompt.push_str("```\n");
    }
    if allow_new_files && !context_files.is_empty() {
        prompt.push_str(
            "\nRead-only context files. Use these as source material, but do not modify them:\n",
        );
        for file in context_files {
            prompt.push_str("\nPath: ");
            prompt.push_str(&file.relative_path);
            prompt.push_str("\n```text\n");
            prompt.push_str(&file.contents);
            if !file.contents.ends_with('\n') {
                prompt.push('\n');
            }
            prompt.push_str("```\n");
        }
    }

    prompt
}

fn build_new_file_body_prompt(
    instruction: &str,
    target: &str,
    context_files: &[ReadFile],
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are creating exactly one new local file.\n\
         Think as needed internally.\n\
         Final answer must be the complete file content only.\n\
         Do not include a unified diff.\n\
         Do not include patch headers, hunk headers, or line prefixes.\n\
         Do not include markdown fences unless they are literal content required inside the file.\n\
         Do not include prose, reasoning, notes, or commentary outside the file content.\n\
         Create content for exactly this target path:\n",
    );
    prompt.push_str(target);
    prompt.push_str(
        "\n\n\
         Return the complete file body for that target.\n\
         Do not create, delete, rename, or modify any other file.\n\
         If no meaningful file content is clearly needed, return NO_CHANGE.\n\n\
         User instruction:\n",
    );
    prompt.push_str(instruction);
    prompt.push_str("\n");

    if !context_files.is_empty() {
        prompt.push_str(
            "\nRead-only context files. Use these as source material, but do not modify them:\n",
        );
        for file in context_files {
            prompt.push_str("\nPath: ");
            prompt.push_str(&file.relative_path);
            prompt.push_str("\n```text\n");
            prompt.push_str(&file.contents);
            if !file.contents.ends_with('\n') {
                prompt.push('\n');
            }
            prompt.push_str("```\n");
        }
    }

    prompt
}

fn build_existing_file_body_prompt(
    instruction: &str,
    target: &ReadFile,
    validation_error: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are updating exactly one existing local file.\n\
         Think as needed internally.\n\
         Final answer must be the complete updated file content only.\n\
         Do not include a unified diff.\n\
         Do not include patch headers, hunk headers, or line prefixes.\n\
         Do not include markdown fences unless they are literal content required inside the file.\n\
         Do not include prose, reasoning, notes, or commentary outside the file content.\n\
         Update exactly this target path:\n",
    );
    prompt.push_str(&target.relative_path);
    prompt.push_str(
        "\n\n\
         Return the full replacement body for that target file.\n\
         Do not create, delete, rename, or modify any other file.\n\
         If no meaningful change is clearly needed, return NO_CHANGE.\n",
    );
    if let Some(validation_error) = validation_error {
        prompt.push_str(
            "\nPrevious full-file body attempt was rejected by the CLI validation layer.\n\
             Validation error:\n",
        );
        prompt.push_str(validation_error.trim());
        prompt.push_str(
            "\nReturn a corrected complete file body only. Do not return a diff or commentary.\n",
        );
    }
    prompt.push_str("\nUser instruction:\n");
    prompt.push_str(instruction);
    prompt.push_str("\n\nCurrent file content:\nPath: ");
    prompt.push_str(&target.relative_path);
    prompt.push_str("\n```text\n");
    prompt.push_str(&target.contents);
    if !target.contents.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push_str("```\n");

    prompt
}

fn confirmed_new_file_target(files: &[ReadFile]) -> Result<&str, String> {
    let [file] = files else {
        return Err("new-file proposal requires exactly one target path".to_string());
    };
    Ok(file.relative_path.as_str())
}

fn confirmed_existing_file_target(files: &[ReadFile]) -> Result<&ReadFile, String> {
    let [file] = files else {
        return Err("existing-file Agent preview requires exactly one target path".to_string());
    };
    Ok(file)
}

fn extract_new_file_body_response(response: &str) -> String {
    let trimmed = response.trim();
    let normalized_marker = trimmed.to_ascii_uppercase().replace([' ', '-'], "_");
    if matches!(
        normalized_marker.as_str(),
        "NO_CHANGE" | "NO_CHANGES" | "NO_CHANGE_NEEDED"
    ) {
        return String::new();
    }

    let lines = trimmed.lines().collect::<Vec<_>>();
    if lines.len() >= 2
        && lines
            .first()
            .is_some_and(|line| line.trim_start().starts_with("```"))
        && lines
            .last()
            .is_some_and(|line| line.trim_start().starts_with("```"))
    {
        return lines[1..lines.len() - 1].join("\n");
    }

    trimmed.to_string()
}

fn extract_existing_file_body_response(response: &str) -> Result<String, String> {
    let body = extract_new_file_body_response(response);
    if body.trim().is_empty() {
        return Ok(String::new());
    }
    if response_looks_like_unified_diff(&body) {
        return Err(
            "The model returned a diff instead of a complete updated file body. No pending proposal was stored. No files were changed."
                .to_string(),
        );
    }
    Ok(body)
}

fn response_looks_like_unified_diff(body: &str) -> bool {
    let lines = body
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.is_empty())
        .take(8)
        .collect::<Vec<_>>();
    lines
        .windows(2)
        .any(|pair| pair[0].starts_with("--- ") && pair[1].starts_with("+++ "))
        || lines.iter().any(|line| line.starts_with("@@ "))
}

pub(crate) fn construct_new_file_unified_diff(target: &str, body: &str) -> String {
    let mut normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.trim().is_empty() {
        return String::new();
    }
    if !normalized.ends_with('\n') {
        normalized.push('\n');
    }

    let lines = normalized.lines().collect::<Vec<_>>();
    let added_count = lines.len();
    let mut diff = String::new();
    diff.push_str("--- /dev/null\n");
    diff.push_str("+++ b/");
    diff.push_str(target);
    diff.push('\n');
    diff.push_str(&format!("@@ -0,0 +1,{added_count} @@\n"));
    for line in lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

pub(crate) fn construct_existing_file_unified_diff(
    target: &str,
    current: &str,
    updated: &str,
) -> String {
    let mut old = current.replace("\r\n", "\n").replace('\r', "\n");
    let mut new = updated.replace("\r\n", "\n").replace('\r', "\n");
    if new.trim().is_empty() || old == new {
        return String::new();
    }
    if !old.ends_with('\n') {
        old.push('\n');
    }
    if !new.ends_with('\n') {
        new.push('\n');
    }

    let old_lines = old.lines().collect::<Vec<_>>();
    let new_lines = new.lines().collect::<Vec<_>>();
    let old_count = old_lines.len();
    let new_count = new_lines.len();

    let mut diff = String::new();
    diff.push_str("--- a/");
    diff.push_str(target);
    diff.push('\n');
    diff.push_str("+++ b/");
    diff.push_str(target);
    diff.push('\n');
    diff.push_str(&format!("@@ -1,{old_count} +1,{new_count} @@\n"));
    for line in old_lines {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in new_lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

fn fallback_candidate_label(model: &ModelInfo) -> &'static str {
    if model.is_fallback_candidate() {
        "yes"
    } else {
        "no"
    }
}

fn select_provider_fallback_model(
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
) -> Result<String, String> {
    let models = provider
        .list_models()
        .map_err(|error| error.user_message())?;
    select_fallback_model(&models)
        .map(|model| model.id.clone())
        .ok_or_else(|| {
            format!(
                "Provider `{}` returned no fallback candidate models for chat.\n\
                 Pass --model <model-id> or configure default_model in ~/.origin/config.toml.",
                provider.id()
            )
        })
}

fn print_auto_selected_model_notice(model: &ResolvedModel) {
    if let Some(notice) = auto_selected_model_notice(model) {
        eprintln!("{notice}");
    }
}

fn auto_selected_model_notice(model: &ResolvedModel) -> Option<String> {
    if model.source == ModelResolutionSource::Fallback {
        Some(auto_selected_model_notice_text(&model.id))
    } else {
        None
    }
}

fn auto_selected_model_notice_text(model_id: &str) -> String {
    format!("using {model_id} (auto-selected; pass --model or set default_model to pin)")
}

#[derive(Debug, Eq, PartialEq)]
struct ResolvedModel {
    id: String,
    source: ModelResolutionSource,
}

#[derive(Debug, Eq, PartialEq)]
enum ModelResolutionSource {
    Cli,
    Env,
    Config,
    Fallback,
}

impl ModelResolutionSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Env => "env",
            Self::Config => "config",
            Self::Fallback => "fallback",
        }
    }
}

fn resolve_model(
    cli_model: Option<String>,
    config_model: Option<String>,
    provider: &origin_provider::openai_compatible::OpenAiCompatibleProvider,
) -> Result<ResolvedModel, String> {
    resolve_model_from_inputs(cli_model, origin_model_env(), config_model, || {
        select_provider_fallback_model(provider)
    })
}

fn origin_model_env() -> Option<String> {
    env::var(ORIGIN_MODEL_ENV)
        .ok()
        .and_then(|value| non_empty_model(value))
}

fn resolve_model_from_inputs<F>(
    cli_model: Option<String>,
    env_model: Option<String>,
    config_model: Option<String>,
    provider_fallback_model: F,
) -> Result<ResolvedModel, String>
where
    F: FnOnce() -> Result<String, String>,
{
    if let Some(id) = cli_model.and_then(non_empty_model) {
        return Ok(ResolvedModel {
            id,
            source: ModelResolutionSource::Cli,
        });
    }

    if let Some(id) = env_model.and_then(non_empty_model) {
        return Ok(ResolvedModel {
            id,
            source: ModelResolutionSource::Env,
        });
    }

    if let Some(id) = config_model.and_then(non_empty_model) {
        return Ok(ResolvedModel {
            id,
            source: ModelResolutionSource::Config,
        });
    }

    provider_fallback_model().map(|id| ResolvedModel {
        id,
        source: ModelResolutionSource::Fallback,
    })
}

fn non_empty_model(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn openai_provider(
    registry: &ProviderRegistry,
    provider_id: &str,
    base_url_override: Option<String>,
    api_key_env_override: Option<String>,
) -> Result<origin_provider::openai_compatible::OpenAiCompatibleProvider, String> {
    registry
        .openai_compatible_provider(provider_id, base_url_override, api_key_env_override)
        .ok_or_else(|| format!("Unknown provider `{provider_id}`. Try `origin providers`."))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_check_with_optional_retry, auto_selected_model_notice,
        auto_selected_model_notice_text, build_diff_prompt, build_existing_file_body_prompt,
        build_new_file_body_prompt, construct_existing_file_unified_diff,
        construct_new_file_unified_diff, extract_existing_file_body_response,
        extract_new_file_body_response, fallback_candidate_label, format_status_output, help_text,
        prompt_with_resume_context, propose_mode_label, provider_auth_label,
        request_and_validate_diff_proposal, resolve_model_from_inputs, validate_diff_integrity,
        validate_diff_targets_allowing_new_files, validate_propose_options, validate_proposed_diff,
        DiffProposal, DiffRetryFeedback, HelpTopic, ModelInfo, ModelResolutionSource, ProviderAuth,
        ReadFile, ResolvedModel, ResumeContext, SessionWriter, StatusOutput,
    };
    use origin_tools::changed_files_from_diff;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn rejects_check_and_apply_together() {
        let err = validate_propose_options(true, true).unwrap_err();

        assert_eq!(err, "Cannot use --check and --apply together.");
    }

    #[test]
    fn root_help_mentions_commands_and_safety() {
        let help = help_text(HelpTopic::Root);

        assert!(help.contains("providers"));
        assert!(help.contains("models"));
        assert!(help.contains("tui"));
        assert!(help.contains("shell"));
        assert!(help.contains("<no args>        Start the TUI workbench"));
        assert!(help.contains("origin shell"));
        assert!(help.contains("status"));
        assert!(help.contains("provider"));
        assert!(help.contains("model"));
        assert!(help.contains("chat"));
        assert!(help.contains("propose"));
        assert!(help.contains("sessions"));
        assert!(help.contains("--resume <id|latest>"));
        assert!(help.contains("background-only context"));
        assert!(help.contains("propose never changes files unless --apply is used"));
    }

    #[test]
    fn command_help_mentions_resume_scope() {
        let chat_help = help_text(HelpTopic::Chat);
        let propose_help = help_text(HelpTopic::Propose);
        let status_help = help_text(HelpTopic::Status);
        let shell_help = help_text(HelpTopic::Shell);
        let tui_help = help_text(HelpTopic::Tui);
        let provider_help = help_text(HelpTopic::Provider);
        let model_help = help_text(HelpTopic::Model);

        assert!(chat_help.contains("--resume"));
        assert!(chat_help.contains("background context only"));
        assert!(status_help.contains("origin status"));
        assert!(shell_help.contains("origin shell"));
        assert!(shell_help.contains("old interactive shell"));
        assert!(shell_help.contains("/propose -> /diff -> /check -> /apply"));
        assert!(shell_help.contains("y/yes"));
        assert!(tui_help.contains("origin tui"));
        assert!(tui_help.contains("origin"));
        assert!(tui_help.contains("TUI workbench"));
        assert!(provider_help.contains("origin provider"));
        assert!(model_help.contains("origin model"));
        assert!(propose_help.contains("--resume"));
        assert!(propose_help.contains("--retry-invalid"));
        assert!(propose_help.contains("--retry-apply-check"));
        assert!(propose_help.contains("background context only"));
        assert!(propose_help.contains("current instruction and current file contents"));
    }

    #[test]
    fn help_text_does_not_print_sensitive_auth_terms() {
        let help = [
            help_text(HelpTopic::Root),
            help_text(HelpTopic::Shell),
            help_text(HelpTopic::Tui),
            help_text(HelpTopic::Providers),
            help_text(HelpTopic::SessionsShow),
        ]
        .join("\n");

        assert!(!help.contains("Authorization"));
        assert!(!help.contains("Bearer"));
    }

    #[test]
    fn provider_auth_label_omits_secret_and_header_terms() {
        let label = provider_auth_label(
            &ProviderAuth::BearerEnv {
                api_key_env: "OPENAI_API_KEY".to_string(),
            },
            None,
        );

        assert_eq!(label, "env");
        assert!(!label.contains("API_KEY"));
        assert!(!label.contains("Authorization"));
        assert!(!label.contains("Bearer"));
    }

    #[test]
    fn status_output_omits_sensitive_auth_terms() {
        let output = format_status_output(StatusOutput {
            provider: "openai",
            model: "zai-org-glm-5-1",
            model_source: "config",
            base_url: "https://api.openai.com/v1",
            config: "/home/user/.origin/config.toml",
            sessions: "/home/user/.origin/sessions",
            cwd: "/repo",
            git: "clean",
            command: "origin",
            workspace_instructions: "none",
            memory: "none",
            policy: "none",
        });

        assert!(output.contains("origin-agent status"));
        assert!(output.contains("provider:"));
        assert!(!output.contains("Authorization"));
        assert!(!output.contains("Bearer"));
        assert!(!output.contains("API_KEY"));
        assert!(!output.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn propose_mode_labels_explain_file_change_behavior() {
        assert_eq!(
            propose_mode_label(false, false),
            "preview (diff only; no files changed)"
        );
        assert_eq!(
            propose_mode_label(true, false),
            "check (git apply --check only; no files changed)"
        );
        assert_eq!(
            propose_mode_label(false, true),
            "apply (requires y/yes approval)"
        );
    }

    #[test]
    fn resolves_cli_model_before_origin_model() {
        let resolved = resolve_model_from_inputs(
            Some("cli-model".to_string()),
            Some("env-model".to_string()),
            Some("config-model".to_string()),
            || panic!("provider model list should not be called when --model is set"),
        )
        .unwrap();

        assert_eq!(resolved.id, "cli-model");
        assert_eq!(resolved.source, ModelResolutionSource::Cli);
    }

    #[test]
    fn resolves_cli_embedding_like_model_before_fallback_filtering() {
        let resolved = resolve_model_from_inputs(
            Some("text-embedding-explicit".to_string()),
            Some("env-model".to_string()),
            Some("config-model".to_string()),
            || panic!("provider fallback should not be called when --model is set"),
        )
        .unwrap();

        assert_eq!(resolved.id, "text-embedding-explicit");
        assert_eq!(resolved.source, ModelResolutionSource::Cli);
    }

    #[test]
    fn resolves_origin_model_before_provider_fallback_model() {
        let resolved = resolve_model_from_inputs(
            None,
            Some("env-model".to_string()),
            Some("config-model".to_string()),
            || panic!("provider model list should not be called when ORIGIN_MODEL is set"),
        )
        .unwrap();

        assert_eq!(resolved.id, "env-model");
        assert_eq!(resolved.source, ModelResolutionSource::Env);
    }

    #[test]
    fn resolves_origin_model_even_if_embedding_like() {
        let resolved = resolve_model_from_inputs(
            None,
            Some("text-embedding-from-env".to_string()),
            Some("config-model".to_string()),
            || panic!("provider fallback should not be called when ORIGIN_MODEL is set"),
        )
        .unwrap();

        assert_eq!(resolved.id, "text-embedding-from-env");
        assert_eq!(resolved.source, ModelResolutionSource::Env);
    }

    #[test]
    fn resolves_config_model_before_provider_fallback_model() {
        let resolved =
            resolve_model_from_inputs(None, None, Some("config-model".to_string()), || {
                panic!("provider model list should not be called when config model is set")
            })
            .unwrap();

        assert_eq!(resolved.id, "config-model");
        assert_eq!(resolved.source, ModelResolutionSource::Config);
    }

    #[test]
    fn resolves_config_model_even_if_embedding_like() {
        let resolved = resolve_model_from_inputs(
            None,
            None,
            Some("text-embedding-from-config".to_string()),
            || panic!("provider fallback should not be called when config model is set"),
        )
        .unwrap();

        assert_eq!(resolved.id, "text-embedding-from-config");
        assert_eq!(resolved.source, ModelResolutionSource::Config);
    }

    #[test]
    fn falls_back_to_provider_fallback_model_without_cli_env_or_config_model() {
        let resolved = resolve_model_from_inputs(None, None, None, || {
            Ok("provider-fallback-model".to_string())
        })
        .unwrap();

        assert_eq!(resolved.id, "provider-fallback-model");
        assert_eq!(resolved.source, ModelResolutionSource::Fallback);
    }

    #[test]
    fn auto_selected_notice_text_matches_noninteractive_spec() {
        assert_eq!(
            auto_selected_model_notice_text("provider-fallback-model"),
            "using provider-fallback-model (auto-selected; pass --model or set default_model to pin)"
        );
    }

    #[test]
    fn auto_selected_notice_is_only_emitted_for_provider_fallback_source() {
        let fallback = ResolvedModel {
            id: "provider-fallback-model".to_string(),
            source: ModelResolutionSource::Fallback,
        };

        assert_eq!(
            auto_selected_model_notice(&fallback).as_deref(),
            Some(
                "using provider-fallback-model (auto-selected; pass --model or set default_model to pin)"
            )
        );

        for source in [
            ModelResolutionSource::Cli,
            ModelResolutionSource::Env,
            ModelResolutionSource::Config,
        ] {
            let resolved = ResolvedModel {
                id: "explicit-model".to_string(),
                source,
            };

            assert_eq!(auto_selected_model_notice(&resolved), None);
        }
    }

    #[test]
    fn ignores_blank_cli_and_env_models() {
        let resolved = resolve_model_from_inputs(
            Some("  ".to_string()),
            Some("\t\n".to_string()),
            Some(" ".to_string()),
            || Ok("provider-fallback-model".to_string()),
        )
        .unwrap();

        assert_eq!(resolved.id, "provider-fallback-model");
        assert_eq!(resolved.source, ModelResolutionSource::Fallback);
    }

    #[test]
    fn propagates_provider_fallback_failure() {
        let err = resolve_model_from_inputs(None, None, None, || {
            Err("no fallback candidates".to_string())
        })
        .unwrap_err();

        assert_eq!(err, "no fallback candidates");
    }

    #[test]
    fn formats_fallback_candidate_labels_for_model_rows() {
        assert_eq!(
            fallback_candidate_label(&ModelInfo::new("qwen3.5-35b")),
            "yes"
        );
        assert_eq!(
            fallback_candidate_label(&ModelInfo::new("text-embedding-nomic-embed-text-v1.5")),
            "no"
        );
    }

    #[test]
    fn builds_strict_propose_diff_prompt_contract() {
        let prompt = build_diff_prompt(
            "fix typo",
            &[ReadFile {
                relative_path: "README.md".to_string(),
                contents: "helo\n".to_string(),
            }],
            &[],
            None,
            None,
            false,
        );

        assert!(prompt.contains("Think as needed internally."));
        assert!(prompt.contains("Final answer must contain only one fenced diff block."));
        assert!(prompt.contains(
            "Do not include prose, reasoning, notes, markdown headings, or commentary outside the diff block."
        ));
        assert!(prompt.contains(
            "Every changed file must include --- a/<path> and +++ b/<path> file headers."
        ));
        assert!(prompt.contains("Do not return hunks without file headers."));
        assert!(prompt.contains(
            "Always use the current file contents provided in this request as the source of truth."
        ));
        assert!(
            prompt.contains("Resume context is background only and is not current file content.")
        );
        assert!(prompt.contains("Only modify files explicitly provided in this request."));
        assert!(
            prompt.contains("Only make changes directly required by the current user instruction.")
        );
        assert!(prompt.contains(
            "Do not rename the project, restructure documentation, perform general cleanup, or expand scope unless explicitly requested."
        ));
        assert!(prompt.contains(
            "Do not treat project names, product names, command names, file paths, config directory names, or code identifiers as typos unless the user explicitly names the exact text to change."
        ));
        assert!(prompt.contains(
            "For vague requests such as \"fix typo\", only fix clear spelling or grammar mistakes in ordinary prose."
        ));
        assert!(prompt.contains("If no change is clearly needed, return an empty diff block."));
        assert!(prompt.contains("Required output shape:"));
        assert!(prompt.contains("--- a/path/from/workspace/root"));
        assert!(prompt.contains("Path: README.md"));
    }

    #[test]
    fn build_diff_prompt_includes_resume_context_and_current_file_contents() {
        let prompt = build_diff_prompt(
            "fix typo",
            &[ReadFile {
                relative_path: "README.md".to_string(),
                contents: "current contents\n".to_string(),
            }],
            &[],
            Some("Previous session context:\n- session_id: previous"),
            None,
            false,
        );

        assert!(prompt.contains("Previous session context:"));
        assert!(prompt.contains("- session_id: previous"));
        assert!(prompt.contains("Path: README.md"));
        assert!(prompt.contains("current contents"));
    }

    #[test]
    fn build_diff_prompt_for_new_file_requires_dev_null_and_single_target() {
        let prompt = build_diff_prompt(
            "create summary",
            &[ReadFile {
                relative_path: "docs/new.md".to_string(),
                contents: String::new(),
            }],
            &[],
            None,
            None,
            true,
        );

        assert!(prompt.contains("creating exactly one new local file"));
        assert!(prompt.contains("Final answer must contain only a valid unified diff"));
        assert!(prompt.contains("Do not include markdown fences"));
        assert!(prompt.contains("--- /dev/null"));
        assert!(prompt.contains("+++ b/<target>"));
        assert!(prompt.contains("Hunk header line counts must match"));
        assert!(prompt.contains("Do not modify existing files"));
        assert!(prompt.contains("Path: docs/new.md"));
    }

    #[test]
    fn build_new_file_body_prompt_asks_for_content_not_diff() {
        let prompt = build_new_file_body_prompt(
            "create a project summary",
            "docs/summary.md",
            &[ReadFile {
                relative_path: "README.md".to_string(),
                contents: "Project notes\n".to_string(),
            }],
        );

        assert!(prompt.contains("Final answer must be the complete file content only."));
        assert!(prompt.contains("Do not include a unified diff."));
        assert!(prompt.contains("Do not include patch headers, hunk headers, or line prefixes."));
        assert!(prompt.contains("docs/summary.md"));
        assert!(prompt.contains("Path: README.md"));
    }

    #[test]
    fn build_existing_file_body_prompt_asks_for_complete_replacement_not_diff() {
        let file = ReadFile {
            relative_path: "README.md".to_string(),
            contents: "old text\n".to_string(),
        };
        let prompt =
            build_existing_file_body_prompt("update the explanation", &file, Some("invalid diff"));

        assert!(prompt.contains("updating exactly one existing local file"));
        assert!(prompt.contains("complete updated file content only"));
        assert!(prompt.contains("Do not include a unified diff"));
        assert!(prompt.contains("README.md"));
        assert!(prompt.contains("old text"));
        assert!(prompt.contains("invalid diff"));
    }

    #[test]
    fn constructs_valid_new_file_unified_diff_from_body() {
        let diff = construct_new_file_unified_diff("docs/summary.md", "# Summary\n\nBody");

        assert_eq!(
            diff,
            "--- /dev/null\n+++ b/docs/summary.md\n@@ -0,0 +1,3 @@\n+# Summary\n+\n+Body\n"
        );
        validate_diff_integrity(&diff).unwrap();
        validate_diff_targets_allowing_new_files(
            &diff,
            &["docs/summary.md".to_string()],
            &origin_tools::policy::PolicySet::empty(),
        )
        .unwrap();
    }

    #[test]
    fn constructs_valid_existing_file_unified_diff_from_full_body() {
        let diff = construct_existing_file_unified_diff(
            "README.md",
            "old line\nkeep\n",
            "new line\nkeep\n",
        );

        assert!(diff.contains("--- a/README.md"));
        assert!(diff.contains("+++ b/README.md"));
        assert!(diff.contains("-old line"));
        assert!(diff.contains("+new line"));
        assert_eq!(changed_files_from_diff(&diff), vec!["README.md"]);
        validate_diff_integrity(&diff).unwrap();
    }

    #[test]
    fn existing_file_body_response_rejects_diff_like_output() {
        let err = extract_existing_file_body_response(
            "--- a/README.md\n+++ b/OTHER.md\n@@ -1 +1 @@\n-old\n+new\n",
        )
        .unwrap_err();

        assert!(err.contains("complete updated file body"));
        assert!(err.contains("No files were changed"));
    }

    #[test]
    fn existing_file_body_response_constructs_single_target_diff() {
        let body = extract_existing_file_body_response("new line\nkeep\n").unwrap();
        let diff = construct_existing_file_unified_diff("README.md", "old line\nkeep\n", &body);

        assert_eq!(changed_files_from_diff(&diff), vec!["README.md"]);
        assert!(diff.contains("-old line"));
        assert!(diff.contains("+new line"));
        validate_diff_integrity(&diff).unwrap();
    }

    #[test]
    fn constructs_empty_new_file_diff_from_empty_body() {
        assert_eq!(
            construct_new_file_unified_diff("docs/summary.md", " \n\t"),
            ""
        );
        assert_eq!(construct_new_file_unified_diff("docs/summary.md", ""), "");
    }

    #[test]
    fn strips_single_surrounding_markdown_fence_from_new_file_body() {
        let body = extract_new_file_body_response("```markdown\n# Title\nBody\n```");
        assert_eq!(body, "# Title\nBody");
        let diff = construct_new_file_unified_diff("docs/title.md", &body);
        assert!(diff.contains("@@ -0,0 +1,2 @@"));
    }

    #[test]
    fn no_change_new_file_body_marker_becomes_empty_body() {
        assert_eq!(extract_new_file_body_response("NO_CHANGE"), "");
    }

    #[test]
    fn build_diff_prompt_for_new_file_includes_read_only_context() {
        let prompt = build_diff_prompt(
            "create summary",
            &[ReadFile {
                relative_path: "docs/new.md".to_string(),
                contents: String::new(),
            }],
            &[ReadFile {
                relative_path: "README.md".to_string(),
                contents: "source material\n".to_string(),
            }],
            None,
            None,
            true,
        );

        assert!(prompt.contains("Read-only context files"));
        assert!(prompt.contains("Use these as source material, but do not modify them"));
        assert!(prompt.contains("Path: README.md"));
        assert!(prompt.contains("source material"));
    }

    #[test]
    fn build_diff_prompt_includes_retry_validation_feedback() {
        let prompt = build_diff_prompt(
            "fix typo",
            &[ReadFile {
                relative_path: "README.md".to_string(),
                contents: "current contents\n".to_string(),
            }],
            &[],
            Some("Previous session context:\n- session_id: previous"),
            Some(&DiffRetryFeedback::Validation(
                "Reason: hunk header line count does not match hunk body".to_string(),
            )),
            false,
        );

        assert!(prompt.contains("Previous diff attempt was rejected"));
        assert!(prompt.contains("Validation error:"));
        assert!(prompt.contains("hunk header line count"));
        assert!(prompt.contains("current file contents in this request as the source of truth"));
        assert!(prompt.contains("Do not broaden scope"));
    }

    #[test]
    fn build_diff_prompt_for_new_file_retry_includes_strict_target_feedback() {
        let prompt = build_diff_prompt(
            "create summary",
            &[ReadFile {
                relative_path: "docs/new.md".to_string(),
                contents: String::new(),
            }],
            &[],
            None,
            Some(&DiffRetryFeedback::Validation(
                "Model returned an invalid unified diff.\nReason: hunk header line count does not match hunk body".to_string(),
            )),
            true,
        );

        assert!(prompt.contains("Previous diff attempt was rejected"));
        assert!(prompt.contains("invalid unified diff"));
        assert!(prompt.contains("hunk header line count does not match hunk body"));
        assert!(prompt.contains("Confirmed target path:\ndocs/new.md"));
        assert!(prompt.contains("Modify/create only the confirmed target path"));
        assert!(prompt.contains("Hunk header line counts must match hunk body lines"));
        assert!(prompt.contains("Do not include markdown fences"));
        assert!(prompt.contains("Do not include explanations outside the diff"));
    }

    #[test]
    fn build_diff_prompt_includes_apply_check_retry_feedback() {
        let prompt = build_diff_prompt(
            "fix typo",
            &[ReadFile {
                relative_path: "README.md".to_string(),
                contents: "current contents\n".to_string(),
            }],
            &[],
            Some("Previous session context:\n- session_id: previous"),
            Some(&DiffRetryFeedback::ApplyCheck(
                "git apply --check failed: patch does not apply".to_string(),
            )),
            false,
        );

        assert!(prompt.contains("git apply --check failed against the current workspace"));
        assert!(prompt.contains("patch does not apply"));
        assert!(prompt.contains("current file contents in this request as the source of truth"));
        assert!(prompt.contains("resume context reflects current file contents"));
        assert!(prompt.contains("Do not broaden scope"));
    }

    #[test]
    fn diff_validation_failure_records_event_before_patch_flow() {
        let root = temp_dir("diff-validation");
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let mut writer = SessionWriter::create_in(
            root.join("sessions"),
            cwd,
            "propose",
            UNIX_EPOCH + Duration::from_secs(1_777_944_225),
        )
        .unwrap();
        let diff = r#"--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1 +1 @@
-old
+new"#;
        let files = vec![ReadFile {
            relative_path: "README.md".to_string(),
            contents: "hello\n".to_string(),
        }];

        let err = validate_proposed_diff(
            diff,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert!(err.contains("outside the requested file scope"));
        assert!(transcript.contains("\"type\":\"diff_validation_failed\""));
        assert!(!transcript.contains("git_apply_check_started"));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn diff_integrity_failure_records_event_before_patch_flow() {
        let root = temp_dir("diff-integrity");
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let mut writer = SessionWriter::create_in(
            root.join("sessions"),
            cwd,
            "propose",
            UNIX_EPOCH + Duration::from_secs(1_777_944_226),
        )
        .unwrap();
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1,2 +1,2 @@
-old
+new"#;
        let files = vec![ReadFile {
            relative_path: "README.md".to_string(),
            contents: "hello\n".to_string(),
        }];

        let err = validate_proposed_diff(
            diff,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert!(err.contains("invalid unified diff"));
        assert!(transcript.contains("\"type\":\"diff_validation_succeeded\""));
        assert!(transcript.contains("\"type\":\"diff_integrity_failed\""));
        assert!(!transcript.contains("git_apply_check_started"));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn invalid_diff_does_not_retry_without_flag() {
        let root = temp_dir("retry-disabled");
        let mut writer = test_session(&root, "propose", 1_777_944_227);
        let files = readme_files();
        let mut attempts = 0;

        let err = request_and_validate_diff_proposal(
            false,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            |retry_feedback| {
                attempts += 1;
                assert!(retry_feedback.is_none());
                Ok((
                    invalid_count_diff().to_string(),
                    DiffProposal::new(invalid_count_diff()),
                ))
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(attempts, 1);
        assert!(err.contains("invalid unified diff"));
        assert!(!transcript.contains("diff_retry_requested"));
        assert!(!transcript.contains("diff_retry_started"));
    }

    #[test]
    fn retry_invalid_retries_once_and_succeeds_with_valid_diff() {
        let root = temp_dir("retry-success");
        let mut writer = test_session(&root, "propose", 1_777_944_228);
        let files = readme_files();
        let mut attempts = 0;

        let proposal =
            request_and_validate_diff_proposal(true, &files, &origin_tools::policy::PolicySet::empty(), &mut writer, |retry_feedback| {
                attempts += 1;
                if attempts == 1 {
                    assert!(retry_feedback.is_none());
                    Ok((
                        invalid_count_diff().to_string(),
                        DiffProposal::new(invalid_count_diff()),
                    ))
                } else {
                    let feedback = retry_feedback.expect("retry should include validation error");
                    assert_eq!(
                        feedback,
                        DiffRetryFeedback::Validation(
                            "Model returned an invalid unified diff.\nReason: hunk header line count does not match hunk body\nNothing was applied.".to_string()
                        )
                    );
                    Ok((
                        valid_readme_diff().to_string(),
                        DiffProposal::new(valid_readme_diff()),
                    ))
                }
            })
            .unwrap();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(attempts, 2);
        assert!(proposal.diff.contains("+new"));
        assert!(transcript.contains("\"type\":\"diff_retry_requested\""));
        assert!(transcript.contains("\"type\":\"diff_retry_started\""));
        assert!(transcript.contains("\"type\":\"diff_retry_succeeded\""));
        assert!(!transcript.contains("\"type\":\"diff_retry_failed\""));
    }

    #[test]
    fn retry_invalid_stops_after_one_failed_retry() {
        let root = temp_dir("retry-failed");
        let mut writer = test_session(&root, "propose", 1_777_944_229);
        let files = readme_files();
        let mut attempts = 0;

        let err = request_and_validate_diff_proposal(
            true,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            |retry_feedback| {
                attempts += 1;
                if attempts == 1 {
                    assert!(retry_feedback.is_none());
                } else {
                    assert!(retry_feedback.unwrap().message().contains("line count"));
                }
                Ok((
                    invalid_count_diff().to_string(),
                    DiffProposal::new(invalid_count_diff()),
                ))
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(attempts, 2);
        assert!(err.contains("invalid unified diff"));
        assert!(transcript.contains("\"type\":\"diff_retry_failed\""));
        assert!(!transcript.contains("git_apply_check_started"));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn retry_invalid_still_rejects_scope_outside_diff() {
        let root = temp_dir("retry-scope-failed");
        let mut writer = test_session(&root, "propose", 1_777_944_230);
        let files = readme_files();
        let mut attempts = 0;

        let err = request_and_validate_diff_proposal(
            true,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            |retry_feedback| {
                attempts += 1;
                if attempts == 1 {
                    assert!(retry_feedback.is_none());
                    Ok((
                        invalid_count_diff().to_string(),
                        DiffProposal::new(invalid_count_diff()),
                    ))
                } else {
                    assert!(retry_feedback.unwrap().message().contains("line count"));
                    Ok((
                        cargo_toml_diff().to_string(),
                        DiffProposal::new(cargo_toml_diff()),
                    ))
                }
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(attempts, 2);
        assert!(err.contains("outside the requested file scope"));
        assert!(transcript.contains("\"type\":\"diff_retry_failed\""));
        assert!(!transcript.contains("git_apply_check_started"));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn apply_check_failure_does_not_retry_without_flag() {
        let root = temp_dir("apply-check-retry-disabled");
        let mut writer = test_session(&root, "propose", 1_777_944_231);
        let files = readme_files();
        let mut request_attempts = 0;
        let mut check_attempts = 0;

        let err = apply_check_with_optional_retry(
            DiffProposal::new(valid_readme_diff()),
            false,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            &mut |feedback| {
                request_attempts += 1;
                assert!(feedback.is_some());
                Ok((
                    valid_readme_diff().to_string(),
                    DiffProposal::new(valid_readme_diff()),
                ))
            },
            |_, _| {
                check_attempts += 1;
                Err("git apply --check failed: patch does not apply".to_string())
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(request_attempts, 0);
        assert_eq!(check_attempts, 1);
        assert!(err.contains("patch does not apply"));
        assert!(!transcript.contains("apply_check_retry_requested"));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn retry_apply_check_retries_once_and_succeeds() {
        let root = temp_dir("apply-check-retry-success");
        let mut writer = test_session(&root, "propose", 1_777_944_232);
        let files = readme_files();
        let mut request_attempts = 0;
        let mut check_attempts = 0;

        let proposal = apply_check_with_optional_retry(
            DiffProposal::new(valid_readme_diff()),
            true,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            &mut |feedback| {
                request_attempts += 1;
                assert_eq!(
                    feedback,
                    Some(DiffRetryFeedback::ApplyCheck(
                        "git apply --check failed: patch does not apply".to_string()
                    ))
                );
                Ok((
                    valid_readme_diff().to_string(),
                    DiffProposal::new(valid_readme_diff()),
                ))
            },
            |_, _| {
                check_attempts += 1;
                if check_attempts == 1 {
                    Err("git apply --check failed: patch does not apply".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(request_attempts, 1);
        assert_eq!(check_attempts, 2);
        assert!(proposal.diff.contains("+new"));
        assert!(transcript.contains("\"type\":\"apply_check_retry_requested\""));
        assert!(transcript.contains("\"type\":\"apply_check_retry_started\""));
        assert!(transcript.contains("\"type\":\"apply_check_retry_succeeded\""));
        assert!(!transcript.contains("\"type\":\"apply_check_retry_failed\""));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn retry_apply_check_stops_after_second_apply_check_failure() {
        let root = temp_dir("apply-check-retry-failed");
        let mut writer = test_session(&root, "propose", 1_777_944_233);
        let files = readme_files();
        let mut request_attempts = 0;
        let mut check_attempts = 0;

        let err = apply_check_with_optional_retry(
            DiffProposal::new(valid_readme_diff()),
            true,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            &mut |feedback| {
                request_attempts += 1;
                assert!(matches!(feedback, Some(DiffRetryFeedback::ApplyCheck(_))));
                Ok((
                    valid_readme_diff().to_string(),
                    DiffProposal::new(valid_readme_diff()),
                ))
            },
            |_, _| {
                check_attempts += 1;
                Err(format!("git apply --check failed attempt {check_attempts}"))
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(request_attempts, 1);
        assert_eq!(check_attempts, 2);
        assert!(err.contains("attempt 2"));
        assert!(transcript.contains("\"type\":\"apply_check_retry_failed\""));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn retry_apply_check_rejects_invalid_retry_diff() {
        let root = temp_dir("apply-check-retry-invalid-diff");
        let mut writer = test_session(&root, "propose", 1_777_944_234);
        let files = readme_files();
        let mut request_attempts = 0;
        let mut check_attempts = 0;

        let err = apply_check_with_optional_retry(
            DiffProposal::new(valid_readme_diff()),
            true,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            &mut |feedback| {
                request_attempts += 1;
                assert!(matches!(feedback, Some(DiffRetryFeedback::ApplyCheck(_))));
                Ok((
                    invalid_count_diff().to_string(),
                    DiffProposal::new(invalid_count_diff()),
                ))
            },
            |_, _| {
                check_attempts += 1;
                Err("git apply --check failed: patch does not apply".to_string())
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(request_attempts, 1);
        assert_eq!(check_attempts, 1);
        assert!(err.contains("invalid unified diff"));
        assert!(transcript.contains("\"type\":\"diff_integrity_failed\""));
        assert!(transcript.contains("\"type\":\"apply_check_retry_failed\""));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn retry_apply_check_rejects_scope_outside_retry_diff() {
        let root = temp_dir("apply-check-retry-scope-outside");
        let mut writer = test_session(&root, "propose", 1_777_944_235);
        let files = readme_files();
        let mut request_attempts = 0;
        let mut check_attempts = 0;

        let err = apply_check_with_optional_retry(
            DiffProposal::new(valid_readme_diff()),
            true,
            &files,
            &origin_tools::policy::PolicySet::empty(),
            &mut writer,
            &mut |feedback| {
                request_attempts += 1;
                assert!(matches!(feedback, Some(DiffRetryFeedback::ApplyCheck(_))));
                Ok((
                    cargo_toml_diff().to_string(),
                    DiffProposal::new(cargo_toml_diff()),
                ))
            },
            |_, _| {
                check_attempts += 1;
                Err("git apply --check failed: patch does not apply".to_string())
            },
        )
        .unwrap_err();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert_eq!(request_attempts, 1);
        assert_eq!(check_attempts, 1);
        assert!(err.contains("outside the requested file scope"));
        assert!(transcript.contains("\"type\":\"diff_validation_failed\""));
        assert!(transcript.contains("\"type\":\"apply_check_retry_failed\""));
        assert!(!transcript.contains("approval_requested"));
    }

    #[test]
    fn chat_prompt_can_include_resume_context() {
        let resume = ResumeContext {
            source_session_id: "previous".to_string(),
            source_path: PathBuf::from("/sessions/previous.jsonl"),
            context: "Previous session context:\n- session_id: previous".to_string(),
        };

        let prompt = prompt_with_resume_context("continue", Some(&resume));

        assert!(prompt.contains("Previous session context:"));
        assert!(prompt.contains("Current user request:\ncontinue"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("origin-cli-{name}-{nanos}"))
    }

    fn test_session(root: &std::path::Path, command: &str, timestamp: u64) -> SessionWriter {
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        SessionWriter::create_in(
            root.join("sessions"),
            cwd,
            command,
            UNIX_EPOCH + Duration::from_secs(timestamp),
        )
        .unwrap()
    }

    fn readme_files() -> Vec<ReadFile> {
        vec![ReadFile {
            relative_path: "README.md".to_string(),
            contents: "old\n".to_string(),
        }]
    }

    fn valid_readme_diff() -> &'static str {
        r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new"#
    }

    fn invalid_count_diff() -> &'static str {
        r#"--- a/README.md
+++ b/README.md
@@ -1,2 +1,2 @@
-old
+new"#
    }

    fn cargo_toml_diff() -> &'static str {
        r#"--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1 +1 @@
-old
+new"#
    }

    fn first_jsonl(root: PathBuf) -> PathBuf {
        let year = fs::read_dir(root).unwrap().next().unwrap().unwrap().path();
        let month = fs::read_dir(year).unwrap().next().unwrap().unwrap().path();
        fs::read_dir(month).unwrap().next().unwrap().unwrap().path()
    }
}
