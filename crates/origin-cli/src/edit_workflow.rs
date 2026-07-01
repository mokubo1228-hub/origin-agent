use origin_protocol::session::SessionEvent;
use origin_session::{sha256_hex, SessionWriter};
use origin_tools::{
    changed_files_from_diff, discover_git_repo, git_apply_check, read_workspace_files,
    validate_diff_targets_allowing_new_files, ReadFile, Workspace,
};

use crate::commands;
use crate::runtime::{first_error_line, RuntimeState};
use crate::status_snapshot::{PendingProposalSnapshot, PendingProposalSnapshotInput};

pub(crate) const AGENT_FULL_FILE_UPDATE_MAX_BYTES: usize = 64 * 1024;

pub(crate) struct ProposePreviewRequest<'a> {
    pub(crate) runtime: &'a RuntimeState,
    pub(crate) paths: &'a [String],
    pub(crate) context_files: &'a [ReadFile],
    pub(crate) instruction: &'a str,
    pub(crate) mode: ProposePreviewMode,
    pub(crate) retry_invalid: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProposePreviewMode {
    ExistingFiles,
    AgentExistingFullFile,
    NewFile,
}

pub(crate) struct ProposePreviewPrepared<'a> {
    pub(crate) runtime: &'a RuntimeState,
    pub(crate) model: &'a str,
    pub(crate) file_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProposePreviewResult {
    pub(crate) pending: Option<PendingProposal>,
    pub(crate) diff: String,
    pub(crate) requested_files: Vec<String>,
    pub(crate) proposed_files: Vec<String>,
    pub(crate) provider: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) session_path: String,
    pub(crate) retry_notice: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProposePreviewError {
    pub(crate) error: String,
    pub(crate) session_path: String,
    pub(crate) partial: Option<ProposePreviewResult>,
    pub(crate) suggested_new_file_path: Option<String>,
    pub(crate) retry_notice: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProposePreviewFailure {
    error: String,
    suggested_new_file_path: Option<String>,
    retry_notice: Option<String>,
}

impl ProposePreviewFailure {
    fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            suggested_new_file_path: None,
            retry_notice: None,
        }
    }

    fn with_suggested_new_file_path(error: impl Into<String>, path: String) -> Self {
        Self {
            error: error.into(),
            suggested_new_file_path: Some(path),
            retry_notice: None,
        }
    }

    fn with_retry_notice(mut self, notice: String) -> Self {
        self.retry_notice = Some(notice);
        self
    }
}

impl From<String> for ProposePreviewFailure {
    fn from(error: String) -> Self {
        Self::new(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingProposal {
    pub(crate) diff: String,
    pub(crate) requested_files: Vec<String>,
    pub(crate) proposed_files: Vec<String>,
    pub(crate) instruction: String,
    pub(crate) provider: String,
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) check_state: PendingCheckState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingCheckState {
    NotChecked,
    Passed,
    Failed(String),
}

impl PendingCheckState {
    pub(crate) fn checked_label(&self) -> &'static str {
        match self {
            PendingCheckState::NotChecked => "no",
            PendingCheckState::Passed => "yes",
            PendingCheckState::Failed(_) => "failed",
        }
    }

    pub(crate) fn check_result(&self) -> Option<&str> {
        match self {
            PendingCheckState::NotChecked => None,
            PendingCheckState::Passed => Some("ok"),
            PendingCheckState::Failed(reason) => Some(reason.as_str()),
        }
    }
}

impl PendingProposal {
    pub(crate) fn from_valid_diff(
        runtime: &RuntimeState,
        model: &str,
        instruction: &str,
        files: &[ReadFile],
        diff: &str,
    ) -> Self {
        Self {
            diff: diff.to_string(),
            requested_files: files
                .iter()
                .map(|file| file.relative_path.clone())
                .collect(),
            proposed_files: changed_files_from_diff(diff),
            instruction: instruction.to_string(),
            provider: runtime.provider_id.clone(),
            base_url: runtime.base_url.clone(),
            model: model.to_string(),
            check_state: PendingCheckState::NotChecked,
        }
    }

    pub(crate) fn update_check_state(&mut self, result: &Result<(), String>) {
        match result {
            Ok(()) => self.check_state = PendingCheckState::Passed,
            Err(error) => self.check_state = PendingCheckState::Failed(first_error_line(error)),
        }
    }

    pub(crate) fn snapshot(&self) -> PendingProposalSnapshot {
        PendingProposalSnapshot::pending(PendingProposalSnapshotInput {
            checked: self.check_state.checked_label().to_string(),
            check_result: self.check_state.check_result().map(str::to_string),
            instruction: self.instruction.clone(),
            requested_files: self.requested_files.clone(),
            proposed_files: self.proposed_files.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
        })
    }
}

pub(crate) fn pending_proposal_snapshot(
    pending: Option<&PendingProposal>,
) -> PendingProposalSnapshot {
    pending
        .map(PendingProposal::snapshot)
        .unwrap_or_else(PendingProposalSnapshot::none)
}

pub(crate) fn run_propose_preview<F>(
    request: ProposePreviewRequest<'_>,
    mut on_prepared: F,
) -> Result<ProposePreviewResult, ProposePreviewError>
where
    F: FnMut(&ProposePreviewPrepared<'_>),
{
    let model = request
        .runtime
        .model_for_chat()
        .map_err(|error| ProposePreviewError {
            error,
            session_path: String::new(),
            partial: None,
            suggested_new_file_path: None,
            retry_notice: None,
        })?
        .to_string();
    let mut session = SessionWriter::create("propose").map_err(|error| ProposePreviewError {
        error: error.to_string(),
        session_path: String::new(),
        partial: None,
        suggested_new_file_path: None,
        retry_notice: None,
    })?;
    let result = run_propose_preview_with_session(request, &model, &mut session, &mut on_prepared);
    let status = if result.is_ok() { "success" } else { "error" };
    if let Err(error) = &result {
        let _ = session.record(SessionEvent::CommandFailed {
            error: error.error.clone(),
        });
    }
    let finish_result = record_session_event(
        &mut session,
        SessionEvent::SessionFinished {
            status: status.to_string(),
        },
    );
    let session_path = session.path().display().to_string();

    match (result, finish_result) {
        (Ok(mut result), Ok(())) => {
            result.session_path = session_path;
            Ok(result)
        }
        (Ok(mut result), Err(error)) => {
            result.session_path = session_path.clone();
            Err(ProposePreviewError {
                error,
                session_path,
                partial: Some(result),
                suggested_new_file_path: None,
                retry_notice: None,
            })
        }
        (Err(error), _) => Err(ProposePreviewError {
            error: error.error,
            session_path,
            partial: None,
            suggested_new_file_path: error.suggested_new_file_path,
            retry_notice: error.retry_notice,
        }),
    }
}

fn run_propose_preview_with_session<F>(
    request: ProposePreviewRequest<'_>,
    model: &str,
    session: &mut SessionWriter,
    on_prepared: &mut F,
) -> Result<ProposePreviewResult, ProposePreviewFailure>
where
    F: FnMut(&ProposePreviewPrepared<'_>),
{
    record_session_event(
        session,
        SessionEvent::UserMessage {
            text: request.instruction.to_string(),
        },
    )?;

    let workspace = Workspace::from_current_dir().map_err(|error| error.to_string())?;
    let files = match request.mode {
        ProposePreviewMode::ExistingFiles => {
            read_workspace_files(&workspace, request.paths).map_err(|error| error.to_string())?
        }
        ProposePreviewMode::AgentExistingFullFile => {
            prepare_agent_existing_full_file_targets(&workspace, request.paths)?
        }
        ProposePreviewMode::NewFile => prepare_new_file_targets(&workspace, request.paths)?,
    };
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
    for file in request.context_files {
        record_session_event(
            session,
            SessionEvent::FileRead {
                path: file.relative_path.clone(),
                bytes: file.contents.len() as u64,
                sha256: sha256_hex(file.contents.as_bytes()),
            },
        )?;
    }
    record_session_event(
        session,
        SessionEvent::ProviderResolved {
            provider: request.runtime.provider_id.clone(),
            base_url: request.runtime.base_url.clone(),
        },
    )?;
    record_session_event(
        session,
        SessionEvent::ModelResolved {
            model: model.to_string(),
        },
    )?;

    on_prepared(&ProposePreviewPrepared {
        runtime: request.runtime,
        model,
        file_count: files.len(),
    });

    let (response, proposal, diff_constructed_locally) = match request.mode {
        ProposePreviewMode::ExistingFiles => commands::request_initial_diff_proposal(
            &request.runtime.provider,
            model,
            request.instruction,
            &files,
        )
        .map(|(response, proposal)| (response, proposal, false))?,
        ProposePreviewMode::AgentExistingFullFile => {
            commands::request_initial_existing_file_body_proposal(
                &request.runtime.provider,
                model,
                request.instruction,
                &files,
            )
            .map(|(response, proposal)| (response, proposal, true))?
        }
        ProposePreviewMode::NewFile => commands::request_initial_new_file_body_proposal(
            &request.runtime.provider,
            model,
            request.instruction,
            &files,
            request.context_files,
        )
        .map(|(response, proposal)| (response, proposal, true))?,
    };
    if diff_constructed_locally {
        record_constructed_response_for_mode(session, request.mode, &proposal)?;
    } else {
        commands::record_model_diff_response(session, &response, &proposal)?;
    }
    let (proposal, retry_notice) =
        validate_or_retry_proposal(&request, model, session, &files, proposal)?;

    let requested_files = files
        .iter()
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();
    let proposed_files = changed_files_from_diff(&proposal.diff);
    if request.mode == ProposePreviewMode::NewFile
        && !new_file_diff_has_meaningful_added_content(&proposal.diff)
    {
        return Err(ProposePreviewFailure::new(
            "The model returned an empty new-file proposal. No pending proposal was stored."
                .to_string(),
        ));
    }
    if request.mode == ProposePreviewMode::AgentExistingFullFile && proposal.is_empty() {
        return Err(ProposePreviewFailure::new(
            "The model returned an empty full-file update proposal. No pending proposal was stored. No files were changed."
                .to_string(),
        ));
    }
    let pending = if proposal.is_empty() {
        None
    } else {
        Some(PendingProposal::from_valid_diff(
            request.runtime,
            model,
            request.instruction,
            &files,
            &proposal.diff,
        ))
    };

    Ok(ProposePreviewResult {
        pending,
        diff: proposal.diff,
        requested_files,
        proposed_files,
        provider: request.runtime.provider_id.clone(),
        base_url: request.runtime.base_url.clone(),
        model: model.to_string(),
        session_path: String::new(),
        retry_notice,
    })
}

fn validate_or_retry_proposal(
    request: &ProposePreviewRequest<'_>,
    model: &str,
    session: &mut SessionWriter,
    files: &[ReadFile],
    proposal: origin_protocol::patch::DiffProposal,
) -> Result<(origin_protocol::patch::DiffProposal, Option<String>), ProposePreviewFailure> {
    validate_or_retry_proposal_with_request(
        request.mode,
        request.retry_invalid,
        files,
        session,
        proposal,
        |error| match request.mode {
            ProposePreviewMode::ExistingFiles => commands::request_retry_diff_proposal(
                &request.runtime.provider,
                model,
                request.instruction,
                files,
                error,
            ),
            ProposePreviewMode::AgentExistingFullFile => {
                commands::request_retry_existing_file_body_proposal(
                    &request.runtime.provider,
                    model,
                    request.instruction,
                    files,
                    error,
                )
            }
            ProposePreviewMode::NewFile => commands::request_retry_new_file_diff_proposal(
                &request.runtime.provider,
                model,
                request.instruction,
                files,
                request.context_files,
                error,
            ),
        },
    )
}

fn validate_or_retry_proposal_with_request<F>(
    mode: ProposePreviewMode,
    retry_invalid: bool,
    files: &[ReadFile],
    session: &mut SessionWriter,
    proposal: origin_protocol::patch::DiffProposal,
    retry_request: F,
) -> Result<(origin_protocol::patch::DiffProposal, Option<String>), ProposePreviewFailure>
where
    F: FnOnce(&str) -> Result<(String, origin_protocol::patch::DiffProposal), String>,
{
    match validate_preview_diff(mode, &proposal.diff, files, session) {
        Ok(()) => return Ok((proposal, None)),
        Err(error) => {
            if mode == ProposePreviewMode::NewFile
                && !new_file_diff_has_meaningful_added_content(&proposal.diff)
            {
                return Err(ProposePreviewFailure::new(
                    "The model returned an empty new-file proposal. No pending proposal was stored.",
                ));
            }
            if let Some(path) = different_new_file_path_suggestion(&proposal.diff, files) {
                return Err(ProposePreviewFailure::with_suggested_new_file_path(
                    error, path,
                ));
            }
            if !retry_invalid {
                return Err(ProposePreviewFailure::new(error));
            }

            record_session_event(
                session,
                SessionEvent::DiffRetryRequested {
                    reason: error.clone(),
                },
            )?;
            record_session_event(session, SessionEvent::DiffRetryStarted { attempt: 1 })?;

            let retry_notice =
                "The model returned an invalid diff.\nRetrying once with validation feedback..."
                    .to_string();
            let retry_result = retry_request(&error);
            let (retry_response, retry_proposal) = match retry_result {
                Ok(result) => result,
                Err(retry_error) => {
                    record_diff_retry_failed(session, &retry_error)?;
                    return Err(
                        ProposePreviewFailure::new(retry_error).with_retry_notice(retry_notice)
                    );
                }
            };
            if mode == ProposePreviewMode::AgentExistingFullFile {
                record_constructed_response_for_mode(session, mode, &retry_proposal)?;
            } else {
                commands::record_model_diff_response(session, &retry_response, &retry_proposal)?;
            }

            match validate_preview_diff(mode, &retry_proposal.diff, files, session) {
                Ok(()) => {
                    record_session_event(session, SessionEvent::DiffRetrySucceeded { attempt: 1 })?;
                    Ok((retry_proposal, Some(retry_notice)))
                }
                Err(retry_error) => {
                    record_diff_retry_failed(session, &retry_error)?;
                    Err(ProposePreviewFailure::new(format!(
                        "The model still returned an invalid diff. No pending proposal was stored. No files were changed.\nReason: {retry_error}"
                    ))
                    .with_retry_notice(retry_notice))
                }
            }
        }
    }
}

fn validate_preview_diff(
    mode: ProposePreviewMode,
    diff: &str,
    files: &[ReadFile],
    session: &mut SessionWriter,
) -> Result<(), String> {
    commands::validate_proposed_diff_with_new_file_mode(
        diff,
        files,
        session,
        mode == ProposePreviewMode::NewFile,
    )?;
    if mode == ProposePreviewMode::AgentExistingFullFile && !diff.trim().is_empty() {
        let repo = discover_git_repo().map_err(|error| {
            format!(
                "git apply --check failed: {}",
                first_error_line(&error.to_string())
            )
        })?;
        git_apply_check(&repo, diff).map_err(|error| {
            format!(
                "git apply --check failed: {}",
                first_error_line(&error.to_string())
            )
        })?;
    }
    Ok(())
}

fn record_constructed_response_for_mode(
    session: &mut SessionWriter,
    mode: ProposePreviewMode,
    proposal: &origin_protocol::patch::DiffProposal,
) -> Result<(), String> {
    match mode {
        ProposePreviewMode::AgentExistingFullFile => {
            commands::record_constructed_existing_file_response(session, proposal)
        }
        ProposePreviewMode::NewFile => {
            commands::record_constructed_new_file_response(session, proposal)
        }
        ProposePreviewMode::ExistingFiles => {
            commands::record_model_diff_response(session, "", proposal)
        }
    }
}

fn new_file_diff_has_meaningful_added_content(diff: &str) -> bool {
    diff.lines().any(|line| {
        line.starts_with('+') && !line.starts_with("+++") && !line[1..].trim().is_empty()
    })
}

fn different_new_file_path_suggestion(diff: &str, requested_files: &[ReadFile]) -> Option<String> {
    let [requested] = requested_files else {
        return None;
    };
    let proposed_files = changed_files_from_diff(diff);
    let [proposed] = proposed_files.as_slice() else {
        return None;
    };
    if proposed == &requested.relative_path {
        return None;
    }
    validate_diff_targets_allowing_new_files(diff, std::slice::from_ref(proposed)).ok()?;
    Some(proposed.clone())
}

fn prepare_new_file_targets(
    workspace: &Workspace,
    paths: &[String],
) -> Result<Vec<ReadFile>, String> {
    let [path] = paths else {
        return Err("new-file proposal requires exactly one target path".to_string());
    };

    if std::path::Path::new(path).is_absolute() || path.contains("..") {
        return Err(
            "new-file target must be a relative path inside the current workspace".to_string(),
        );
    }
    if let Some(reason) = unsafe_new_file_target_reason(path) {
        return Err(reason);
    }
    if workspace.resolve_existing_file(path).is_ok() {
        return Err(format!("new-file target `{path}` already exists"));
    }
    let parent = std::path::Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().to_string());
    if let Some(parent) = parent {
        let parent_path = workspace
            .resolve_existing_file(&parent)
            .map_err(|error| error.to_string())?;
        if !parent_path.is_dir() {
            return Err(format!("new-file parent `{parent}` is not a directory"));
        }
    }

    Ok(vec![ReadFile {
        relative_path: path.clone(),
        contents: String::new(),
    }])
}

fn prepare_agent_existing_full_file_targets(
    workspace: &Workspace,
    paths: &[String],
) -> Result<Vec<ReadFile>, String> {
    let [path] = paths else {
        return Err("existing-file Agent preview requires exactly one target path".to_string());
    };
    let files = read_workspace_files(workspace, std::slice::from_ref(path))
        .map_err(|error| error.to_string())?;
    let [file] = files.as_slice() else {
        return Err("existing-file Agent preview requires exactly one target path".to_string());
    };
    if file.contents.len() > AGENT_FULL_FILE_UPDATE_MAX_BYTES {
        return Err(
            "This file is too large for a full-file Agent update preview.\n\
             Please make the instruction narrower, specify a target section, choose a smaller file, or provide an explicit narrower target.\n\
             No files changed."
                .to_string(),
        );
    }
    Ok(files)
}

fn unsafe_new_file_target_reason(path: &str) -> Option<String> {
    let components = std::path::Path::new(path)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| component.starts_with('.'))
    {
        return Some("new-file target must not be hidden".to_string());
    }

    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();
    if filename == ".env" || filename.starts_with(".env.") {
        return Some("new-file target must not be an environment file".to_string());
    }
    if filename.ends_with(".pem")
        || filename.ends_with(".key")
        || filename.ends_with(".p12")
        || filename.ends_with(".pfx")
        || filename == "id_rsa"
        || filename == "id_ed25519"
        || filename.contains("api_key")
        || filename.contains("apikey")
        || filename.contains("private_key")
        || filename.contains("secret")
        || filename.contains("token")
        || filename.contains("credential")
    {
        return Some("new-file target must not be secret-like".to_string());
    }

    None
}

fn record_session_event(session: &mut SessionWriter, event: SessionEvent) -> Result<(), String> {
    session.record(event).map_err(|error| error.to_string())
}

fn record_diff_retry_failed(session: &mut SessionWriter, reason: &str) -> Result<(), String> {
    record_session_event(
        session,
        SessionEvent::DiffRetryFailed {
            attempt: 1,
            reason: reason.to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        different_new_file_path_suggestion, new_file_diff_has_meaningful_added_content,
        pending_proposal_snapshot, prepare_agent_existing_full_file_targets,
        prepare_new_file_targets, validate_or_retry_proposal_with_request, PendingCheckState,
        PendingProposal, ProposePreviewMode, AGENT_FULL_FILE_UPDATE_MAX_BYTES,
    };
    use crate::status_snapshot::PENDING_STATUS_NONE;
    use origin_protocol::patch::DiffProposal;
    use origin_session::SessionWriter;
    use origin_tools::{ReadFile, Workspace};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn test_pending_proposal() -> PendingProposal {
        PendingProposal {
            diff: "--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
            requested_files: vec!["README.md".to_string()],
            proposed_files: vec!["README.md".to_string()],
            instruction: "fix typo".to_string(),
            provider: "lmstudio".to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
            model: "test-model".to_string(),
            check_state: PendingCheckState::NotChecked,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("origin-edit-workflow-{name}-{unique}"));
        path
    }

    fn test_session(root: &std::path::Path) -> SessionWriter {
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        SessionWriter::create_in(
            root.join("sessions"),
            cwd,
            "propose",
            UNIX_EPOCH + Duration::from_secs(1_777_944_250),
        )
        .unwrap()
    }

    fn new_file_target() -> Vec<ReadFile> {
        vec![ReadFile {
            relative_path: "docs/new.md".to_string(),
            contents: String::new(),
        }]
    }

    fn invalid_new_file_diff() -> &'static str {
        "--- /dev/null\n+++ b/docs/new.md\n@@ -0,0 +1,2 @@\n+one\n"
    }

    fn valid_new_file_diff() -> &'static str {
        "--- /dev/null\n+++ b/docs/new.md\n@@ -0,0 +1,2 @@\n+one\n+two\n"
    }

    #[test]
    fn pending_check_state_labels_are_stable() {
        assert_eq!(PendingCheckState::NotChecked.checked_label(), "no");
        assert_eq!(PendingCheckState::NotChecked.check_result(), None);
        assert_eq!(PendingCheckState::Passed.checked_label(), "yes");
        assert_eq!(PendingCheckState::Passed.check_result(), Some("ok"));
        let failed = PendingCheckState::Failed("git apply --check failed".to_string());
        assert_eq!(failed.checked_label(), "failed");
        assert_eq!(failed.check_result(), Some("git apply --check failed"));
    }

    #[test]
    fn pending_proposal_updates_check_state_from_result() {
        let mut pending = test_pending_proposal();
        pending.update_check_state(&Ok(()));
        assert_eq!(pending.check_state, PendingCheckState::Passed);

        pending.update_check_state(&Err("first line\nsecond line".to_string()));
        assert_eq!(
            pending.check_state,
            PendingCheckState::Failed("first line".to_string())
        );
    }

    #[test]
    fn pending_proposal_snapshot_preserves_metadata() {
        let mut pending = test_pending_proposal();
        pending.check_state = PendingCheckState::Passed;

        let snapshot = pending_proposal_snapshot(Some(&pending));
        assert_eq!(snapshot.checked.as_deref(), Some("yes"));
        assert_eq!(snapshot.check_result.as_deref(), Some("ok"));
        assert_eq!(snapshot.instruction.as_deref(), Some("fix typo"));
        assert_eq!(snapshot.requested_files, vec!["README.md"]);
        assert_eq!(snapshot.proposed_files, vec!["README.md"]);
        assert_eq!(snapshot.provider.as_deref(), Some("lmstudio"));
        assert_eq!(snapshot.model.as_deref(), Some("test-model"));
    }

    #[test]
    fn empty_pending_proposal_snapshot_is_none() {
        let snapshot = pending_proposal_snapshot(None);
        assert_eq!(snapshot.status, PENDING_STATUS_NONE);
        assert!(snapshot.instruction.is_none());
    }

    #[test]
    fn prepare_new_file_targets_accepts_safe_missing_file_with_existing_parent() {
        let root = temp_dir("new-file");
        fs::create_dir_all(root.join("docs")).unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let files = prepare_new_file_targets(&workspace, &["docs/new.md".to_string()]).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path, "docs/new.md");
        assert!(files[0].contents.is_empty());
    }

    #[test]
    fn prepare_new_file_targets_rejects_unsafe_targets() {
        let root = temp_dir("unsafe-new-file");
        fs::create_dir_all(root.join("docs")).unwrap();
        let workspace = Workspace::new(&root).unwrap();

        for path in [
            "/tmp/outside.md",
            "../outside.md",
            ".hidden.md",
            "docs/.env",
            "docs/api_key.md",
        ] {
            let err = prepare_new_file_targets(&workspace, &[path.to_string()]).unwrap_err();
            assert!(!err.is_empty());
        }
    }

    #[test]
    fn prepare_agent_existing_full_file_targets_accepts_single_small_existing_file() {
        let root = temp_dir("agent-existing-small");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "small\n").unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let files =
            prepare_agent_existing_full_file_targets(&workspace, &["README.md".to_string()])
                .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path, "README.md");
        assert_eq!(files[0].contents, "small\n");
    }

    #[test]
    fn prepare_agent_existing_full_file_targets_rejects_over_64kb_without_raw_diff_fallback() {
        let root = temp_dir("agent-existing-large");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("README.md"),
            "x".repeat(AGENT_FULL_FILE_UPDATE_MAX_BYTES + 1),
        )
        .unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let err = prepare_agent_existing_full_file_targets(&workspace, &["README.md".to_string()])
            .unwrap_err();

        assert!(err.contains("This file is too large for a full-file Agent update preview."));
        assert!(err.contains("narrower"));
        assert!(err.contains("No files changed."));
    }

    #[test]
    fn empty_new_file_diff_content_is_not_meaningful() {
        assert!(!new_file_diff_has_meaningful_added_content(""));
        assert!(!new_file_diff_has_meaningful_added_content(
            "--- /dev/null\n+++ b/docs/new.md\n@@ -0,0 +1,2 @@\n+\n+   \n"
        ));
        assert!(new_file_diff_has_meaningful_added_content(
            "--- /dev/null\n+++ b/docs/new.md\n@@ -0,0 +1 @@\n+real content\n"
        ));
    }

    #[test]
    fn empty_constructed_new_file_body_is_rejected_as_not_meaningful() {
        let diff = crate::commands::construct_new_file_unified_diff("docs/new.md", " \n\t");

        assert!(diff.is_empty());
        assert!(!new_file_diff_has_meaningful_added_content(&diff));
    }

    #[test]
    fn detects_different_new_file_path_suggestion() {
        let requested = [ReadFile {
            relative_path: "settings-summary.md".to_string(),
            contents: String::new(),
        }];
        let diff = "--- /dev/null\n+++ b/旅計画_統合完了版.md\n@@ -0,0 +1 @@\n+統合内容\n";

        assert_eq!(
            different_new_file_path_suggestion(diff, &requested),
            Some("旅計画_統合完了版.md".to_string())
        );
    }

    #[test]
    fn ignores_matching_new_file_path_as_suggestion() {
        let requested = [ReadFile {
            relative_path: "settings-summary.md".to_string(),
            contents: String::new(),
        }];
        let diff = "--- /dev/null\n+++ b/settings-summary.md\n@@ -0,0 +1 @@\n+content\n";

        assert_eq!(different_new_file_path_suggestion(diff, &requested), None);
    }

    #[test]
    fn agent_new_file_invalid_diff_retry_uses_same_confirmed_target_and_succeeds() {
        let root = temp_dir("agent-retry-success");
        let mut session = test_session(&root);
        let files = new_file_target();
        let mut attempts = 0;

        let (proposal, notice) = validate_or_retry_proposal_with_request(
            ProposePreviewMode::NewFile,
            true,
            &files,
            &mut session,
            DiffProposal::new(invalid_new_file_diff()),
            |error| {
                attempts += 1;
                assert!(error.contains("invalid unified diff"));
                assert!(error.contains("hunk header line count does not match hunk body"));
                Ok((
                    valid_new_file_diff().to_string(),
                    DiffProposal::new(valid_new_file_diff()),
                ))
            },
        )
        .expect("retry should succeed");

        assert_eq!(attempts, 1);
        assert_eq!(proposal.diff, valid_new_file_diff());
        assert!(notice.unwrap().contains("Retrying once"));
        assert_eq!(
            changed_files_from_retry_test(&proposal.diff),
            ["docs/new.md"]
        );
    }

    #[test]
    fn agent_new_file_invalid_diff_retry_failure_stores_no_pending_proposal() {
        let root = temp_dir("agent-retry-failure");
        let mut session = test_session(&root);
        let files = new_file_target();

        let err = validate_or_retry_proposal_with_request(
            ProposePreviewMode::NewFile,
            true,
            &files,
            &mut session,
            DiffProposal::new(invalid_new_file_diff()),
            |_error| {
                Ok((
                    invalid_new_file_diff().to_string(),
                    DiffProposal::new(invalid_new_file_diff()),
                ))
            },
        )
        .unwrap_err();

        assert!(err.error.contains("still returned an invalid diff"));
        assert!(err.error.contains("No pending proposal was stored"));
        assert!(err.error.contains("No files were changed"));
        assert!(err.retry_notice.unwrap().contains("Retrying once"));
    }

    fn changed_files_from_retry_test(diff: &str) -> Vec<String> {
        origin_tools::changed_files_from_diff(diff)
    }
}
