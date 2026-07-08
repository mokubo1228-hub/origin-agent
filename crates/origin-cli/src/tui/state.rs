//! TUI セッション状態と Pending*/Tui* データモデル。
//! provider 呼び出し・preview・write 承認は同じ TuiState を共有するが、
//! 永続設定や session ログは runtime/session 側に残す。
use super::*;

/// active context に保持する最大イベント数(超過分は古い側から落とす)。
pub(super) const ACTIVE_CONTEXT_MAX_EVENTS: usize = 12;
/// active context 全体の文字数上限(provider へ渡す量を bounded に保つ)。
pub(super) const ACTIVE_CONTEXT_MAX_CHARS: usize = 3_000;
/// 1エントリあたりの文字数上限(長文は切り詰めてから積む)。
pub(super) const ACTIVE_CONTEXT_ENTRY_MAX_CHARS: usize = 600;
/// TUI の対話モード。Ask は読み取り中心、Agent は提案・書き込み経路を開く。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiInteractionMode {
    Ask,
    Agent,
}

impl TuiInteractionMode {
    /// status / 表示用の安定ラベル文字列。
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Ask => "ASK",
            Self::Agent => "AGENT",
        }
    }
}

/// TUI セッション中の可変状態を1つに集約するハブ。
/// run loop がこの単一の値を全 handler に &mut で渡すことで、入力処理・
/// provider 呼び出し・preview・write 承認が同じ状態を共有する。
/// ここに置くのは「セッション中だけ生きる」状態(入力・スクロール・
/// 進行中ワーカ・pending な提案/承認/Agent plan など)であり、永続化される
/// 設定や session ログはここではなく runtime / session 側が持つ。
pub(super) struct TuiState {
    pub(super) runtime: RuntimeState,
    pub(super) status: StatusSnapshot,
    pub(super) input: String,
    pub(super) input_cursor: usize,
    pub(super) suggestion_selection: usize,
    pub(super) chat_scroll_offset: usize,
    pub(super) messages: Vec<TuiMessage>,
    pub(super) loading: bool,
    pub(super) chat_in_flight: Option<TuiChatInFlight>,
    pub(super) agent_plan_in_flight: Option<TuiAgentPlanInFlight>,
    pub(super) next_agent_plan_request_id: u64,
    pub(super) pending_proposal: Option<PendingProposal>,
    pub(super) pending_apply: Option<TuiPendingApply>,
    pub(super) proposal_review: Option<TuiProposalReview>,
    pub(super) agent_generation: Option<TuiAgentGeneration>,
    pub(super) agent_propose_in_flight: Option<TuiAgentProposeInFlight>,
    pub(super) write_progress: Option<TuiWriteProgress>,
    pub(super) result_panel: Option<TuiResultPanel>,
    pub(super) model_route_picker: Option<ModelRoutePicker>,
    pub(super) resume_picker: Option<ResumePicker>,
    pub(super) interaction_mode: TuiInteractionMode,
    pub(super) active_context: TuiActiveConversationContext,
    pub(super) pending_agent_existing_edit_plan: Option<PendingAgentExistingEditPlan>,
    pub(super) pending_agent_new_file_plan: Option<PendingAgentNewFilePlan>,
    pub(super) pending_agent_destination_clarification:
        Option<PendingAgentDestinationClarification>,
    pub(super) pending_agent_rationale: Option<String>,
    pub(super) pending_agent_new_file_override: bool,
    pub(super) pending_agent_action_selection: usize,
    pub(super) show_advanced_actions: bool,
    pub(super) selection_buffer: String,
    pub(super) workspace_instructions: WorkspaceInstructions,
    pub(super) user_memory: UserMemory,
    pub(super) resumed_session: Option<TuiResumedSession>,
}

/// `/resume` が読み込んだ過去 session の bounded read-only background。
/// 状態は何も復元しない(proposal / diff / provider 状態は持たない)。
/// 以降の chat リクエストに resumed_session_background として添えられるだけの値。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiResumedSession {
    pub(super) session_id: String,
    pub(super) source_path: String,
    pub(super) context: String,
}

/// provider へ送る短期会話文脈。意図的に上限付き(イベント数・文字数)で、
/// 全文ではなく直近の往復だけを保持する。
/// secret は redact 済み・ファイル全文は載せない、を不変条件とする
/// (積む前に必ず bounded_active_context_text を通す)。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct TuiActiveConversationContext {
    pub(super) entries: Vec<TuiActiveConversationEntry>,
}

/// active context の1往復分。role(user/assistant/tui など)と bounded 済みテキストを持つ。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiActiveConversationEntry {
    pub(super) role: &'static str,
    pub(super) text: String,
}

/// 既存ファイル更新の pending plan。検証済み target と元指示を保持し、
/// review / 承認(y/n)を経るまで書き込みには使われない。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingAgentExistingEditPlan {
    pub(super) original_prompt: String,
    pub(super) target_path: String,
    pub(super) read_context_description: String,
}

/// 新規ファイル作成の pending plan。作成 target と、読み取り専用で添える参照ファイル
/// (context_files)を保持する。書き込みは承認後にだけ行う。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingAgentNewFilePlan {
    pub(super) original_prompt: String,
    pub(super) target_path: String,
    pub(super) read_context_description: String,
    pub(super) context_files: Vec<origin_tools::ReadFile>,
}

/// 行き先(対象ファイル)を安全に決められない時に開く clarification。元入力と質問を保持し、
/// 書き込みには進めない安全側の待機状態。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingAgentDestinationClarification {
    pub(super) original_prompt: String,
    pub(super) question: String,
}

/// proposal review パネルの状態。表示モード・由来・apply 可否・選択中アクションを束ねる。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiProposalReview {
    pub(super) mode: TuiProposalReviewMode,
    pub(super) origin: TuiProposalReviewOrigin,
    pub(super) apply_status: TuiProposalApplyStatus,
    pub(super) selected_action: usize,
}

/// Agent が proposal を生成中であることを表す進行状態。対象パス・種別・interrupt 可否を持つ。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiAgentGeneration {
    pub(super) target_path: String,
    pub(super) kind: TuiPreviewKind,
    pub(super) started_at: Instant,
    pub(super) interruptible: bool,
}

/// 進行中の propose worker への受信端だけを持つハンドル。
pub(super) struct TuiAgentProposeInFlight {
    pub(super) receiver: Receiver<TuiAgentProposeWorkerResult>,
}

/// 承認後の書き込み進行を表示するための状態。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiWriteProgress {
    pub(super) target_summary: String,
    pub(super) started_at: Instant,
}

/// 操作の終端結果を出すパネル(created/updated/cancelled/interrupted/blocked と対象ファイル)。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiResultPanel {
    pub(super) kind: TuiResultKind,
    pub(super) files: Vec<String>,
    pub(super) reason: Option<String>,
}

/// preview / generation が作成・更新・汎用のどれかを示す種別。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiPreviewKind {
    Create,
    Update,
    Generic,
}

/// 終端結果の種類。UI の result パネル文言と対応する。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiResultKind {
    Created,
    Updated,
    Cancelled,
    Interrupted,
    Blocked,
}

/// review パネルの入力モード。承認待ちか、修正指示の入力中か。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiProposalReviewMode {
    Review,
    RevisionInput,
}

/// proposal を今 apply できるか。Blocked の場合は理由(redact 済み)を保持する。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiProposalApplyStatus {
    Available,
    Blocked { reason: String },
}

/// この提案がどこから来たか(手動 /propose か、Agent の既存編集か新規作成か)。
/// 失敗時の復帰先(どの target に戻すか)と承認後の結果表示を出し分けるために
/// 保持する。承認の可否そのものには影響しない。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiProposalReviewOrigin {
    Manual,
    AgentExisting {
        target_path: String,
        original_prompt: String,
    },
    AgentNewFile {
        target_path: String,
        original_prompt: String,
        context_files: Vec<origin_tools::ReadFile>,
    },
}

impl TuiActiveConversationContext {
    /// user 発話を1件積む。
    pub(super) fn push_user(&mut self, text: &str) {
        self.push("user", text);
    }

    /// assistant 応答を1件積む。
    pub(super) fn push_assistant(&mut self, text: &str) {
        self.push("assistant", text);
    }

    /// TuiMessage を role に振り分けて active context へ積む。
    pub(super) fn push_tui_message(&mut self, message: &TuiMessage) {
        match message {
            TuiMessage::User(text) => self.push_user(text),
            TuiMessage::Assistant(text) => self.push_assistant(text),
            TuiMessage::Action(text) => self.push("tui", text),
            TuiMessage::Error(text) => self.push("tui_error", text),
            TuiMessage::Waiting(text) => self.push("tui", text),
        }
    }

    /// role 付きで1エントリ積む共通段。bounded_active_context_text を必ず通し、空なら
    /// 捨て、積んだ後に trim する(secret 除去・長さ制限を外さない不変条件の入口)。
    pub(super) fn push(&mut self, role: &'static str, text: &str) {
        let text = bounded_active_context_text(text);
        if text.is_empty() {
            return;
        }
        self.entries.push(TuiActiveConversationEntry { role, text });
        self.trim();
    }

    /// 蓄積した文脈を空にする。
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    /// 蓄積したエントリを provider へ渡す1テキストに連結する。空なら None。
    pub(super) fn provider_context(&self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        let mut output = String::new();
        for entry in &self.entries {
            output.push_str(entry.role);
            output.push_str(": ");
            output.push_str(&entry.text);
            output.push('\n');
        }
        let output = output.trim().to_string();
        if output.is_empty() {
            None
        } else {
            Some(output)
        }
    }

    /// イベント数・総文字数の上限を超えた分を古い側から落とす(bounded を維持する)。
    pub(super) fn trim(&mut self) {
        while self.entries.len() > ACTIVE_CONTEXT_MAX_EVENTS {
            self.entries.remove(0);
        }

        while self.total_chars() > ACTIVE_CONTEXT_MAX_CHARS && self.entries.len() > 1 {
            self.entries.remove(0);
        }
    }

    /// 現在の総文字数(各エントリの role + text + 区切り)を概算する内部ヘルパ。
    fn total_chars(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.role.chars().count() + entry.text.chars().count() + 3)
            .sum()
    }
}

/// active context へ積む前に通す安全フィルタ。
/// 目的は「provider に渡る文字列から secret を除き、長さを抑える」こと。
/// redact_sensitive_text による秘匿情報除去はここが最後の砦なので外さない。
pub(super) fn bounded_active_context_text(text: &str) -> String {
    let redacted = redact_sensitive_text(text);
    let mut compact = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > ACTIVE_CONTEXT_ENTRY_MAX_CHARS {
        compact = compact
            .chars()
            .take(ACTIVE_CONTEXT_ENTRY_MAX_CHARS)
            .collect::<String>();
        compact.push_str("...");
    }
    compact
}

/// 進行中の chat worker のハンドル。受信端と、初回警告済みフラグ・開始時刻を持つ。
pub(super) struct TuiChatInFlight {
    pub(super) receiver: Receiver<TuiChatWorkerResult>,
    pub(super) warning_shown: bool,
    pub(super) started_at: Instant,
}

/// chat worker が返す結果。更新後 runtime と chat 結果を同梱し、main state に取り込ませる。
pub(super) struct TuiChatWorkerResult {
    pub(super) runtime: RuntimeState,
    pub(super) result: Result<ChatWorkflowResult, String>,
}

/// 進行中の Agent plan worker のハンドル。request_id を持ち、poll 側が late result を
/// 識別して捨てるのに使う(interrupt/再送後に古い結果を適用しない)。
pub(super) struct TuiAgentPlanInFlight {
    pub(super) request_id: u64,
    pub(super) receiver: Receiver<TuiAgentPlanWorkerResult>,
    pub(super) input: String,
    pub(super) active_context: Option<String>,
    pub(super) warning_shown: bool,
    pub(super) started_at: Instant,
}

/// Agent plan worker が返す結果。どの要求の結果かを示す request_id を同梱する。
pub(super) struct TuiAgentPlanWorkerResult {
    pub(super) request_id: u64,
    pub(super) runtime: RuntimeState,
    pub(super) result: Result<AgentPlannerAction, String>,
}

/// transcript / active context に流れる1メッセージ。役割ごとに表示と文脈の扱いが変わる。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiMessage {
    User(String),
    Assistant(String),
    Action(String),
    Waiting(String),
    Error(String),
}

/// run loop が処理する送信の単位。chat / Agent plan / command / apply 判断 / 終了に分かれる。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiSubmit {
    Chat(TuiChatSubmit),
    AgentPlan(TuiAgentPlanSubmit),
    Command(TuiCommand),
    ApplyDecision(String),
    Exit,
}

/// chat 送信1件のペイロード。prompt と mode に加え、添える read-only file context を束ねる。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiChatSubmit {
    pub(super) prompt: String,
    pub(super) active_context: Option<String>,
    pub(super) prompt_mode: ChatPromptMode,
    pub(super) file_contexts: Vec<ChatFileContext>,
    pub(super) file_context_records: Vec<ChatFileContextRecord>,
    pub(super) workspace_overview: Option<ChatWorkspaceOverviewContext>,
}

/// Agent planner への入力。自然文と、参照する active context、
/// および ORIGIN.md / user memory / resumed session を束ねた inert read-only background。
/// background は action/path 判断の文脈にだけ使われ、permission・validation・
/// approval gate には一切影響しない(prompt 内でもその旨を明示する)。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TuiAgentPlanSubmit {
    pub(super) input: String,
    pub(super) active_context: Option<String>,
    pub(super) background: Option<String>,
}

/// slash command を表す内部表現。handle_tui_command がこれを実行に落とす。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiCommand {
    Help,
    Status,
    Mode,
    ModeSet(TuiInteractionMode),
    Model,
    ModelList,
    ModelSet(String),
    Provider,
    ProviderList,
    ProviderSet(String),
    Sessions,
    ResumePicker,
    Resume {
        target: Option<String>,
    },
    Propose {
        path: String,
        instruction: String,
    },
    AgentPropose {
        path: String,
        instruction: String,
    },
    AgentProposeNewFile {
        path: String,
        instruction: String,
        context_files: Vec<origin_tools::ReadFile>,
    },
    Diff,
    Check,
    Apply,
    Clear,
    Error(String),
    Unknown(String),
}
