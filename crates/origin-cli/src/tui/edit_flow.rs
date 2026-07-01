//! TUI の proposal preview / check / apply 連携。
//! 書き込みは Proposal review の permission gate 後に限定し、Agent preview 失敗時は
//! no-write の clarification へ戻す。

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::edit_workflow::{
    run_propose_preview, PendingProposal, ProposePreviewError, ProposePreviewMode,
    ProposePreviewRequest, ProposePreviewResult,
};
use crate::runtime::RuntimeState;
use origin_protocol::session::SessionEvent;
use origin_session::{redact_sensitive_text, SessionWriter};
use origin_tools::{
    changed_files_from_diff, discover_git_repo, git_apply, git_apply_check,
    parse_approval_decision, validate_diff_integrity, validate_diff_targets_allowing_new_files,
    ApprovalDecision, Workspace,
};

use super::{
    pin_chat_to_latest, refresh_status, unsafe_agent_edit_target_path_reason, TuiMessage,
    TuiProposalApplyStatus, TuiProposalReview, TuiProposalReviewMode, TuiProposalReviewOrigin,
    TuiResultKind, TuiResultPanel, TuiState,
};

/// non-git workspace で apply を塞ぐ時の固定理由文。git repo でないと git-backed apply は使えない。
const NON_GIT_APPLY_BLOCKED_REASON: &str = "this workspace is not a Git repository";
/// non-git apply で許すのは安全な単一 new-file proposal のみ、という制約の説明文。
const NON_GIT_NEW_FILE_DETAIL: &str =
    "Non-git apply currently supports safe single new-file proposals only.";

/// 承認待ちの apply を1件だけ保持する箱。git apply --check を通した pending と、その
/// session を抱えて y/yes を待つ。承認されるまでファイルには触れない(安全境界)。
pub(super) struct TuiPendingApply {
    session: SessionWriter,
    session_path: String,
    pending: PendingProposal,
}

/// Agent が preview 生成を依頼する単位。既存ファイル更新か新規作成かで分け、
/// どちらも単一ターゲット。新規は read-only の参照 context を一緒に運ぶ。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiAgentProposeRequest {
    Existing {
        path: String,
        instruction: String,
    },
    NewFile {
        path: String,
        instruction: String,
        context_files: Vec<origin_tools::ReadFile>,
    },
}

/// worker で生成した preview 結果と、その由来(review origin)を main へ返す束。
/// origin は後で review panel をどの Agent 文脈で開くか・失敗時にどこへ戻すかを決める。
pub(super) struct TuiAgentProposeWorkerResult {
    pub(super) result: Result<ProposePreviewResult, ProposePreviewError>,
    pub(super) review_origin: TuiProposalReviewOrigin,
}

/// non-git workspace での単一 new-file 書き込みを、検証済みの target / 絶対パス / 本文として
/// 確定したもの。これが作れた時点で「安全に書ける1ファイル」が決まっている。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NonGitNewFileApply {
    pub target_path: String,
    pub absolute_path: PathBuf,
    pub body: String,
}

/// 手動 `/propose` の入口。retry なし・Manual origin で既存ファイル proposal を生成する。
/// preview を作って review を開く所までで、書き込みはしない。
pub(super) fn run_tui_propose(state: &mut TuiState, path: String, instruction: String) {
    run_tui_propose_with_retry(
        state,
        path,
        instruction,
        false,
        TuiProposalReviewOrigin::Manual,
    );
}

/// Agent の preview 生成リクエストを mode 別に振り分ける。
/// 既存ファイルは model に raw diff を書かせず「更新後の全文」を要求し、origin 側で
/// unified diff を構築する(AgentExistingFullFile)。新規は NewFile mode。どちらも
/// 単一ターゲット + 1回だけの retry。
/// (全文を要求する際の 64KiB 上限と local diff 構築の実体は edit_workflow.rs 側。
///  なぜ全文かは Phase 25.1 設計参照:壊れた model 製 diff への依存を断つため。)
pub(super) fn run_tui_agent_propose_request(
    runtime: RuntimeState,
    request: TuiAgentProposeRequest,
) -> TuiAgentProposeWorkerResult {
    match request {
        TuiAgentProposeRequest::Existing { path, instruction } => {
            let origin = TuiProposalReviewOrigin::AgentExisting {
                target_path: path.clone(),
                original_prompt: instruction.clone(),
            };
            let paths = vec![path];
            let result = run_propose_preview(
                ProposePreviewRequest {
                    runtime: &runtime,
                    paths: &paths,
                    context_files: &[],
                    instruction: &instruction,
                    mode: ProposePreviewMode::AgentExistingFullFile,
                    retry_invalid: true,
                },
                |_| {},
            );
            TuiAgentProposeWorkerResult {
                result,
                review_origin: origin,
            }
        }
        TuiAgentProposeRequest::NewFile {
            path,
            instruction,
            context_files,
        } => {
            let origin = TuiProposalReviewOrigin::AgentNewFile {
                target_path: path.clone(),
                original_prompt: instruction.clone(),
                context_files: context_files.clone(),
            };
            let paths = vec![path];
            let result = run_propose_preview(
                ProposePreviewRequest {
                    runtime: &runtime,
                    paths: &paths,
                    context_files: &context_files,
                    instruction: &instruction,
                    mode: ProposePreviewMode::NewFile,
                    retry_invalid: true,
                },
                |_| {},
            );
            TuiAgentProposeWorkerResult {
                result,
                review_origin: origin,
            }
        }
    }
}

/// worker が作った Agent preview 結果を、main 側の状態反映(handle_tui_propose_result)へ
/// 渡すだけの橋渡し。
pub(super) fn finish_tui_agent_propose_result(
    state: &mut TuiState,
    result: TuiAgentProposeWorkerResult,
) {
    handle_tui_propose_result(state, result.result, result.review_origin);
}

/// 既存ファイル proposal の共通生成段。preview を作り review を開くまでで書き込みはしない。
/// retry_invalid と review_origin だけが呼び元で変わる(手動 vs Agent 由来)。
fn run_tui_propose_with_retry(
    state: &mut TuiState,
    path: String,
    instruction: String,
    retry_invalid: bool,
    review_origin: TuiProposalReviewOrigin,
) {
    let paths = vec![path];
    let result = run_propose_preview(
        ProposePreviewRequest {
            runtime: &state.runtime,
            paths: &paths,
            context_files: &[],
            instruction: &instruction,
            mode: ProposePreviewMode::ExistingFiles,
            retry_invalid,
        },
        |_| {},
    );

    handle_tui_propose_result(state, result, review_origin);
}

/// preview 生成の成否を state へ反映する分岐点。成功・部分成功なら pending proposal と
/// review panel を立て、提案が一切作れない完全失敗(かつ Agent 由来)なら no-write の
/// clarification に落とす。どの経路でもこの時点ではまだファイルを書かない。
fn handle_tui_propose_result(
    state: &mut TuiState,
    result: Result<
        crate::edit_workflow::ProposePreviewResult,
        crate::edit_workflow::ProposePreviewError,
    >,
    review_origin: TuiProposalReviewOrigin,
) {
    match result {
        Ok(result) => {
            state.agent_generation = None;
            if let Some(notice) = &result.retry_notice {
                let message = TuiMessage::Action(notice.clone());
                state.active_context.push_tui_message(&message);
                state.messages.push(message);
            }
            state.pending_proposal = result.pending.clone();
            set_proposal_review_for_pending(state, &result.pending, review_origin.clone());
            refresh_status(state);
            let message = TuiMessage::Action(format_tui_propose_result(&result));
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
        }
        Err(error) => {
            state.agent_generation = None;
            if let Some(notice) = &error.retry_notice {
                let message = TuiMessage::Action(notice.clone());
                state.active_context.push_tui_message(&message);
                state.messages.push(message);
            }
            if let Some(result) = error.partial {
                state.pending_proposal = result.pending.clone();
                set_proposal_review_for_pending(state, &result.pending, review_origin);
                refresh_status(state);
                let message = TuiMessage::Action(format_tui_propose_result(&result));
                state.active_context.push_tui_message(&message);
                state.messages.push(message);
            } else {
                state.pending_proposal = None;
                state.proposal_review = None;
                open_agent_preview_failure_clarification(state, &review_origin, &error.error);
                refresh_status(state);
            }
            let message = TuiMessage::Error(format_tui_propose_error(&error.error));
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
            if !error.session_path.is_empty() {
                state.messages.push(TuiMessage::Action(format!(
                    "session: {}",
                    error.session_path
                )));
            }
        }
    }
}

/// preview が提案を作れずに失敗した時の復帰経路。
/// dead-end にせず「指示を狭める / 別の target / cancel」の clarification に落とす。
/// 失敗時もファイルは一切書いていないこと(No files were changed)を明示し、
/// provider のエラーは redact してから rationale に載せる。
fn open_agent_preview_failure_clarification(
    state: &mut TuiState,
    review_origin: &TuiProposalReviewOrigin,
    error: &str,
) {
    let original_prompt = match review_origin {
        TuiProposalReviewOrigin::AgentExisting {
            original_prompt, ..
        }
        | TuiProposalReviewOrigin::AgentNewFile {
            original_prompt, ..
        } => original_prompt.clone(),
        TuiProposalReviewOrigin::Manual => return,
    };
    state.pending_agent_existing_edit_plan = None;
    state.pending_agent_new_file_plan = None;
    state.pending_agent_destination_clarification = Some(super::PendingAgentDestinationClarification {
        original_prompt,
        question: "The preview failed before a proposal could be created. Narrow the instruction, choose a smaller or different target, or cancel.".to_string(),
    });
    state.pending_agent_rationale = Some(redact_sensitive_text(error));
    state.pending_agent_new_file_override = false;
    state.pending_agent_action_selection = 0;
    super::reset_selection_buffer(state);
    let message = TuiMessage::Action(
        "Next: narrow the instruction, choose another target, or cancel. No files were changed."
            .to_string(),
    );
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
}

/// pending proposal の有無に応じて review panel を立てる/畳む。pending があれば
/// Review mode の permission panel を開き(apply 可否も判定して持たせる)、無ければ review を消す。
fn set_proposal_review_for_pending(
    state: &mut TuiState,
    pending: &Option<PendingProposal>,
    origin: TuiProposalReviewOrigin,
) {
    if let Some(pending) = pending {
        state.result_panel = None;
        state.write_progress = None;
        state.proposal_review = Some(TuiProposalReview {
            mode: TuiProposalReviewMode::Review,
            origin,
            apply_status: current_apply_status(pending),
            selected_action: 0,
        });
        super::reset_advanced_reveal(state);
        super::reset_selection_buffer(state);
    } else {
        state.proposal_review = None;
        super::reset_advanced_reveal(state);
    }
}

/// この proposal を今 apply できるかの状態判定。git repo なら Available、git が無くても
/// 安全な単一 new-file に解決できれば Available、それ以外は Blocked。判定だけで書き込みはしない。
fn current_apply_status(pending: &PendingProposal) -> TuiProposalApplyStatus {
    match discover_git_repo() {
        Ok(_) => TuiProposalApplyStatus::Available,
        // git repo が無くても、安全な単一 new-file として準備できるなら apply 可能とみなす。
        Err(error) if is_git_not_repository_error(&error.to_string()) => {
            match Workspace::from_current_dir()
                .ok()
                .and_then(|workspace| prepare_non_git_new_file_apply(&workspace, pending).ok())
            {
                Some(_) => TuiProposalApplyStatus::Available,
                None => non_git_apply_blocked_status(),
            }
        }
        Err(error) => TuiProposalApplyStatus::Blocked {
            reason: format!("git-backed apply is unavailable: {error}"),
        },
    }
}

/// pending proposal の diff を表示するだけの read-only コマンド。無ければエラー文を出す。
pub(super) fn run_tui_diff(state: &mut TuiState) {
    match state.pending_proposal.as_ref() {
        Some(pending) => state
            .messages
            .push(TuiMessage::Action(format_tui_pending_diff(pending))),
        None => state
            .messages
            .push(TuiMessage::Error(format_tui_no_pending_proposal())),
    }
}

/// `/check` の入口:pending proposal に対して git apply --check のみを走らせる。
/// 検証専用で書き込みはせず、成否を message に出す(安全境界)。
pub(super) fn run_tui_check(state: &mut TuiState) {
    let result = match state.pending_proposal.as_mut() {
        Some(pending) => run_tui_check_pending(pending),
        None => {
            state
                .messages
                .push(TuiMessage::Error(format_tui_no_pending_proposal()));
            return;
        }
    };

    match result {
        Ok(message) => state.messages.push(TuiMessage::Action(message)),
        Err(message) => {
            mark_review_blocked_if_git_unavailable(state, &message);
            state.messages.push(TuiMessage::Error(message));
        }
    }
    refresh_status(state);
    pin_chat_to_latest(state);
}

/// 手動 `/apply` の入口。git apply --check を再実行し、通れば y/yes 承認待ち(pending_apply)に
/// 入る。ここではまだ書き込まない。実際の apply は handle_tui_apply_decision が y/yes の時だけ行う。
pub(super) fn run_tui_apply(state: &mut TuiState) {
    if state.pending_apply.is_some() {
        state.write_progress = None;
        state.messages.push(TuiMessage::Error(
            "Approval is already pending.\nType y/yes to apply or anything else to reject."
                .to_string(),
        ));
        return;
    }

    let result = match state.pending_proposal.as_mut() {
        Some(pending) => prepare_tui_apply(pending),
        None => {
            state
                .messages
                .push(TuiMessage::Error(format_tui_no_pending_proposal()));
            return;
        }
    };

    match result {
        Ok(prepared) => {
            state
                .messages
                .push(TuiMessage::Action(format_tui_apply_approval_prompt(
                    &prepared.pending,
                    &prepared.session_path,
                )));
            state.pending_apply = Some(prepared);
        }
        Err(message) => {
            mark_review_blocked_if_git_unavailable(state, &message);
            state.messages.push(TuiMessage::Error(message));
        }
    }
    refresh_status(state);
    pin_chat_to_latest(state);
}

/// Proposal review の permission gate(y / Enter 承認)から呼ばれる apply。これは write/create/
/// update の許可後にだけ到達する書き込み経路。git なら prepare_tui_apply で git apply --check を
/// 再実行してから finish_tui_apply_approved で apply し、non-git なら安全な単一 new-file のみ書く。
/// 失敗時は proposal と review を保持したまま blocked にして、書き込みは行わない。
pub(super) fn run_tui_proposal_review_apply(state: &mut TuiState) {
    if state.pending_apply.is_some() {
        state.messages.push(TuiMessage::Error(
            "Approval is already pending.\nType y/yes to apply or anything else to reject."
                .to_string(),
        ));
        return;
    }

    if state.pending_proposal.is_none() {
        state
            .messages
            .push(TuiMessage::Error(format_tui_no_pending_proposal()));
        state.proposal_review = None;
        super::reset_advanced_reveal(state);
        return;
    }
    if handle_non_git_proposal_review_apply(state) {
        refresh_status(state);
        pin_chat_to_latest(state);
        return;
    }

    let result = match state.pending_proposal.as_mut() {
        Some(pending) => prepare_tui_apply(pending),
        None => {
            state
                .messages
                .push(TuiMessage::Error(format_tui_no_pending_proposal()));
            state.proposal_review = None;
            super::reset_advanced_reveal(state);
            return;
        }
    };

    match result {
        Ok(mut prepared) => match finish_tui_apply_approved(&mut prepared) {
            Ok(message) => {
                state.result_panel = Some(success_result_panel_for_pending(&prepared.pending));
                state.pending_proposal = None;
                state.proposal_review = None;
                super::reset_advanced_reveal(state);
                state.write_progress = None;
                state.messages.push(TuiMessage::Action(message));
            }
            Err(message) => {
                state.write_progress = None;
                state.result_panel = Some(blocked_result_panel_from_message(&message));
                state.pending_proposal = Some(prepared.pending);
                state.proposal_review = Some(TuiProposalReview {
                    mode: TuiProposalReviewMode::Review,
                    origin: state
                        .proposal_review
                        .as_ref()
                        .map(|review| review.origin.clone())
                        .unwrap_or(TuiProposalReviewOrigin::Manual),
                    apply_status: state
                        .proposal_review
                        .as_ref()
                        .map(|review| review.apply_status.clone())
                        .unwrap_or(TuiProposalApplyStatus::Available),
                    selected_action: 0,
                });
                mark_review_blocked_if_git_unavailable(state, &message);
                state.messages.push(TuiMessage::Error(message));
            }
        },
        Err(message) => {
            state.write_progress = None;
            state.result_panel = Some(blocked_result_panel_from_message(&message));
            mark_review_blocked_if_git_unavailable(state, &message);
            state.messages.push(TuiMessage::Error(message));
        }
    }
    refresh_status(state);
    pin_chat_to_latest(state);
}

/// non-git workspace 用の apply 分岐。git repo が無い場合だけ作動し、安全な単一 new-file を
/// root 直下へ書き込む。処理したら true を返して呼び元の git 経路をスキップさせる。
/// git repo がある時は false を返し、通常の git apply 経路に委ねる。
fn handle_non_git_proposal_review_apply(state: &mut TuiState) -> bool {
    let Err(git_error) = discover_git_repo() else {
        return false;
    };
    if !is_git_not_repository_error(&git_error.to_string()) {
        return false;
    }

    let pending = match state.pending_proposal.as_ref() {
        Some(pending) => pending.clone(),
        None => return false,
    };

    let workspace = match Workspace::from_current_dir() {
        Ok(workspace) => workspace,
        Err(error) => {
            set_non_git_apply_blocked(state);
            state.write_progress = None;
            state.result_panel = Some(TuiResultPanel {
                kind: TuiResultKind::Blocked,
                files: Vec::new(),
                reason: Some(format!("Could not resolve current workspace: {error}")),
            });
            state
                .messages
                .push(TuiMessage::Error(format_non_git_apply_blocked(Some(
                    &format!("Could not resolve current workspace: {error}"),
                ))));
            return true;
        }
    };

    match apply_non_git_new_file_at_root(workspace.root(), &pending) {
        Ok(message) => {
            state.result_panel = Some(TuiResultPanel {
                kind: TuiResultKind::Created,
                files: changed_files_from_diff(&pending.diff),
                reason: None,
            });
            state.pending_proposal = None;
            state.proposal_review = None;
            super::reset_advanced_reveal(state);
            state.write_progress = None;
            state.messages.push(TuiMessage::Action(message));
        }
        Err(reason) => {
            state.write_progress = None;
            state.result_panel = Some(TuiResultPanel {
                kind: TuiResultKind::Blocked,
                files: Vec::new(),
                reason: Some(reason.clone()),
            });
            set_non_git_apply_blocked(state);
            state
                .messages
                .push(TuiMessage::Error(format_non_git_apply_blocked(Some(
                    &reason,
                ))));
        }
    }
    true
}

/// review panel の apply 状態を non-git blocked に落とすヘルパ。
fn set_non_git_apply_blocked(state: &mut TuiState) {
    if let Some(review) = state.proposal_review.as_mut() {
        review.apply_status = non_git_apply_blocked_status();
    }
}

/// apply 成功後の result panel を作る。diff が `--- /dev/null` を含むかで Created / Updated を判定する。
fn success_result_panel_for_pending(pending: &PendingProposal) -> TuiResultPanel {
    let kind = if pending
        .diff
        .lines()
        .any(|line| line.trim() == "--- /dev/null")
    {
        TuiResultKind::Created
    } else {
        TuiResultKind::Updated
    };
    TuiResultPanel {
        kind,
        files: changed_files_from_diff(&pending.diff),
        reason: None,
    }
}

/// 失敗理由から Blocked の result panel を作る。理由は redact してから載せる。
fn blocked_result_panel_from_message(message: &str) -> TuiResultPanel {
    TuiResultPanel {
        kind: TuiResultKind::Blocked,
        files: Vec::new(),
        reason: Some(redact_sensitive_text(message)),
    }
}

/// non-git で apply を塞いだ status 値を作る定型ヘルパ。
fn non_git_apply_blocked_status() -> TuiProposalApplyStatus {
    TuiProposalApplyStatus::Blocked {
        reason: NON_GIT_APPLY_BLOCKED_REASON.to_string(),
    }
}

/// workspace root を基準に non-git new-file apply を準備する公開ラッパ。実体は _at_root 版。
pub(super) fn prepare_non_git_new_file_apply(
    workspace: &Workspace,
    pending: &PendingProposal,
) -> Result<NonGitNewFileApply, String> {
    prepare_non_git_new_file_apply_at_root(workspace.root(), pending)
}

/// non-git new-file 書き込みの前段検証を全部通す関門。単一 new-file・workspace 内の安全な
/// 相対パス・diff target 整合・diff 整合性・本文非空を確認し、検証済みの NonGitNewFileApply を返す。
/// ここを通らない限り non-git 書き込みは起きない(安全境界)。
pub(super) fn prepare_non_git_new_file_apply_at_root(
    root: &Path,
    pending: &PendingProposal,
) -> Result<NonGitNewFileApply, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("outside current directory: {error}"))?;
    let target_path = single_new_file_target(pending)?;
    let absolute_path = validate_non_git_new_file_target(&root, &target_path)?;
    validate_diff_targets_allowing_new_files(&pending.diff, &pending.requested_files)
        .map_err(|error| error.reason)?;
    validate_diff_integrity(&pending.diff).map_err(|error| error.reason)?;
    let body = extract_new_file_body_from_diff(&pending.diff, &target_path)?;
    if body.trim().is_empty() {
        return Err("content is empty".to_string());
    }

    Ok(NonGitNewFileApply {
        target_path,
        absolute_path,
        body,
    })
}

/// non-git の単一 new-file を実際に書き込む唯一点。prepare_*_at_root の検証を通った
/// body だけを fs::write する。検証で弾かれた場合はここに来ず、書き込みは起きない。
pub(super) fn apply_non_git_new_file_at_root(
    root: &Path,
    pending: &PendingProposal,
) -> Result<String, String> {
    let plan = prepare_non_git_new_file_apply_at_root(root, pending)?;
    fs::write(&plan.absolute_path, &plan.body)
        .map_err(|error| format!("Could not create `{}`: {error}", plan.target_path))?;
    Ok(format_non_git_new_file_apply_success(&plan.target_path))
}

/// pending が「ちょうど1つの new-file」で requested と proposed が一致することを確認し target を返す。
/// 複数ファイルや requested/proposed の不一致は拒否する(単一安全 new-file 原則)。
fn single_new_file_target(pending: &PendingProposal) -> Result<String, String> {
    if pending.requested_files.len() != 1 || pending.proposed_files.len() != 1 {
        return Err("proposal must create exactly one new file".to_string());
    }
    let requested = pending.requested_files[0].clone();
    let proposed = pending.proposed_files[0].clone();
    if requested != proposed {
        return Err("proposal target does not match the requested file".to_string());
    }
    Ok(requested)
}

/// non-git new-file の target パスを検証して絶対パスに解決する。空・絶対パス・親ディレクトリ
/// 遡上(`..`)・危険パス・親不在・既存衝突を全て弾き、解決後の親が root 配下にあることを保証する。
/// workspace root の外には決して出さない(安全境界)。
fn validate_non_git_new_file_target(root: &Path, target: &str) -> Result<PathBuf, String> {
    let path = Path::new(target);
    if target.trim().is_empty() {
        return Err("invalid path".to_string());
    }
    if path.is_absolute() {
        return Err("outside current directory".to_string());
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("outside current directory".to_string());
    }
    if let Some(reason) = unsafe_agent_edit_target_path_reason(target) {
        return Err(reason);
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let parent_abs = parent.map_or_else(|| root.to_path_buf(), |parent| root.join(parent));
    let parent_abs = parent_abs
        .canonicalize()
        .map_err(|_| "parent directory missing".to_string())?;
    if !parent_abs.starts_with(root) {
        return Err("outside current directory".to_string());
    }
    if !parent_abs.is_dir() {
        return Err("parent directory missing".to_string());
    }
    let file_name = path.file_name().ok_or_else(|| "invalid path".to_string())?;
    let absolute_path = parent_abs.join(file_name);
    if absolute_path.exists() {
        return Err("path already exists".to_string());
    }
    Ok(absolute_path)
}

/// new-file 用 `/dev/null` unified diff から追加行だけを取り出して本文を組む。
/// `---` が `/dev/null`、`+++` が target、`@@` hunk の順を厳密に確認し、削除行や文脈行が
/// 混ざる「new-file 以外の diff」は拒否する(non-git 書き込みは純粋な新規作成に限る)。
fn extract_new_file_body_from_diff(diff: &str, target: &str) -> Result<String, String> {
    let mut saw_old_dev_null = false;
    let mut saw_new_target = false;
    let mut in_hunk = false;
    let mut added_lines = Vec::new();

    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("--- ") {
            if saw_old_dev_null {
                return Err("proposal must create exactly one new file".to_string());
            }
            if path.trim() != "/dev/null" {
                return Err(NON_GIT_NEW_FILE_DETAIL.to_string());
            }
            saw_old_dev_null = true;
            continue;
        }

        if let Some(path) = line.strip_prefix("+++ ") {
            let path = path.trim().strip_prefix("b/").unwrap_or(path.trim());
            if !saw_old_dev_null || path != target {
                return Err("proposal target does not match the requested file".to_string());
            }
            saw_new_target = true;
            continue;
        }

        if line.starts_with("@@ ") {
            if !saw_new_target {
                return Err("invalid diff".to_string());
            }
            in_hunk = true;
            continue;
        }

        if !in_hunk {
            continue;
        }

        if let Some(content) = line.strip_prefix('+') {
            added_lines.push(content.to_string());
        } else if line.starts_with("\\ No newline at end of file") {
            continue;
        } else if line.starts_with('-') || line.starts_with(' ') {
            // 追加行以外(削除行・文脈行)が現れたら純粋な new-file ではないので拒否。
            return Err(NON_GIT_NEW_FILE_DETAIL.to_string());
        }
    }

    if !saw_old_dev_null || !saw_new_target || !in_hunk {
        return Err("invalid diff".to_string());
    }
    let mut body = added_lines.join("\n");
    if !added_lines.is_empty() {
        body.push('\n');
    }
    Ok(body)
}

/// non-git new-file 作成成功の表示文を組む。target は redact 済み。
fn format_non_git_new_file_apply_success(target_path: &str) -> String {
    format!(
        "Created\n\nFile:\n- {}\n\nNo other files were changed.\n\nReady.",
        redact_sensitive_text(target_path)
    )
}

/// non-git apply が塞がれた時の表示文を組む。No files were changed を明示し、詳細があれば
/// redact して付ける。
fn format_non_git_apply_blocked(detail: Option<&str>) -> String {
    let mut output = String::new();
    output.push_str("Blocked\n\nReason:\n- this workspace is not a Git repository\n");
    output.push_str("- ");
    output.push_str(NON_GIT_NEW_FILE_DETAIL);
    output.push_str("\n\nNo files were changed.\n\nStatus: blocked");
    if let Some(detail) = detail {
        output.push_str("\n\nDetail:\n");
        output.push_str(&redact_sensitive_text(detail));
    }
    output
}

/// apply 承認に進む前に git apply --check を session 付きで再実行する。通れば承認待ちの
/// TuiPendingApply を返し、失敗なら blocked 文面で止める。ここでは書き込まない(承認の前段)。
fn prepare_tui_apply(pending: &mut PendingProposal) -> Result<TuiPendingApply, String> {
    let mut session = SessionWriter::create("apply").map_err(|error| error.to_string())?;
    let result = run_tui_apply_check_with_session(pending, &mut session);
    pending.update_check_state(&result);
    let session_path = session.path().display().to_string();

    match result {
        Ok(()) => Ok(TuiPendingApply {
            session,
            session_path,
            pending: pending.clone(),
        }),
        Err(error) => {
            let _ = session.record(SessionEvent::CommandFailed {
                error: error.clone(),
            });
            let _ = session.record(SessionEvent::SessionFinished {
                status: "error".to_string(),
            });
            Err(format_tui_apply_check_failure(&error, &session_path))
        }
    }
}

/// apply 直前の git apply --check 本体。pending の文脈を記録し、check が通った時だけ
/// ApprovalRequested を残す。失敗はそのまま Err で返し、apply に進ませない。
fn run_tui_apply_check_with_session(
    pending: &PendingProposal,
    session: &mut SessionWriter,
) -> Result<(), String> {
    record_tui_pending_context(session, pending, "/apply")?;
    record_tui_session_event(session, SessionEvent::GitApplyCheckStarted)?;
    let repo = discover_git_repo().map_err(|error| format_git_backed_apply_unavailable(&error))?;
    match git_apply_check(&repo, &pending.diff) {
        Ok(()) => {
            record_tui_session_event(session, SessionEvent::GitApplyCheckSucceeded)?;
            record_tui_session_event(session, SessionEvent::ApprovalRequested)?;
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            record_tui_session_event(
                session,
                SessionEvent::GitApplyCheckFailed {
                    error: message.clone(),
                },
            )?;
            Err(message)
        }
    }
}

/// 承認待ち pending_apply に対する y/n 判定。Approved の時だけ finish_tui_apply_approved で
/// 書き込み、それ以外は reject して何も書き換えない。Enter / n / no / その他は reject
/// (parse_approval_decision の規約)。これが手動 `/apply` 経路の permission gate。
pub(super) fn handle_tui_apply_decision(state: &mut TuiState, input: String) {
    let Some(mut approval) = state.pending_apply.take() else {
        return;
    };

    match parse_approval_decision(&input) {
        ApprovalDecision::Approved => {
            let result = finish_tui_apply_approved(&mut approval);
            match result {
                Ok(message) => {
                    state.result_panel = Some(success_result_panel_for_pending(&approval.pending));
                    state.pending_proposal = None;
                    state.proposal_review = None;
                    super::reset_advanced_reveal(state);
                    state.write_progress = None;
                    state.messages.push(TuiMessage::Action(message));
                }
                Err(message) => {
                    state.write_progress = None;
                    state.result_panel = Some(blocked_result_panel_from_message(&message));
                    state.pending_proposal = Some(approval.pending);
                    state.proposal_review =
                        state.proposal_review.take().or(Some(TuiProposalReview {
                            mode: TuiProposalReviewMode::Review,
                            origin: TuiProposalReviewOrigin::Manual,
                            apply_status: TuiProposalApplyStatus::Available,
                            selected_action: 0,
                        }));
                    mark_review_blocked_if_git_unavailable(state, &message);
                    state.messages.push(TuiMessage::Error(message));
                }
            }
        }
        ApprovalDecision::Rejected => {
            let message = finish_tui_apply_rejected(&mut approval);
            state.write_progress = None;
            state.result_panel = Some(TuiResultPanel {
                kind: TuiResultKind::Cancelled,
                files: Vec::new(),
                reason: None,
            });
            state.pending_proposal = Some(approval.pending);
            state.messages.push(TuiMessage::Action(message));
        }
    }
    refresh_status(state);
}

/// 承認後に実際の git apply を行う唯一の書き込み点。approved を記録してから patch を適用し、
/// 成功なら変更ファイルと Created/Updated を返す。apply が失敗したら no-write のまま blocked 文面を返す。
fn finish_tui_apply_approved(approval: &mut TuiPendingApply) -> Result<String, String> {
    record_tui_session_event(
        &mut approval.session,
        SessionEvent::ApprovalDecision {
            decision: "approved".to_string(),
        },
    )?;
    let repo = discover_git_repo().map_err(|error| format_git_backed_apply_unavailable(&error))?;
    match git_apply(&repo, &approval.pending.diff) {
        Ok(()) => {
            let files = changed_files_from_diff(&approval.pending.diff);
            record_tui_session_event(
                &mut approval.session,
                SessionEvent::PatchApplied {
                    files: files.clone(),
                },
            )?;
            record_tui_session_event(
                &mut approval.session,
                SessionEvent::SessionFinished {
                    status: "success".to_string(),
                },
            )?;
            let kind = if approval
                .pending
                .diff
                .lines()
                .any(|line| line.trim() == "--- /dev/null")
            {
                TuiResultKind::Created
            } else {
                TuiResultKind::Updated
            };
            if kind == TuiResultKind::Created {
                Ok(format_tui_apply_success_with_kind(
                    &files,
                    kind,
                    &approval.session_path,
                ))
            } else {
                Ok(format_tui_apply_success(&files, &approval.session_path))
            }
        }
        Err(error) => {
            let message = error.to_string();
            let _ = approval.session.record(SessionEvent::CommandFailed {
                error: message.clone(),
            });
            let _ = approval.session.record(SessionEvent::SessionFinished {
                status: "error".to_string(),
            });
            Err(format_tui_apply_failure(&message, &approval.session_path))
        }
    }
}

/// reject 時の session 記録と表示文。patch は適用せず、No files were changed を返す。
fn finish_tui_apply_rejected(approval: &mut TuiPendingApply) -> String {
    let _ = approval.session.record(SessionEvent::ApprovalDecision {
        decision: "rejected".to_string(),
    });
    let _ = approval.session.record(SessionEvent::PatchRejected);
    let _ = approval.session.record(SessionEvent::SessionFinished {
        status: "success".to_string(),
    });
    format_tui_apply_rejected(&approval.session_path)
}

/// `/check` の中核:session を作り git apply --check を回して結果を pending に反映する。
/// 検証専用で書き込みはしない。
fn run_tui_check_pending(pending: &mut PendingProposal) -> Result<String, String> {
    let mut session = SessionWriter::create("check").map_err(|error| error.to_string())?;
    let result = run_tui_check_pending_with_session(pending, &mut session);
    pending.update_check_state(&result);
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

    let message = match (result, finish_result) {
        (Ok(()), Ok(())) => Ok(format_tui_check_success(pending, &session_path)),
        (Ok(()), Err(error)) => Err(error.to_string()),
        (Err(error), _) => Err(format_tui_check_failure(&error, &session_path)),
    }?;
    Ok(message)
}

/// check の本体:pending の文脈を記録し git apply --check を実行、成否を session event に残す。
fn run_tui_check_pending_with_session(
    pending: &PendingProposal,
    session: &mut SessionWriter,
) -> Result<(), String> {
    record_tui_pending_context(session, pending, "/check")?;
    record_tui_session_event(session, SessionEvent::GitApplyCheckStarted)?;
    let repo = discover_git_repo().map_err(|error| format_git_backed_apply_unavailable(&error))?;
    match git_apply_check(&repo, &pending.diff) {
        Ok(()) => {
            record_tui_session_event(session, SessionEvent::GitApplyCheckSucceeded)?;
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            record_tui_session_event(
                session,
                SessionEvent::GitApplyCheckFailed {
                    error: message.clone(),
                },
            )?;
            Err(message)
        }
    }
}

/// git error を利用者向け文面に整える。repo でない場合は No files were changed を含む
/// 定型文に置き換え、それ以外は原文を返す。
fn format_git_backed_apply_unavailable(error: &origin_tools::GitError) -> String {
    let message = error.to_string();
    if is_git_not_repository_error(&message) {
        "This workspace is not a Git repository.\nGit-backed apply is unavailable here.\nNo files were changed."
            .to_string()
    } else {
        message
    }
}

/// git error が「repo ではない」種別かを判定する小述語。non-git 分岐の入口に使う。
fn is_git_not_repository_error(message: &str) -> bool {
    message.contains("not a Git repository")
}

/// メッセージが git 不在を示す時だけ、review panel の apply 状態を blocked に落とす。
/// それ以外のエラー(通常の check 失敗など)では apply 可否を変えない。
fn mark_review_blocked_if_git_unavailable(state: &mut TuiState, message: &str) {
    if !message.contains("Git-backed apply is unavailable")
        && !message.contains("This workspace is not a Git repository")
    {
        return;
    }
    if let Some(review) = state.proposal_review.as_mut() {
        review.apply_status = non_git_apply_blocked_status();
    }
}

/// check / apply session に pending の文脈(user message・provider・model・diff)を記録する。
/// 監査用メタデータで、secret は redact / session 規約に従う。
pub(super) fn record_tui_pending_context(
    session: &mut SessionWriter,
    pending: &PendingProposal,
    command: &str,
) -> Result<(), String> {
    record_tui_session_event(
        session,
        SessionEvent::UserMessage {
            text: format!("{command} {}", pending.instruction),
        },
    )?;
    record_tui_session_event(
        session,
        SessionEvent::ProviderResolved {
            provider: pending.provider.clone(),
            base_url: pending.base_url.clone(),
        },
    )?;
    record_tui_session_event(
        session,
        SessionEvent::ModelResolved {
            model: pending.model.clone(),
        },
    )?;
    record_tui_session_event(
        session,
        SessionEvent::DiffProposed {
            diff: pending.diff.clone(),
        },
    )
}

/// session への record をエラー文字列化する薄いラッパ。
fn record_tui_session_event(
    session: &mut SessionWriter,
    event: SessionEvent,
) -> Result<(), String> {
    session.record(event).map_err(|error| error.to_string())
}

/// preview 前に diff が拒否された時の文面。reason は redact し、No files were changed を明示する。
pub(super) fn format_tui_propose_error(error: &str) -> String {
    format!(
        "Proposed diff was rejected before preview.\nReason: {}\nNo files were changed.\nNext: inspect /status and try /propose again with a narrower instruction.",
        redact_sensitive_text(error)
    )
}

/// pending proposal が無い時の定型エラー文。
pub(super) fn format_tui_no_pending_proposal() -> String {
    "No pending proposal.\nRun `/propose <file> <instruction...>` first.".to_string()
}

/// proposal preview の結果表示を組む。提案が無ければ no-change を、あれば「review panel が
/// 主経路・手動 fallback コマンド・diff」を示す。常に No files were changed を明示する。
pub(super) fn format_tui_propose_result(
    result: &crate::edit_workflow::ProposePreviewResult,
) -> String {
    let mut output = String::new();
    output.push_str("Proposal preview\n\n");
    output.push_str("Status:\n");
    output.push_str(&format!("  provider: {}\n", result.provider));
    output.push_str(&format!("  model:    {}\n", result.model));
    output.push_str("  applied:  no\n");
    push_target_section(&mut output, &result.requested_files);

    if result.pending.is_none() {
        output.push_str("Next:\n");
        output.push_str(
            "  No changes proposed. Try a narrower /propose instruction or continue chatting.\n",
        );
        output.push_str("  No files were changed.");
    } else {
        output.push_str("Next:\n");
        output.push_str("  Proposal review panel is the normal path.\n");
        output.push_str("  Manual fallback: /diff inspect | /check validate | /apply approve | /clear discard\n\n");
        output.push_str("Diff:\n");
        output.push_str(&result.diff);
        output.push_str("\n\nPending proposal stored. No files were changed.");
    }
    output.push_str(&format!("\n\nsession: {}", result.session_path));
    output
}

/// 保留中 proposal の状態と diff を表示する。check 状態・instruction(redact)・target・
/// proposed files・diff を並べ、review panel が主経路だと案内する。
pub(super) fn format_tui_pending_diff(pending: &PendingProposal) -> String {
    let mut output = String::new();
    output.push_str("Pending proposal\n\n");
    output.push_str("Status:\n");
    output.push_str(&format!(
        "  checked:     {}\n",
        pending.check_state.checked_label()
    ));
    output.push_str(&format!(
        "  instruction: {}\n",
        redact_sensitive_text(&pending.instruction)
    ));
    push_target_section(&mut output, &pending.requested_files);
    if !pending.proposed_files.is_empty() && pending.proposed_files != pending.requested_files {
        output.push_str("Proposed files:\n");
        for file in &pending.proposed_files {
            output.push_str(&format!("  - {}\n", redact_sensitive_text(file)));
        }
    }
    output.push_str("Next:\n");
    output.push_str("  Proposal review panel is the normal path.\n");
    output.push_str("  Manual fallback: /check validate | /apply approve | /clear discard\n");
    output.push_str("\nDiff:\n");
    output.push_str(&pending.diff);
    output
}

/// `/clear` 後の文面。pending proposal を破棄したか否かで分岐し、どちらも no files changed を示す。
pub(super) fn format_tui_clear_result(cleared_pending: bool) -> String {
    if cleared_pending {
        "Clear completed\n\nStatus:\n  visible messages cleared\n  pending proposal discarded\n  no files were changed".to_string()
    } else {
        "Clear completed\n\nStatus:\n  visible messages cleared\n  no pending proposal to discard"
            .to_string()
    }
}

/// check 成功の文面。no files were changed と git apply --check ok を示し、apply には
/// 再 check と y/yes 承認が要ると明記する。
pub(super) fn format_tui_check_success(pending: &PendingProposal, session_path: &str) -> String {
    let mut output = String::new();
    output.push_str("Check passed\n\n");
    output.push_str("Status:\n  no files were changed\n");
    push_target_section(&mut output, &pending.requested_files);
    output.push_str("Check:\n  git apply --check: ok\n");
    output
        .push_str("Next:\n  /apply approve after review | /diff inspect again | /clear discard\n");
    output.push_str("  /apply will re-run git apply --check and require y/yes approval.");
    output.push_str(&format!("\n\nsession: {session_path}"));
    output
}

/// check 失敗の文面。pending を保持・no files changed を示し、redact した理由を載せる。
pub(super) fn format_tui_check_failure(error: &str, session_path: &str) -> String {
    format!(
        "Check failed\n\nStatus:\n  no files were changed\n  pending proposal kept\n\nCheck:\n  git apply --check failed\n  reason: {}\n\nNext:\n  /diff inspect | /propose revise | /clear discard\n\nsession: {}",
        redact_sensitive_text(error),
        session_path
    )
}

/// apply 直前の check 失敗で blocked にする文面。redact した理由と No files were changed を示す。
pub(super) fn format_tui_apply_check_failure(error: &str, session_path: &str) -> String {
    format!(
        "Blocked\n\nReason:\n- git apply --check failed\n- {}\n\nNo files were changed.\n\nStatus: blocked\n\nAdvanced: /diff inspect | /propose revise | /clear discard\n\nsession: {}",
        redact_sensitive_text(error),
        session_path
    )
}

/// apply 承認を求める panel 文面。まだ書き込んでいないことを示し、y/yes で適用・
/// Enter/n/no/その他は reject と明示する permission gate の表示。
pub(super) fn format_tui_apply_approval_prompt(
    pending: &PendingProposal,
    session_path: &str,
) -> String {
    let mut output = String::new();
    output.push_str("Apply approval required\n\n");
    output.push_str("Status:\n  no files have been changed yet\n");
    push_target_section(&mut output, &pending.requested_files);
    output.push_str(
        "Check:\n  git apply --check: ok\n  /apply will only continue after explicit approval\n\n",
    );
    output.push_str(
        "Approval:\n  y or yes applies this patch\n  Enter, n, no, or anything else rejects\n",
    );
    output.push_str(&format!("\nsession: {session_path}"));
    output
}

/// apply 成功(更新)の文面を Updated kind として組む薄いラッパ。
pub(super) fn format_tui_apply_success(files: &[String], session_path: &str) -> String {
    format_tui_apply_success_with_kind(files, TuiResultKind::Updated, session_path)
}

/// apply 成功文面の本体。Created / Updated を切り替え、変更ファイルを redact して並べる。
fn format_tui_apply_success_with_kind(
    files: &[String],
    kind: TuiResultKind,
    session_path: &str,
) -> String {
    let mut output = String::new();
    let title = match kind {
        TuiResultKind::Created => "Created",
        _ => "Updated",
    };
    output.push_str(title);
    output.push_str("\n\n");
    if !files.is_empty() {
        output.push_str("File:");
        for file in files {
            output.push_str(&format!("\n- {}", redact_sensitive_text(file)));
        }
        output.push_str("\n\n");
    }
    output.push_str("No other files were changed.\n\nReady.");
    output.push_str(&format!("\n\nsession: {session_path}"));
    output
}

/// apply 失敗の blocked 文面。redact した理由と No files were changed を示す。
pub(super) fn format_tui_apply_failure(error: &str, session_path: &str) -> String {
    format!(
        "Blocked\n\nReason:\n- {}\n\nNo files were changed.\n\nStatus: blocked\n\nAdvanced: inspect the working tree | /diff inspect | /clear discard\n\nsession: {}",
        redact_sensitive_text(error),
        session_path
    )
}

/// reject 時の Cancelled 文面。No files were changed を示す。
pub(super) fn format_tui_apply_rejected(session_path: &str) -> String {
    format!("Cancelled\n\nNo files were changed.\n\nReady.\n\nsession: {session_path}")
}

/// 出力に Target セクションを追記する共通ヘルパ。ファイルは redact して並べ、空なら proposal と示す。
fn push_target_section(output: &mut String, files: &[String]) {
    output.push_str("\nTarget:\n");
    if files.is_empty() {
        output.push_str("  proposal\n\n");
        return;
    }

    for file in files {
        output.push_str(&format!("  - {}\n", redact_sensitive_text(file)));
    }
    output.push('\n');
}
