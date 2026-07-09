use crate::chat_workflow::{UserMemory, WorkspaceInstructions};
use crate::runtime::RuntimeState;

pub(crate) const PENDING_STATUS_NONE: &str = "none";
pub(crate) const PENDING_STATUS_PENDING: &str = "pending";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusSnapshot {
    pub(crate) mode: String,
    pub(crate) command: String,
    pub(crate) cwd: String,
    pub(crate) git_state: String,
    pub(crate) provider_id: String,
    pub(crate) provider_kind: String,
    pub(crate) base_url: String,
    pub(crate) auth_label: String,
    pub(crate) model_id: Option<String>,
    pub(crate) model_source: String,
    pub(crate) model_error: Option<String>,
    pub(crate) config_path: String,
    pub(crate) sessions_root: String,
    pub(crate) workspace_instructions: WorkspaceInstructionsSnapshot,
    pub(crate) user_memory: WorkspaceInstructionsSnapshot,
    /// TUI の Ask/Agent モードラベル(ASK / AGENT)。非 TUI 経路では None。
    pub(crate) interaction_mode: Option<String>,
    /// `/resume` で読み込んだ過去 session id(background only)。無ければ None。
    pub(crate) resumed_session: Option<String>,
    /// policy(P1)の要約。件数と警告のみで、パターン本文は持たない(P-5)。
    pub(crate) policy_workspace_rules: usize,
    pub(crate) policy_user_rules: usize,
    pub(crate) policy_warning: Option<String>,
    /// agent memory(N1)の要約(on/off と件数のみ)。
    pub(crate) agent_memory_enabled: bool,
    pub(crate) agent_memory_entries: usize,
    pub(crate) pending: PendingProposalSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceInstructionsSnapshot {
    pub(crate) status: String,
    pub(crate) bytes: Option<u64>,
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingProposalSnapshot {
    pub(crate) status: String,
    pub(crate) checked: Option<String>,
    pub(crate) check_result: Option<String>,
    pub(crate) instruction: Option<String>,
    pub(crate) requested_files: Vec<String>,
    pub(crate) proposed_files: Vec<String>,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
}

impl StatusSnapshot {
    pub(crate) fn from_runtime(runtime: &RuntimeState, pending: PendingProposalSnapshot) -> Self {
        Self::from_runtime_with_workspace_instructions(
            runtime,
            pending,
            &WorkspaceInstructions::None,
        )
    }

    pub(crate) fn from_runtime_with_workspace_instructions(
        runtime: &RuntimeState,
        pending: PendingProposalSnapshot,
        workspace_instructions: &WorkspaceInstructions,
    ) -> Self {
        Self::from_tui_context(
            runtime,
            pending,
            workspace_instructions,
            &UserMemory::None,
            "",
            None,
            false,
            0,
        )
    }

    /// TUI セッションの実状態(Ask/Agent モード・user memory・resumed session)を含む
    /// フル snapshot。interaction_mode_label が空文字列なら非 TUI 扱いで None にする。
    pub(crate) fn from_tui_context(
        runtime: &RuntimeState,
        pending: PendingProposalSnapshot,
        workspace_instructions: &WorkspaceInstructions,
        user_memory: &UserMemory,
        interaction_mode_label: &str,
        resumed_session: Option<String>,
        agent_memory_enabled: bool,
        agent_memory_entries: usize,
    ) -> Self {
        Self {
            mode: "interactive".to_string(),
            command: "origin".to_string(),
            cwd: runtime.cwd.clone(),
            git_state: runtime.git_state.clone(),
            provider_id: runtime.provider_id.clone(),
            provider_kind: runtime.provider_kind.clone(),
            base_url: runtime.base_url.clone(),
            auth_label: runtime.auth_label.clone(),
            model_id: runtime.model_id.clone(),
            model_source: runtime.model_source.clone(),
            model_error: runtime.model_error.clone(),
            config_path: runtime.config_path.clone(),
            sessions_root: runtime.sessions_root.clone(),
            workspace_instructions: WorkspaceInstructionsSnapshot::from_workspace_instructions(
                workspace_instructions,
            ),
            user_memory: WorkspaceInstructionsSnapshot::from_user_memory(user_memory),
            interaction_mode: if interaction_mode_label.is_empty() {
                None
            } else {
                Some(interaction_mode_label.to_string())
            },
            resumed_session,
            agent_memory_enabled,
            agent_memory_entries,
            policy_workspace_rules: runtime.policy.workspace_rules,
            policy_user_rules: runtime.policy.user_rules,
            policy_warning: runtime.policy.warnings.first().cloned(),
            pending,
        }
    }
}

impl WorkspaceInstructionsSnapshot {
    pub(crate) fn from_workspace_instructions(instructions: &WorkspaceInstructions) -> Self {
        match instructions {
            WorkspaceInstructions::None => Self {
                status: "none".to_string(),
                bytes: None,
                reason: None,
            },
            WorkspaceInstructions::Loaded { bytes, .. } => Self {
                status: "loaded".to_string(),
                bytes: Some(*bytes),
                reason: None,
            },
            WorkspaceInstructions::Skipped { bytes, reason, .. } => Self {
                status: "skipped".to_string(),
                bytes: *bytes,
                reason: Some(reason.clone()),
            },
        }
    }

    /// user memory(~/.origin/memory.md)も同じ loaded/skipped/none の3状態で表す。
    pub(crate) fn from_user_memory(memory: &UserMemory) -> Self {
        match memory {
            UserMemory::None => Self {
                status: "none".to_string(),
                bytes: None,
                reason: None,
            },
            UserMemory::Loaded { bytes, .. } => Self {
                status: "loaded".to_string(),
                bytes: Some(*bytes),
                reason: None,
            },
            UserMemory::Skipped { bytes, reason, .. } => Self {
                status: "skipped".to_string(),
                bytes: *bytes,
                reason: Some(reason.clone()),
            },
        }
    }
}

impl PendingProposalSnapshot {
    pub(crate) fn none() -> Self {
        Self {
            status: PENDING_STATUS_NONE.to_string(),
            checked: None,
            check_result: None,
            instruction: None,
            requested_files: Vec::new(),
            proposed_files: Vec::new(),
            provider: None,
            model: None,
        }
    }

    pub(crate) fn pending(input: PendingProposalSnapshotInput) -> Self {
        Self {
            status: PENDING_STATUS_PENDING.to_string(),
            checked: Some(input.checked),
            check_result: input.check_result,
            instruction: Some(input.instruction),
            requested_files: input.requested_files,
            proposed_files: input.proposed_files,
            provider: Some(input.provider),
            model: Some(input.model),
        }
    }
}

pub(crate) struct PendingProposalSnapshotInput {
    pub(crate) checked: String,
    pub(crate) check_result: Option<String>,
    pub(crate) instruction: String,
    pub(crate) requested_files: Vec<String>,
    pub(crate) proposed_files: Vec<String>,
    pub(crate) provider: String,
    pub(crate) model: String,
}

#[cfg(test)]
mod tests {
    use origin_protocol::provider::{ProviderAuth, ProviderId, ProviderKind};
    use origin_provider::openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleProvider};

    use crate::runtime::RuntimeState;

    use super::{
        PendingProposalSnapshot, PendingProposalSnapshotInput, StatusSnapshot, PENDING_STATUS_NONE,
        PENDING_STATUS_PENDING,
    };

    fn test_runtime() -> RuntimeState {
        RuntimeState {
            provider: OpenAiCompatibleProvider::new(OpenAiCompatibleConfig {
                id: ProviderId::new("lmstudio"),
                base_url: "http://localhost:1234/v1".to_string(),
                auth: ProviderAuth::None,
            }),
            provider_id: "lmstudio".to_string(),
            provider_kind: ProviderKind::OpenAiCompatible.as_str().to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
            auth_label: "none".to_string(),
            model_id: Some("qwen-test".to_string()),
            model_source: "config".to_string(),
            model_error: None,
            config_path: "/home/user/.origin/config.toml".to_string(),
            sessions_root: "/home/user/.origin/sessions".to_string(),
            cwd: "/repo".to_string(),
            git_state: "clean".to_string(),
            policy: origin_tools::policy::PolicySet::empty(),
        }
    }

    #[test]
    fn builds_status_snapshot_from_runtime() {
        let snapshot =
            StatusSnapshot::from_runtime(&test_runtime(), PendingProposalSnapshot::none());

        assert_eq!(snapshot.mode, "interactive");
        assert_eq!(snapshot.command, "origin");
        assert_eq!(snapshot.provider_id, "lmstudio");
        assert_eq!(snapshot.provider_kind, "openai-compatible");
        assert_eq!(snapshot.base_url, "http://localhost:1234/v1");
        assert_eq!(snapshot.auth_label, "none");
        assert_eq!(snapshot.model_id.as_deref(), Some("qwen-test"));
        assert_eq!(snapshot.model_source, "config");
        assert_eq!(snapshot.config_path, "/home/user/.origin/config.toml");
        assert_eq!(snapshot.sessions_root, "/home/user/.origin/sessions");
        assert_eq!(snapshot.cwd, "/repo");
        assert_eq!(snapshot.git_state, "clean");
        assert_eq!(snapshot.pending.status, PENDING_STATUS_NONE);
    }

    #[test]
    fn pending_snapshot_none_has_no_payload() {
        let pending = PendingProposalSnapshot::none();

        assert_eq!(pending.status, PENDING_STATUS_NONE);
        assert_eq!(pending.checked, None);
        assert_eq!(pending.check_result, None);
        assert!(pending.requested_files.is_empty());
        assert!(pending.proposed_files.is_empty());
    }

    #[test]
    fn pending_snapshot_carries_instruction_files_and_checked_state() {
        let pending = PendingProposalSnapshot::pending(PendingProposalSnapshotInput {
            checked: "no".to_string(),
            check_result: None,
            instruction: "fix typo".to_string(),
            requested_files: vec!["README.md".to_string()],
            proposed_files: vec!["README.md".to_string()],
            provider: "lmstudio".to_string(),
            model: "qwen-test".to_string(),
        });

        assert_eq!(pending.status, PENDING_STATUS_PENDING);
        assert_eq!(pending.checked.as_deref(), Some("no"));
        assert_eq!(pending.instruction.as_deref(), Some("fix typo"));
        assert_eq!(pending.requested_files, vec!["README.md"]);
        assert_eq!(pending.proposed_files, vec!["README.md"]);
        assert_eq!(pending.provider.as_deref(), Some("lmstudio"));
        assert_eq!(pending.model.as_deref(), Some("qwen-test"));
    }

    #[test]
    fn pending_snapshot_carries_failed_check_result() {
        let pending = PendingProposalSnapshot::pending(PendingProposalSnapshotInput {
            checked: "failed".to_string(),
            check_result: Some("git apply --check failed".to_string()),
            instruction: "fix typo".to_string(),
            requested_files: vec!["README.md".to_string()],
            proposed_files: vec!["README.md".to_string()],
            provider: "lmstudio".to_string(),
            model: "qwen-test".to_string(),
        });

        assert_eq!(pending.checked.as_deref(), Some("failed"));
        assert_eq!(
            pending.check_result.as_deref(),
            Some("git apply --check failed")
        );
    }

    #[test]
    fn snapshot_does_not_contain_secret_terms() {
        let snapshot =
            StatusSnapshot::from_runtime(&test_runtime(), PendingProposalSnapshot::none());
        let debug = format!("{snapshot:?}");

        assert!(!debug.contains("Authorization"));
        assert!(!debug.contains("Bearer"));
        assert!(!debug.contains("OPENAI_API_KEY"));
    }
}
