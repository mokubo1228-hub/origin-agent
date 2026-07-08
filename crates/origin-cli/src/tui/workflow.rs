//! TUI command 実行、background worker 起動、in-flight request polling。
//! provider 往復は worker に逃がし、late result は request/state guard で main state を守る。
use super::*;

/// slash command 1件を実行する単一ディスパッチ。
/// provider/model の切り替えや /clear では、宙に浮いた pending proposal / apply /
/// review / in-flight worker を明示的に畳んでから状態を更新する(古い承認対象を
/// 残したまま別 model の結果を適用しないための state guard)。
pub(super) fn handle_tui_command(state: &mut TuiState, command: TuiCommand) {
    match command {
        TuiCommand::Help => state.messages.push(TuiMessage::Action(tui_help_text())),
        TuiCommand::Status => state
            .messages
            .push(TuiMessage::Action(format_tui_status(&state.status))),
        TuiCommand::Mode => toggle_tui_mode(state),
        TuiCommand::ModeSet(mode) => set_tui_mode(state, mode),
        TuiCommand::Model => state
            .messages
            .push(TuiMessage::Action(format_tui_model(&state.status))),
        TuiCommand::ModelList => open_model_route_picker(state),
        TuiCommand::ModelSet(model_id) => {
            let result = match parse_model_route_argument(&model_id) {
                Some((provider_id, route_model_id)) => state
                    .runtime
                    .set_model_route(&provider_id, &route_model_id)
                    .map(|_| ()),
                None => state
                    .runtime
                    .set_model_with_source(&model_id, "session")
                    .map(|_| ()),
            };
            match result {
                Ok(()) => {
                    let cleared_pending = state.pending_proposal.take().is_some();
                    state.pending_apply = None;
                    state.proposal_review = None;
                    state.agent_plan_in_flight = None;
                    state.loading = state.chat_in_flight.is_some();
                    state.agent_generation = None;
                    state.agent_propose_in_flight = None;
                    refresh_status(state);
                    state
                        .messages
                        .push(TuiMessage::Action(format_tui_model_set_success(
                            &state.status,
                        )));
                    if cleared_pending {
                        state.messages.push(TuiMessage::Action(
                            "Pending proposal cleared because the model changed.".to_string(),
                        ));
                    }
                }
                Err(error) => state
                    .messages
                    .push(TuiMessage::Error(format_tui_model_set_error(&error))),
            }
        }
        TuiCommand::Provider => state
            .messages
            .push(TuiMessage::Action(format_tui_provider(&state.status))),
        TuiCommand::ProviderList => state
            .messages
            .push(TuiMessage::Action(tui_provider_list(&state.runtime))),
        TuiCommand::ProviderSet(provider_id) => match state.runtime.set_provider(&provider_id) {
            Ok(_) => {
                let cleared_pending = state.pending_proposal.take().is_some();
                state.pending_apply = None;
                state.proposal_review = None;
                state.agent_plan_in_flight = None;
                state.loading = state.chat_in_flight.is_some();
                state.agent_generation = None;
                state.agent_propose_in_flight = None;
                refresh_status(state);
                state
                    .messages
                    .push(TuiMessage::Action(format_tui_provider_set_success(
                        &state.status,
                    )));
                if cleared_pending {
                    state.messages.push(TuiMessage::Action(
                        "Pending proposal cleared because the provider changed.".to_string(),
                    ));
                }
            }
            Err(error) => state
                .messages
                .push(TuiMessage::Error(format_tui_provider_set_error(&error))),
        },
        TuiCommand::Sessions => match list_sessions() {
            Ok(sessions) => state
                .messages
                .push(TuiMessage::Action(format_tui_sessions(&sessions))),
            Err(error) => state
                .messages
                .push(TuiMessage::Error(format_tui_sessions_error(
                    &error.to_string(),
                ))),
        },
        TuiCommand::ResumePicker => open_resume_picker(state),
        TuiCommand::Resume { target } => run_tui_resume(state, target),
        TuiCommand::Propose { path, instruction } => run_tui_propose(state, path, instruction),
        TuiCommand::AgentPropose { path, instruction } => start_tui_agent_propose(
            state,
            TuiAgentProposeRequest::Existing { path, instruction },
        ),
        TuiCommand::AgentProposeNewFile {
            path,
            instruction,
            context_files,
        } => start_tui_agent_propose(
            state,
            TuiAgentProposeRequest::NewFile {
                path,
                instruction,
                context_files,
            },
        ),
        TuiCommand::Diff => run_tui_diff(state),
        TuiCommand::Check => run_tui_check(state),
        TuiCommand::Apply => run_tui_apply(state),
        TuiCommand::Clear => {
            let cleared_pending = state.pending_proposal.take().is_some();
            state.resumed_session = None;
            state.pending_apply = None;
            state.proposal_review = None;
            state.agent_plan_in_flight = None;
            state.loading = state.chat_in_flight.is_some();
            state.agent_generation = None;
            state.agent_propose_in_flight = None;
            state.write_progress = None;
            state.result_panel = None;
            state.pending_agent_existing_edit_plan = None;
            state.pending_agent_new_file_plan = None;
            state.pending_agent_destination_clarification = None;
            state.pending_agent_rationale = None;
            state.pending_agent_new_file_override = false;
            state.pending_agent_action_selection = 0;
            reset_advanced_reveal(state);
            reset_selection_buffer(state);
            state.messages.clear();
            state.active_context.clear();
            refresh_status(state);
            state
                .messages
                .push(TuiMessage::Action(format_tui_clear_result(cleared_pending)));
        }
        TuiCommand::Error(message) => state.messages.push(TuiMessage::Error(message)),
        TuiCommand::Unknown(command) => state.messages.push(TuiMessage::Error(format!(
            "Unknown slash command `{command}`.\nType `/help` for available commands."
        ))),
    }
    pin_chat_to_latest(state);
}

/// Ask ⇄ Agent を反転する薄いラッパ。実際の遷移と防御は set_tui_mode に委譲する。
pub(super) fn toggle_tui_mode(state: &mut TuiState) {
    let next = match state.interaction_mode {
        TuiInteractionMode::Ask => TuiInteractionMode::Agent,
        TuiInteractionMode::Agent => TuiInteractionMode::Ask,
    };
    set_tui_mode(state, next);
}

/// interaction mode を設定する。ただし Agent planning / review / proposal / apply が
/// 未解決の間は変更を拒否する(承認待ちの対象を mode 切り替えで取りこぼさない安全境界)。
pub(super) fn set_tui_mode(state: &mut TuiState, mode: TuiInteractionMode) {
    if has_pending_agent_plan(state)
        || state.agent_plan_in_flight.is_some()
        || state.pending_proposal.is_some()
        || state.pending_apply.is_some()
    {
        state.messages.push(TuiMessage::Error(
            "Mode was not changed because Agent planning, an Agent review, proposal, or apply approval is pending.\nResolve it first, or use /clear to discard pending state."
                .to_string(),
        ));
        return;
    }

    state.interaction_mode = mode;
    reset_advanced_reveal(state);
    reset_selection_buffer(state);
    // 切替イベントを表示だけでなく active context にも積む。これで以降の provider
    // リクエストに再生される会話履歴へ `tui: Mode switched to ...` 行が残り、
    // model が「どこでモードが変わったか」を prompt 先頭の mode contract 行と併せて
    // 認識できる(切替が model に伝わらない従来のズレの修正点)。
    let message = TuiMessage::Action(format_tui_mode_set(mode));
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    refresh_status(state);
}

/// runtime・workspace instructions・user memory・現在の Ask/Agent モード・resumed session
/// から status snapshot を作り直す。pending proposal の有無も畳んで表示用 status に反映する。
pub(super) fn refresh_status(state: &mut TuiState) {
    state.status = StatusSnapshot::from_tui_context(
        &state.runtime,
        pending_proposal_snapshot(state.pending_proposal.as_ref()),
        &state.workspace_instructions,
        &state.user_memory,
        state.interaction_mode.label(),
        state
            .resumed_session
            .as_ref()
            .map(|resumed| resumed.session_id.clone()),
    );
}

/// chat worker の結果を main state に反映する終端。成功なら assistant 応答を、
/// 失敗なら redact 済みエラーを active context と表示の両方へ積む。
pub(super) fn finish_chat_submit(state: &mut TuiState, result: Result<ChatWorkflowResult, String>) {
    state.loading = false;
    match result {
        Ok(result) => {
            let assistant_message = TuiMessage::Assistant(result.response_text);
            state.active_context.push_tui_message(&assistant_message);
            state.messages.push(assistant_message);
            state
                .messages
                .push(TuiMessage::Action("transcript saved".to_string()));
        }
        Err(error) => {
            let error_message = TuiMessage::Error(format_tui_chat_error(&error));
            state.active_context.push_tui_message(&error_message);
            state.messages.push(error_message);
        }
    }
    pin_chat_to_latest(state);
}

/// chat の provider 往復を background thread に逃がす。runtime を clone して worker に
/// 渡し、結果は channel 経由で受ける(main state は in_flight ハンドルだけ持ち、UI を塞がない)。
pub(super) fn start_tui_chat(state: &mut TuiState, submit: TuiChatSubmit) {
    let (sender, receiver) = mpsc::channel();
    let mut runtime = state.runtime.clone();
    let workspace_instructions = state.workspace_instructions.clone();
    let user_memory = state.user_memory.clone();
    let resumed_session = state.resumed_session.clone();

    thread::spawn(move || {
        let result = run_tui_chat(
            &mut runtime,
            submit,
            &workspace_instructions,
            &user_memory,
            resumed_session.as_ref(),
        );
        let _ = sender.send(TuiChatWorkerResult { runtime, result });
    });

    state.chat_in_flight = Some(TuiChatInFlight {
        receiver,
        warning_shown: false,
        started_at: Instant::now(),
    });
    state.loading = true;
}

/// Agent planner の provider 往復を worker に逃がす。送信前に request_id を採番して
/// in_flight と worker 双方に持たせ、後で「今待っている要求の結果か」を識別できるようにする
/// (interrupt 後に届く late result を捨てるための土台)。
pub(super) fn start_tui_agent_plan(state: &mut TuiState, submit: TuiAgentPlanSubmit) {
    let (sender, receiver) = mpsc::channel();
    let mut runtime = state.runtime.clone();
    let request_id = state.next_agent_plan_request_id;
    state.next_agent_plan_request_id = state.next_agent_plan_request_id.wrapping_add(1);
    let input = submit.input;
    let active_context = submit.active_context;
    let background = submit.background;
    let worker_input = input.clone();
    let worker_active_context = active_context.clone();

    thread::spawn(move || {
        let result = {
            let mut planner = RuntimeAgentActionPlanner {
                runtime: &mut runtime,
            };
            planner.plan_agent_action(
                &worker_input,
                worker_active_context.as_deref(),
                background.as_deref(),
            )
        };
        let _ = sender.send(TuiAgentPlanWorkerResult {
            request_id,
            runtime,
            result,
        });
    });

    state.agent_plan_in_flight = Some(TuiAgentPlanInFlight {
        request_id,
        receiver,
        input,
        active_context,
        warning_shown: false,
        started_at: Instant::now(),
    });
    state.loading = true;
    state.result_panel = None;
    pin_chat_to_latest(state);
}

/// Agent の proposal 生成を worker に逃がす。生成結果は poll 側で受け取り、
/// review / permission gate を通るまで書き込みには使われない。
pub(super) fn start_tui_agent_propose(state: &mut TuiState, request: TuiAgentProposeRequest) {
    let (sender, receiver) = mpsc::channel();
    let runtime = state.runtime.clone();

    thread::spawn(move || {
        let result = run_tui_agent_propose_request(runtime, request);
        let _ = sender.send(result);
    });

    state.agent_propose_in_flight = Some(TuiAgentProposeInFlight { receiver });
}

/// 進行中の Agent plan worker を1回だけ非ブロッキングで polling する。
/// 受け取った結果の request_id が現在の in_flight と一致しなければ破棄する
/// (interrupt や再送で入れ替わった後の late result を main state に適用しない state guard)。
pub(super) fn poll_tui_agent_plan_worker(state: &mut TuiState) {
    let worker_result = match state.agent_plan_in_flight.as_ref() {
        Some(in_flight) => match in_flight.receiver.try_recv() {
            Ok(result) => Some(Ok(result)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(
                "Agent planner worker disconnected before returning a response.".to_string(),
            )),
        },
        None => None,
    };

    match worker_result {
        Some(Ok(worker_result)) => {
            let Some(in_flight) = state.agent_plan_in_flight.take() else {
                return;
            };
            if worker_result.request_id != in_flight.request_id {
                return;
            }
            state.runtime = worker_result.runtime;
            refresh_status(state);
            finish_tui_agent_plan_result(
                state,
                in_flight.input,
                in_flight.active_context,
                worker_result.result,
            );
        }
        Some(Err(error)) => {
            let Some(in_flight) = state.agent_plan_in_flight.take() else {
                return;
            };
            finish_tui_agent_plan_result(
                state,
                in_flight.input,
                in_flight.active_context,
                Err(error),
            );
        }
        None => {}
    }
}

/// plan worker の分類結果を submit へ翻訳し、現在の workspace で適用する。
/// workspace を解決できなければ書き込みに進めず destination clarification に落とす(安全側)。
pub(super) fn finish_tui_agent_plan_result(
    state: &mut TuiState,
    input: String,
    active_context: Option<String>,
    planner_result: Result<AgentPlannerAction, String>,
) {
    state.loading = false;
    let result = match Workspace::from_current_dir() {
        Ok(workspace) => agent_submit_from_planner_result(input, &workspace, planner_result),
        Err(error) => agent_workspace_error_submit(input, error.to_string()),
    };
    apply_agent_prompt_submit_result(state, result, active_context);
}

/// 進行中の propose worker を1回 polling する。成功なら結果を適用し、worker 切断などの
/// 失敗時は blocked パネルと「no files changed」を出して終える(中途で書き込まない)。
pub(super) fn poll_tui_agent_propose_worker(state: &mut TuiState) {
    let worker_result = match state.agent_propose_in_flight.as_ref() {
        Some(in_flight) => match in_flight.receiver.try_recv() {
            Ok(result) => Some(Ok(result)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(
                "Agent proposal worker disconnected before returning a response.".to_string(),
            )),
        },
        None => None,
    };

    match worker_result {
        Some(Ok(worker_result)) => {
            state.agent_propose_in_flight = None;
            finish_tui_agent_propose_result(state, worker_result);
        }
        Some(Err(error)) => {
            state.agent_propose_in_flight = None;
            state.agent_generation = None;
            state.result_panel = Some(TuiResultPanel {
                kind: TuiResultKind::Blocked,
                files: Vec::new(),
                reason: Some(error.clone()),
            });
            state.messages.push(TuiMessage::Error(format!(
                "Blocked\n\nReason:\n- {}\n\nNo files were changed.\n\nStatus: blocked",
                redact_sensitive_text(&error)
            )));
            pin_chat_to_latest(state);
        }
        None => {}
    }
}

/// 実行中の Agent generation を中断し、pending な proposal / apply / review / write を
/// すべて畳む。何も書いていないことを保証して Interrupted 結果へ落とす
/// (`esc to interrupt` 表示と実挙動を一致させる中断点)。
pub(super) fn interrupt_agent_generation(state: &mut TuiState) {
    state.agent_propose_in_flight = None;
    state.agent_generation = None;
    state.pending_proposal = None;
    state.pending_apply = None;
    state.proposal_review = None;
    state.write_progress = None;
    state.result_panel = Some(TuiResultPanel {
        kind: TuiResultKind::Interrupted,
        files: Vec::new(),
        reason: None,
    });
    clear_tui_input(state);
    reset_suggestion_selection(state);
    refresh_status(state);
    let message = TuiMessage::Action("Interrupted\n\nNo files were changed.\n\nReady.".to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// 進行中の Agent planning を中断する。in_flight を捨てるので、worker が後から返す
/// late result は poll 側の request_id guard で破棄され、main state には反映されない。
pub(super) fn interrupt_agent_planning(state: &mut TuiState) {
    state.agent_plan_in_flight = None;
    state.loading = false;
    state.result_panel = Some(TuiResultPanel {
        kind: TuiResultKind::Interrupted,
        files: Vec::new(),
        reason: None,
    });
    clear_tui_input(state);
    reset_suggestion_selection(state);
    let message = TuiMessage::Action("Interrupted\n\nNo files were changed.\n\nReady.".to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// 進行中の chat worker を1回 polling する。成功なら更新後 runtime を取り込んで結果を
/// 反映し、切断時は redact 済みエラーを表示する。
pub(super) fn poll_tui_chat_worker(state: &mut TuiState) {
    let worker_result = match state.chat_in_flight.as_ref() {
        Some(in_flight) => match in_flight.receiver.try_recv() {
            Ok(result) => Some(Ok(result)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(
                "Chat worker disconnected before returning a response.".to_string(),
            )),
        },
        None => None,
    };

    match worker_result {
        Some(Ok(worker_result)) => {
            state.chat_in_flight = None;
            state.runtime = worker_result.runtime;
            refresh_status(state);
            finish_chat_submit(state, worker_result.result);
        }
        Some(Err(error)) => {
            state.chat_in_flight = None;
            state.loading = false;
            state
                .messages
                .push(TuiMessage::Error(format_tui_chat_error(&error)));
            pin_chat_to_latest(state);
        }
        None => {}
    }
}

/// Agent plan が進行中の間は新しい submit をブロックする。初回だけ進行中の案内を出し、
/// true を返して送信を止める(plan 結果が出る前に別要求を重ねない)。
pub(super) fn block_submit_while_agent_plan_in_flight(state: &mut TuiState) -> bool {
    let Some(in_flight) = state.agent_plan_in_flight.as_mut() else {
        return false;
    };

    if !in_flight.warning_shown {
        state
            .messages
            .push(TuiMessage::Action(format_tui_agent_plan_in_progress()));
        in_flight.warning_shown = true;
        pin_chat_to_latest(state);
    }

    true
}

/// chat 応答が進行中の間は新しい submit をブロックする。ただし apply 承認待ち
/// (pending_apply)は別概念なので妨げない(plan/action selection と execution approval の分離)。
pub(super) fn block_submit_while_chat_in_flight(state: &mut TuiState) -> bool {
    if state.pending_apply.is_some() {
        return false;
    }

    let Some(in_flight) = state.chat_in_flight.as_mut() else {
        return false;
    };

    if !in_flight.warning_shown {
        state
            .messages
            .push(TuiMessage::Action(format_tui_chat_in_progress()));
        in_flight.warning_shown = true;
        pin_chat_to_latest(state);
    }

    true
}

/// chat が進行中で submit を受けられない時の案内文。
pub(super) fn format_tui_chat_in_progress() -> String {
    "A chat response is still in progress.\nWait for it to finish before submitting another prompt."
        .to_string()
}

/// Agent planning 中の案内文。Esc/Ctrl+C で interrupt 可能なことを併記する(実挙動と一致)。
pub(super) fn format_tui_agent_plan_in_progress() -> String {
    "Agent is still planning the next action.\nWait for it to finish, or press Esc/Ctrl+C to interrupt."
        .to_string()
}

/// worker thread 側で走る chat 実体。model を解決し、mode に応じた context 付きで
/// chat_workflow を回す。main state には触れず、結果だけを channel に返す。
pub(super) fn run_tui_chat(
    runtime: &mut RuntimeState,
    submit: TuiChatSubmit,
    workspace_instructions: &WorkspaceInstructions,
    user_memory: &UserMemory,
    resumed_session: Option<&TuiResumedSession>,
) -> Result<ChatWorkflowResult, String> {
    runtime.ensure_model_for_chat()?;
    let resumed = resumed_session.map(|resumed| ChatResumedSession {
        source_session_id: resumed.session_id.clone(),
        source_path: resumed.source_path.clone(),
        context: resumed.context.clone(),
    });
    chat_workflow::run_chat_with_context_for_mode(
        runtime,
        &submit.prompt,
        submit.file_contexts,
        submit.file_context_records,
        submit.workspace_overview,
        submit.active_context.as_deref(),
        Some(workspace_instructions),
        Some(user_memory),
        resumed.as_ref(),
        submit.prompt_mode,
    )
}

/// `/resume`(引数なし)の実体。直近候補を読んで resume picker を開くだけの read-only 操作で、
/// provider 呼び出し・ファイル変更・状態変更(picker 以外)は一切しない。
/// write permission gate(proposal review / pending apply)や Agent plan が pending の間は、
/// 選択 UI と承認 UI を同時に画面へ出さないため開かずに拒否する。
pub(super) fn open_resume_picker(state: &mut TuiState) {
    if refuse_resume_picker_if_pending(state) {
        return;
    }
    let candidates = origin_session::resume_candidates(RESUME_PICKER_MAX_ROWS);
    open_resume_picker_with_result(state, candidates);
}

/// テスト用:sessions root を注入して picker を開く(本番 open_resume_picker は
/// default root の thin wrapper。ガード・分岐は同一経路を通す)。
#[cfg(test)]
pub(super) fn open_resume_picker_in(state: &mut TuiState, root: &Path) {
    if refuse_resume_picker_if_pending(state) {
        return;
    }
    let candidates = origin_session::resume_candidates_in(root, RESUME_PICKER_MAX_ROWS);
    open_resume_picker_with_result(state, candidates);
}

/// picker を開けない pending 状態なら拒否メッセージを出して true を返す(状態変化ゼロ)。
/// chat_in_flight は許可する(resume background は次リクエストから効くだけで、
/// 進行中リクエストには影響しない)。
fn refuse_resume_picker_if_pending(state: &mut TuiState) -> bool {
    if is_proposal_review_active(state)
        || state.pending_apply.is_some()
        || has_pending_agent_plan(state)
        || state.agent_plan_in_flight.is_some()
        || state.agent_generation.is_some()
        || state.agent_propose_in_flight.is_some()
        || state.write_progress.is_some()
        || state.model_route_picker.is_some()
    {
        state.messages.push(TuiMessage::Error(
            "Resume picker is unavailable while a review or approval is pending.\nResolve it first, then run /resume again."
                .to_string(),
        ));
        pin_chat_to_latest(state);
        return true;
    }
    false
}

/// 候補ロード結果から picker を開く共通段。エラーと0件は picker を開かず
/// 通常メッセージで終える(load error / empty を picker state に持たない)。
fn open_resume_picker_with_result(
    state: &mut TuiState,
    result: Result<Vec<origin_session::ResumeCandidate>, origin_session::SessionError>,
) {
    match result {
        Err(error) => {
            state
                .messages
                .push(TuiMessage::Error(format_tui_resume_error(
                    &error.to_string(),
                )));
        }
        Ok(candidates) if candidates.is_empty() => {
            state.messages.push(TuiMessage::Action(
                "No saved sessions yet.\nStart a chat first — transcripts are saved automatically."
                    .to_string(),
            ));
        }
        Ok(candidates) => {
            state.resume_picker = Some(ResumePicker {
                candidates,
                selected: 0,
                buffer: String::new(),
            });
        }
    }
    pin_chat_to_latest(state);
}

/// resume picker を閉じる(Esc など)。resume_picker 以外のフィールドには一切触らない
/// (キャンセルは状態変化ゼロ、が安全契約)。
pub(super) fn close_resume_picker(state: &mut TuiState) {
    state.resume_picker = None;
}

/// picker の選択候補を lightweight resume に合流させる(Enter)。
/// 候補の中身は使わず常に session_id で再解決し、既存の run_tui_resume →
/// apply_tui_resume_result 経路(読み込み・notice・status 反映)をそのまま通す。
pub(super) fn apply_selected_resume_candidate(state: &mut TuiState) {
    let Some(session_id) = take_selected_resume_session_id(state) else {
        return;
    };
    run_tui_resume(state, Some(session_id));
}

/// テスト用:sessions root を注入した select(本番は default root の run_tui_resume に合流)。
#[cfg(test)]
pub(super) fn apply_selected_resume_candidate_in(state: &mut TuiState, root: &Path) {
    let Some(session_id) = take_selected_resume_session_id(state) else {
        return;
    };
    apply_tui_resume_result(
        state,
        origin_session::build_resume_context_in(root, &session_id),
    );
}

/// 選択中候補の session_id を取り出し、picker を閉じる(select の共通前段)。
fn take_selected_resume_session_id(state: &mut TuiState) -> Option<String> {
    let picker = state.resume_picker.as_ref()?;
    let session_id = picker.candidates.get(picker.selected)?.session_id.clone();
    close_resume_picker(state);
    Some(session_id)
}

/// `/resume [latest|<session-id>]` の実体。過去 transcript から bounded・redacted な
/// 背景要約を作り、以降の chat リクエストへ read-only で添えるだけの lightweight resume。
/// 状態復元・diff 再利用・pending proposal 復元・provider 呼び出し・ファイル変更は一切しない。
pub(super) fn run_tui_resume(state: &mut TuiState, target: Option<String>) {
    let target = target.unwrap_or_else(|| "latest".to_string());
    apply_tui_resume_result(state, build_resume_context(&target));
}

/// resume 結果を TuiState に反映する終端(テストからは結果を注入して呼べる)。
pub(super) fn apply_tui_resume_result(
    state: &mut TuiState,
    result: Result<origin_session::ResumeContext, origin_session::SessionError>,
) {
    match result {
        Ok(resume) => {
            let session_id = resume.source_session_id.clone();
            state.resumed_session = Some(TuiResumedSession {
                session_id: session_id.clone(),
                source_path: resume.source_path.display().to_string(),
                context: resume.context,
            });
            refresh_status(state);
            // 短い notice を active context にも積み、model 側にも resume が起きたこと
            // だけは伝わるようにする(要約本文は provider prompt の専用ブロックで渡る)。
            let message = TuiMessage::Action(format_tui_resume_loaded(&session_id));
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
        }
        Err(error) => state
            .messages
            .push(TuiMessage::Error(format_tui_resume_error(
                &error.to_string(),
            ))),
    }
    pin_chat_to_latest(state);
}
