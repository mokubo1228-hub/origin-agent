//! TUI の run loop と top-level key dispatch の配線。
//! 個々の入力処理・描画・workflow は submodule に委ね、ここでは状態の受け渡しと
//! interruptible な進行状態の駆動だけを持つ。

use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

mod agent;
mod command;
mod composer;
mod context;
mod edit_flow;
mod input;
mod messages;
mod model_picker;
mod render;
mod resume_picker;
mod routing;
mod state;
mod style;
mod terminal;
#[cfg(test)]
mod tests;
mod workflow;

use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::chat_workflow::{
    self, ChatBackground, ChatFileContext, ChatFileContextRecord, ChatPromptMode,
    ChatResumedSession, ChatWorkflowResult, ChatWorkspaceOverviewContext, UserMemory,
    WorkspaceInstructions,
};
use crate::edit_workflow::{pending_proposal_snapshot, PendingCheckState, PendingProposal};
use crate::runtime::{auth_label, RuntimeState, MODEL_SOURCE_UNAVAILABLE};
use crate::status_snapshot::{PendingProposalSnapshot, StatusSnapshot};
use origin_config::{resolve_config, CliOverrides};
use origin_protocol::chat::{ChatMessage, ChatRequest};
use origin_protocol::provider::{ProviderAuth, ProviderPreset};
use origin_protocol::session::SessionEvent;
use origin_provider::{builtin_registry, ModelProvider};
use origin_session::{
    build_resume_context, list_sessions, redact_sensitive_text, sha256_hex, SessionSummary,
    SessionWriter,
};
use origin_tools::discover_git_repo_from;
use origin_tools::{file_read::read_workspace_file, Workspace, WorkspaceError};

use self::composer::{
    backspace_input_char, clamp_to_char_boundary, composer_cell_lines_with_placeholder,
    composer_cursor_offset, composer_line_count, display_width, insert_input_char,
    move_input_cursor_down, move_input_cursor_left, move_input_cursor_right, move_input_cursor_up,
    COMPOSER_BORDER_LINES, MAX_INPUT_TEXT_LINES,
};
#[cfg(test)]
use self::composer::{composer_cell_lines, composer_lines, PROMPT_CONTINUATION_PREFIX};
#[cfg(test)]
use self::edit_flow::{
    apply_non_git_new_file_at_root, format_tui_apply_approval_prompt,
    format_tui_apply_check_failure, format_tui_apply_failure, format_tui_apply_rejected,
    format_tui_apply_success, format_tui_check_failure, format_tui_check_success,
    format_tui_pending_diff, format_tui_propose_error, format_tui_propose_result,
    prepare_non_git_new_file_apply_at_root, record_tui_pending_context, test_pending_apply_fixture,
};
use self::edit_flow::{
    finish_tui_agent_propose_result, format_tui_clear_result, handle_tui_apply_decision,
    run_tui_agent_propose_request, run_tui_apply, run_tui_check, run_tui_diff,
    run_tui_proposal_review_apply, run_tui_propose, TuiAgentProposeRequest,
    TuiAgentProposeWorkerResult, TuiPendingApply,
};
use self::messages::{
    chat_line_blocks, chat_visible_lines, flatten_message_blocks, wrap_chat_lines,
};
#[cfg(test)]
use self::messages::{chat_lines, tui_message_block, CHAT_NEWER_INDICATOR, CHAT_OLDER_INDICATOR};
#[cfg(test)]
use self::model_picker::{format_tui_model_routes, model_route_picker_lines};
use self::model_picker::{model_route_picker_visible_lines, ModelRoutePicker};
#[cfg(test)]
use self::resume_picker::{relative_time_label, resume_picker_lines};
use self::resume_picker::{resume_picker_visible_lines, ResumePicker, RESUME_PICKER_MAX_ROWS};

use self::agent::*;
use self::command::*;
use self::context::*;
use self::input::*;
use self::render::*;
use self::routing::*;
use self::state::*;
use self::terminal::*;
use self::workflow::*;
/// 入力ポーリングの待ち時間。worker 進捗の取り込みと再描画の周期も兼ねる。
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// キーボードスクロール1回で動かすチャット行数。
const KEYBOARD_SCROLL_ROWS: usize = 1;
/// TUI の起点。runtime と workspace instructions を読み、初期 TuiState を組んでから
/// 端末に入り、worker poll → 描画 → 入力 dispatch の run loop を回す。loop を抜けると
/// TuiTerminal の Drop が端末を復帰する。
pub(crate) fn run() -> Result<(), String> {
    let runtime = RuntimeState::initialize_interactive()?;
    let workspace_instructions = WorkspaceInstructions::load_from_cwd(&runtime.cwd);
    let user_memory = UserMemory::load_default();
    let agent_memory = origin_session::memory::default_memory_dir()
        .map(|dir| origin_session::memory::load_snapshot(&dir))
        .unwrap_or_default();
    let status = StatusSnapshot::from_tui_context(
        &runtime,
        PendingProposalSnapshot::none(),
        &workspace_instructions,
        &user_memory,
        TuiInteractionMode::Ask.label(),
        None,
        agent_memory.enabled,
        agent_memory.entries.len(),
    );
    // 初期状態:Ask mode で開始し、pending な提案/承認/Agent plan は全て無し、worker も未起動。
    // 以降はこの単一の TuiState を run loop と全 handler が &mut で共有する。
    let mut state = TuiState {
        messages: Vec::new(),
        status,
        runtime,
        agent_memory,
        input: String::new(),
        input_cursor: 0,
        suggestion_selection: 0,
        chat_scroll_offset: 0,
        loading: false,
        chat_in_flight: None,
        agent_plan_in_flight: None,
        next_agent_plan_request_id: 0,
        pending_proposal: None,
        pending_apply: None,
        proposal_review: None,
        agent_generation: None,
        agent_propose_in_flight: None,
        write_progress: None,
        result_panel: None,
        model_route_picker: None,
        resume_picker: None,
        compact_in_flight: None,
        next_compact_request_id: 0,
        interaction_mode: TuiInteractionMode::Ask,
        active_context: TuiActiveConversationContext::default(),
        pending_agent_existing_edit_plan: None,
        pending_agent_new_file_plan: None,
        pending_agent_destination_clarification: None,
        pending_agent_rationale: None,
        pending_agent_new_file_override: false,
        pending_agent_action_selection: 0,
        show_advanced_actions: false,
        selection_buffer: String::new(),
        workspace_instructions,
        user_memory,
        resumed_session: None,
    };
    // policy ファイルの読み込み警告は黙って握らない(P-5: 気づかない緩和が最悪の失敗)。
    if !state.runtime.policy.warnings.is_empty() {
        state.messages.push(TuiMessage::Error(format!(
            "Policy file warning:\n{}\nDeny rules from that file are not active. Check /status.",
            state.runtime.policy.warnings.join("\n")
        )));
    }
    let mut terminal = TuiTerminal::enter()?;

    // run loop:① background worker の結果取り込み → ② 描画 → ③ 入力を1つ読んで dispatch。
    // worker poll を毎周回の先頭に置くことで、late result(キャンセル後の遅延応答含む)も
    // 各 poll_* 側の guard を通して安全に反映される。
    loop {
        poll_tui_agent_plan_worker(&mut state);
        poll_tui_agent_propose_worker(&mut state);
        poll_tui_chat_worker(&mut state);
        poll_tui_compact_worker(&mut state);
        terminal
            .draw(|frame| render(frame, &state))
            .map_err(|error| format!("Could not draw TUI frame: {error}"))?;

        // 入力イベントが無ければ、再描画と worker 取り込みのためだけに周回する。
        if !event::poll(POLL_INTERVAL).map_err(|error| format!("Could not poll input: {error}"))? {
            continue;
        }

        let event = event::read().map_err(|error| format!("Could not read input: {error}"))?;
        // mouse capture はしていないので Mouse イベントは端末側の選択/コピーに委ねて無視し、
        // 押下(Press)以外の key イベントも無視する。
        let key = match event {
            Event::Key(key) => key,
            Event::Mouse(_) => continue,
            _ => continue,
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // dispatch は優先順位順。まず running state の中断(Esc/Ctrl+C で planning/generation を
        // 止める)を捌き、実際に中断できた時だけここで完了する。
        if handle_running_interrupt_key(&mut state, key.code, key.modifiers) {
            continue;
        }

        // 次に PageUp/PageDown のチャットスクロール(composer 編集とは衝突しない)。
        let area = terminal.size().unwrap_or_else(|_| Rect::new(0, 0, 80, 24));
        if handle_tui_scroll_key(&mut state, area, key.code, key.modifiers) {
            continue;
        }

        // 残りは文脈優先で解釈する。特に Esc は「今開いている最も手前のもの」から畳む:
        // resume picker を閉じる > model route picker を閉じる > proposal review を reject >
        // target override 取消 > agent plan 取消 > どれも無ければ終了。
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Char('q') => break,
            KeyCode::Esc if state.resume_picker.is_some() => {
                close_resume_picker(&mut state);
            }
            KeyCode::Esc if state.model_route_picker.is_some() => {
                close_model_route_picker(&mut state);
            }
            KeyCode::Esc if is_proposal_review_active(&state) => {
                reject_pending_proposal_review(&mut state);
            }
            KeyCode::Esc if state.pending_agent_new_file_override => {
                cancel_agent_target_override(&mut state);
            }
            KeyCode::Esc if has_pending_agent_plan(&state) => {
                cancel_pending_agent_plan(&mut state);
            }
            KeyCode::Esc => break,
            // Proposal review が出ている間のキーは write/create/update の permission gate。
            // ここでの y/n(と panel 内 Enter)はファイル書き込みの承認であって、候補の selection ではない。
            code if is_proposal_review_active(&state) => {
                if should_show_write_progress_for_key(&state, code, key.modifiers) {
                    start_write_progress(&mut state);
                    draw_active_progress_frame_if_needed(&mut terminal, &state)?;
                }
                let (handled, submit) = handle_proposal_review_key(&mut state, code, key.modifiers);
                if handled {
                    if let Some(submit) = submit {
                        draw_active_progress_frame_if_needed(&mut terminal, &state)?;
                        handle_tui_submit(&mut state, submit);
                    }
                    continue;
                }
            }
            // Agent review のキーは plan/action の selection。書き込み承認ではなく、
            // どの候補へ進むかの選択なので、ここで Enter が write gate になることはない。
            // 未処理なら通常の入力編集へ流す。
            code if has_pending_agent_plan(&state) && !state.pending_agent_new_file_override => {
                let (handled, submit) =
                    handle_pending_agent_review_key(&mut state, code, key.modifiers);
                if handled {
                    if let Some(submit) = submit {
                        draw_active_progress_frame_if_needed(&mut terminal, &state)?;
                        handle_tui_submit(&mut state, submit);
                    }
                    continue;
                }
                if handle_tui_input_key(&mut state, code, key.modifiers) {
                    continue;
                }
            }
            // proposal review / agent review のどちらでもなければ、composer 編集(文字入力・
            // カーソル移動・slash 補完など)を試す。消費されたらこの周回はここで完了。
            code if handle_tui_input_key(&mut state, code, key.modifiers) => {}
            // 通常の submit。plan/chat が in-flight の間は二重送信を防ぐ。submit を作れたら
            // 進行表示を即描画してから dispatch し、Exit が返れば true で loop を抜ける。
            KeyCode::Enter => {
                if block_submit_while_agent_plan_in_flight(&mut state) {
                    continue;
                }
                if block_submit_while_chat_in_flight(&mut state) {
                    continue;
                }
                let Some(submit) = prepare_tui_submit(&mut state) else {
                    continue;
                };
                draw_active_progress_frame_if_needed(&mut terminal, &state)?;
                if handle_tui_submit(&mut state, submit) {
                    break;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// in-flight な進行状態(agent plan / generation / write)が立っている時だけ、その場で
/// 1フレーム再描画する。キー処理の途中でも thinking / generating / writing 表示を
/// 即座に出し、次の poll を待たずに状態変化を見せるための割り込み描画。
fn draw_active_progress_frame_if_needed(
    terminal: &mut TuiTerminal,
    state: &TuiState,
) -> Result<(), String> {
    if state.agent_plan_in_flight.is_none()
        && state.agent_generation.is_none()
        && state.write_progress.is_none()
    {
        return Ok(());
    }
    terminal
        .draw(|frame| render(frame, state))
        .map(|_| ())
        .map_err(|error| format!("Could not draw TUI frame: {error}"))
}

/// 確定した TuiSubmit を対応する workflow へ振り分ける唯一の dispatch 点。
/// 戻り値 true は run loop を抜ける合図(Exit のみ)。
fn handle_tui_submit(state: &mut TuiState, submit: TuiSubmit) -> bool {
    match submit {
        TuiSubmit::Chat(chat) => {
            start_tui_chat(state, chat);
            false
        }
        TuiSubmit::AgentPlan(plan) => {
            start_tui_agent_plan(state, plan);
            false
        }
        TuiSubmit::Command(command) => {
            handle_tui_command(state, command);
            false
        }
        TuiSubmit::ApplyDecision(input) => {
            handle_tui_apply_decision(state, input);
            false
        }
        TuiSubmit::Exit => true,
    }
}
