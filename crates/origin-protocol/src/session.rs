use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionStarted {
        session_id: String,
        created_at: String,
        cwd: String,
    },
    CommandStarted {
        command: String,
    },
    ProviderResolved {
        provider: String,
        base_url: String,
    },
    ModelResolved {
        model: String,
    },
    UserMessage {
        text: String,
    },
    ResumedFrom {
        source_session_id: String,
        source_path: String,
        context: String,
    },
    FileRead {
        path: String,
        bytes: u64,
        sha256: String,
    },
    FileContext {
        mode: String,
        path: String,
        status: String,
        bytes: Option<u64>,
        sha256: Option<String>,
        reason: Option<String>,
    },
    WorkspaceOverview {
        mode: String,
        root: String,
        workspace_kind: String,
        listed_entries: usize,
        included_files: usize,
        skipped_items: usize,
    },
    AgentPlannerAction {
        action: String,
        path: Option<String>,
    },
    ModelResponse {
        text: String,
    },
    DiffProposed {
        diff: String,
    },
    DiffValidationSucceeded {
        requested_files: Vec<String>,
        proposed_files: Vec<String>,
    },
    DiffValidationFailed {
        requested_files: Vec<String>,
        proposed_files: Vec<String>,
        reason: String,
    },
    DiffIntegritySucceeded {
        file_count: usize,
        hunk_count: usize,
    },
    DiffIntegrityFailed {
        reason: String,
    },
    DiffRetryRequested {
        reason: String,
    },
    DiffRetryStarted {
        attempt: u8,
    },
    DiffRetrySucceeded {
        attempt: u8,
    },
    DiffRetryFailed {
        attempt: u8,
        reason: String,
    },
    ApplyCheckRetryRequested {
        error: String,
    },
    ApplyCheckRetryStarted {
        attempt: u8,
    },
    ApplyCheckRetrySucceeded {
        attempt: u8,
    },
    ApplyCheckRetryFailed {
        attempt: u8,
        reason: String,
    },
    GitApplyCheckStarted,
    GitApplyCheckSucceeded,
    GitApplyCheckFailed {
        error: String,
    },
    ApprovalRequested,
    ApprovalDecision {
        decision: String,
    },
    PatchApplied {
        files: Vec<String>,
    },
    PatchRejected,
    CommandFailed {
        error: String,
    },
    SessionFinished {
        status: String,
    },
}
