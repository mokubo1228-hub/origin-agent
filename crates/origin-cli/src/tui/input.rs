//! キー / イベント handler と viewport scrolling。
//! Proposal review の y/n write gate と Agent review の action selection を分け、
//! plan 選択が直接 write にならない境界をここで守る。
use super::*;

/// slash 候補の選択位置を先頭に戻す小ヘルパ。入力が変わった時の初期化に使う。
pub(super) fn reset_suggestion_selection(state: &mut TuiState) {
    state.suggestion_selection = 0;
}

/// advanced/debug action の表示フラグを畳む。これらは secondary なので default では隠す。
pub(super) fn reset_advanced_reveal(state: &mut TuiState) {
    state.show_advanced_actions = false;
}

/// 数字選択用の入力バッファを空にする。選択を確定/破棄したタイミングで呼ぶ。
pub(super) fn reset_selection_buffer(state: &mut TuiState) {
    state.selection_buffer.clear();
}

/// composer のキー入力(文字入力・カーソル移動・slash 候補ナビ・model route picker 操作)を
/// 捌く入口。ここは composer 編集だけを担い、ファイル書き込み承認は扱わない。
/// 書き込み承認は handle_proposal_review_key の y/n gate に限る(安全境界)。
pub(super) fn handle_tui_input_key(
    state: &mut TuiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    // resume picker 表示中は、そのキー(数字/矢印/Enter/Esc)を picker 操作に占有する。
    // ここでの選択・Enter は read-only な background 取り込みの選択であって、
    // write 承認ではない(model route picker と同じ文法)。
    if state.resume_picker.is_some() {
        return match code {
            KeyCode::Char(c) if c.is_ascii_digit() => {
                resume_picker_push_digit(state, c);
                true
            }
            KeyCode::Backspace => {
                resume_picker_pop_digit(state);
                true
            }
            KeyCode::Down => {
                clear_resume_picker_buffer(state);
                move_resume_picker_selection(state, 1);
                true
            }
            KeyCode::Up => {
                clear_resume_picker_buffer(state);
                move_resume_picker_selection(state, -1);
                true
            }
            KeyCode::Enter => {
                apply_selected_resume_candidate(state);
                true
            }
            KeyCode::Esc => {
                close_resume_picker(state);
                true
            }
            _ => false,
        };
    }

    // model route picker 表示中は、そのキー(数字/矢印/Enter/Esc)を picker 操作に占有する。
    if state.model_route_picker.is_some() {
        return match code {
            KeyCode::Char(c) if c.is_ascii_digit() => {
                model_route_picker_push_digit(state, c);
                true
            }
            KeyCode::Backspace => {
                model_route_picker_pop_digit(state);
                true
            }
            KeyCode::Down => {
                clear_model_route_picker_buffer(state);
                move_model_route_picker_selection(state, 1);
                true
            }
            KeyCode::Up => {
                clear_model_route_picker_buffer(state);
                move_model_route_picker_selection(state, -1);
                true
            }
            KeyCode::Enter => {
                apply_selected_model_route(state);
                true
            }
            KeyCode::Esc => {
                close_model_route_picker(state);
                true
            }
            _ => false,
        };
    }

    match code {
        KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
            insert_tui_newline(
                &mut state.input,
                &mut state.input_cursor,
                state.pending_apply.is_some(),
            );
            reset_suggestion_selection(state);
            true
        }
        KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
            move_slash_suggestion_selection(state, 1);
            true
        }
        KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
            move_slash_suggestion_selection(state, -1);
            true
        }
        KeyCode::Down => {
            if should_show_slash_suggestions(state) {
                move_slash_suggestion_selection(state, 1);
            } else {
                move_input_cursor_down(&state.input, &mut state.input_cursor);
            }
            true
        }
        KeyCode::Up => {
            if should_show_slash_suggestions(state) {
                move_slash_suggestion_selection(state, -1);
            } else {
                move_input_cursor_up(&state.input, &mut state.input_cursor);
            }
            true
        }
        KeyCode::Tab => {
            accept_slash_suggestion(state);
            true
        }
        KeyCode::Enter => accept_slash_suggestion(state),
        KeyCode::Left => {
            move_input_cursor_left(&state.input, &mut state.input_cursor);
            true
        }
        KeyCode::Right => {
            move_input_cursor_right(&state.input, &mut state.input_cursor);
            true
        }
        KeyCode::Char(value)
            if !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
        {
            insert_input_char(&mut state.input, &mut state.input_cursor, value);
            reset_suggestion_selection(state);
            true
        }
        KeyCode::Backspace => {
            backspace_input_char(&mut state.input, &mut state.input_cursor);
            clamp_suggestion_selection(state);
            true
        }
        _ => false,
    }
}

/// 入力変化で slash 候補数が減っても、選択 index が範囲外に残らないよう丸める。
pub(super) fn clamp_suggestion_selection(state: &mut TuiState) {
    let count = slash_suggestions(
        &state.input,
        state.pending_proposal.is_some(),
        state.pending_apply.is_some(),
    )
    .len();
    if count == 0 {
        state.suggestion_selection = 0;
    } else if state.suggestion_selection >= count {
        state.suggestion_selection = count - 1;
    }
}

/// chat viewport のスクロール方向。Older は過去方向(offset 増)、Newer は最新方向(offset 減)。
#[derive(Clone, Copy)]
pub(super) enum ChatScrollDirection {
    Older,
    Newer,
}

/// 既定のキーボードスクロール行数で chat を1段だけ動かす薄いラッパ。
pub(super) fn scroll_chat_view(state: &mut TuiState, area: Rect, direction: ChatScrollDirection) {
    scroll_chat_view_by_rows(state, area, direction, KEYBOARD_SCROLL_ROWS);
}

/// chat scroll offset を指定行数だけ動かし、wrap 後の内容上限で clamp する。
/// メッセージが空かつ非ロード時はスクロール対象が無いので先頭に固定する。
pub(super) fn scroll_chat_view_by_rows(
    state: &mut TuiState,
    area: Rect,
    direction: ChatScrollDirection,
    rows: usize,
) {
    if state.messages.is_empty() && !state.loading {
        state.chat_scroll_offset = 0;
        return;
    }

    let rows = rows.max(1);
    match direction {
        ChatScrollDirection::Older => {
            state.chat_scroll_offset = state.chat_scroll_offset.saturating_add(rows);
        }
        ChatScrollDirection::Newer => {
            state.chat_scroll_offset = state.chat_scroll_offset.saturating_sub(rows);
        }
    }
    state.chat_scroll_offset = state
        .chat_scroll_offset
        .min(max_chat_scroll_offset(state, area.width));
}

/// PageUp/PageDown を chat スクロールに割り当てる。modifier 付きキーは扱わない。
pub(super) fn handle_tui_scroll_key(
    state: &mut TuiState,
    terminal_area: Rect,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if !modifiers.is_empty() {
        return false;
    }

    let chat_area = chat_area(terminal_area, state);
    match code {
        KeyCode::PageUp => {
            scroll_chat_view(state, chat_area, ChatScrollDirection::Older);
            true
        }
        KeyCode::PageDown => {
            scroll_chat_view(state, chat_area, ChatScrollDirection::Newer);
            true
        }
        _ => false,
    }
}

/// scroll offset を 0 に戻して最新行へ追従させる。新メッセージ追加後に呼ぶ。
pub(super) fn pin_chat_to_latest(state: &mut TuiState) {
    state.chat_scroll_offset = 0;
}

/// 現在の幅で wrap した総行数から、過去方向に許される最大スクロール量を求める。
pub(super) fn max_chat_scroll_offset(state: &TuiState, visible_width: u16) -> usize {
    wrap_chat_lines(
        flatten_message_blocks(chat_line_blocks(&state.messages, state.loading)),
        visible_width,
    )
    .len()
    .saturating_sub(1)
}

/// slash 候補リスト内で選択を delta だけ動かす。候補非表示や0件なら何もしない。
pub(super) fn move_slash_suggestion_selection(state: &mut TuiState, delta: isize) {
    if !should_show_slash_suggestions(state) {
        return;
    }

    let count = slash_suggestions(
        &state.input,
        state.pending_proposal.is_some(),
        state.pending_apply.is_some(),
    )
    .len();
    if count == 0 {
        state.suggestion_selection = 0;
        return;
    }

    let current = state.suggestion_selection.min(count - 1);
    state.suggestion_selection = if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs()).min(count - 1)
    } else {
        (current + delta as usize).min(count - 1)
    };
}

/// 選択中の slash 候補で入力を補完する。候補非表示や補完で変化しない場合は false。
pub(super) fn accept_slash_suggestion(state: &mut TuiState) -> bool {
    if !should_show_slash_suggestions(state) {
        return false;
    }

    let suggestions = slash_suggestions(
        &state.input,
        state.pending_proposal.is_some(),
        state.pending_apply.is_some(),
    );
    let Some(suggestion) = suggestions.get(state.suggestion_selection) else {
        return false;
    };

    let completion = slash_completion_text(*suggestion);
    if completion == state.input {
        return false;
    }

    state.input = completion;
    state.input_cursor = state.input.len();
    reset_suggestion_selection(state);
    true
}

/// resume picker の選択行を delta だけ動かし、範囲内に clamp する(wrap しない)。
pub(super) fn move_resume_picker_selection(state: &mut TuiState, delta: isize) {
    let Some(picker) = state.resume_picker.as_mut() else {
        return;
    };
    if picker.candidates.is_empty() {
        picker.selected = 0;
        return;
    }

    let max = picker.candidates.len() - 1;
    picker.selected = if delta.is_negative() {
        picker
            .selected
            .saturating_sub(delta.unsigned_abs())
            .min(max)
    } else {
        picker.selected.saturating_add(delta as usize).min(max)
    };
}

/// resume picker の数字入力バッファを空にする(矢印操作に切り替えた時など)。
pub(super) fn clear_resume_picker_buffer(state: &mut TuiState) {
    if let Some(picker) = state.resume_picker.as_mut() {
        picker.buffer.clear();
    }
}

/// 数字キーをバッファに積み、1..=候補数 の数値なら選択へ反映する。
/// 範囲外バッファは選択を変えない(bare digit で resume は実行されない)。
pub(super) fn resume_picker_push_digit(state: &mut TuiState, digit: char) {
    if let Some(picker) = state.resume_picker.as_mut() {
        picker.buffer.push(digit);
        apply_resume_picker_buffer(picker);
    }
}

/// バッファ末尾の数字を1つ削り、選択を再計算する(Backspace)。
pub(super) fn resume_picker_pop_digit(state: &mut TuiState) {
    if let Some(picker) = state.resume_picker.as_mut() {
        picker.buffer.pop();
        apply_resume_picker_buffer(picker);
    }
}

/// 数字バッファを「候補の n 番目」として解釈し selected を更新する。
fn apply_resume_picker_buffer(picker: &mut ResumePicker) {
    if let Some(index) = style::selection_buffer_highlight(&picker.buffer, picker.candidates.len())
    {
        picker.selected = index;
    }
}

/// runtime から model route 一覧を集めて picker を開く入口。
pub(super) fn open_model_route_picker(state: &mut TuiState) {
    let routes = model_route_entries(&state.runtime);
    open_model_route_picker_with_routes(state, routes);
}

/// route 一覧を受けて picker state を構築する。空なら開かず、設定方法の案内だけ出す。
pub(super) fn open_model_route_picker_with_routes(
    state: &mut TuiState,
    routes: Vec<ModelRouteEntry>,
) {
    if routes.is_empty() {
        state.messages.push(TuiMessage::Action(
            "No configured model routes found.\nUse /model set <model-id> to set a model for the current provider."
                .to_string(),
        ));
        pin_chat_to_latest(state);
        return;
    }

    let selected = routes
        .iter()
        .position(|route| route.current)
        .or_else(|| {
            routes
                .iter()
                .position(|route| route.recommended && route.selectable)
        })
        .unwrap_or(0);
    state.model_route_picker = Some(ModelRoutePicker {
        routes,
        selected,
        numeric_buffer: String::new(),
    });
    state.chat_scroll_offset = 0;
}

/// model route picker を閉じる(Esc など)。
pub(super) fn close_model_route_picker(state: &mut TuiState) {
    state.model_route_picker = None;
}

/// picker の選択行を delta だけ動かし、範囲内に clamp する。
pub(super) fn move_model_route_picker_selection(state: &mut TuiState, delta: isize) {
    let Some(picker) = state.model_route_picker.as_mut() else {
        return;
    };
    if picker.routes.is_empty() {
        picker.selected = 0;
        return;
    }

    let max = picker.routes.len() - 1;
    picker.selected = if delta.is_negative() {
        picker
            .selected
            .saturating_sub(delta.unsigned_abs())
            .min(max)
    } else {
        picker.selected.saturating_add(delta as usize).min(max)
    };
}

/// picker の数字入力バッファを空にする(矢印操作に切り替えた時など)。
pub(super) fn clear_model_route_picker_buffer(state: &mut TuiState) {
    if let Some(picker) = state.model_route_picker.as_mut() {
        picker.numeric_buffer.clear();
    }
}

/// 数字キーをバッファに積み、対応する route を選択へ反映する。
pub(super) fn model_route_picker_push_digit(state: &mut TuiState, digit: char) {
    if let Some(picker) = state.model_route_picker.as_mut() {
        picker.numeric_buffer.push(digit);
        apply_model_route_picker_buffer(picker);
    }
}

/// バッファ末尾の数字を1つ削り、選択を再計算する(Backspace)。
pub(super) fn model_route_picker_pop_digit(state: &mut TuiState) {
    if let Some(picker) = state.model_route_picker.as_mut() {
        picker.numeric_buffer.pop();
        apply_model_route_picker_buffer(picker);
    }
}

/// 数字バッファを「選択可能 route の n 番目」として解釈し selected を更新する。
/// 選択不能なプレースホルダ行は対象から外し、有効な route だけを数える。
pub(super) fn apply_model_route_picker_buffer(picker: &mut ModelRoutePicker) {
    let selectable: Vec<usize> = picker
        .routes
        .iter()
        .enumerate()
        .filter(|(_, route)| !(!route.selectable && route.model_id.is_empty()))
        .map(|(index, _)| index)
        .collect();
    if let Some(nth) = style::selection_buffer_highlight(&picker.numeric_buffer, selectable.len()) {
        if let Some(&routes_index) = selectable.get(nth) {
            picker.selected = routes_index;
        }
    }
}

/// 選択中の route を runtime に設定する。これは model 切替であってファイル書き込みではない。
/// 切替時は in-flight な proposal / apply / agent plan を破棄して状態の食い違いを防ぎ、
/// status を更新する。選択不能 route は理由を出して何も変えない。
pub(super) fn apply_selected_model_route(state: &mut TuiState) {
    let Some(picker) = state.model_route_picker.take() else {
        return;
    };
    let Some(route) = picker.routes.get(picker.selected) else {
        return;
    };

    if !route.selectable {
        state
            .messages
            .push(TuiMessage::Error(format_tui_model_route_unavailable(route)));
        pin_chat_to_latest(state);
        return;
    }

    match state
        .runtime
        .set_model_route(&route.provider_id, &route.model_id)
    {
        Ok(_) => {
            let cleared_pending = collapse_pending_flows(state);
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
    pin_chat_to_latest(state);
}

/// composer の Enter/submit を、現在の pending 状態に応じて適切な経路へ振り分ける主ディスパッチ。
/// pending_apply 中は ApplyDecision、proposal review 中は revision 入力 or action hint、
/// agent plan 保留中は plan/action selection、それ以外は slash command / Agent plan / Ask chat。
/// 安全境界:ここで通常の composer 入力が直接ファイル書き込みになることはない。
/// 書き込み承認は handle_proposal_review_key の y/n gate に限る。
pub(super) fn prepare_tui_submit(state: &mut TuiState) -> Option<TuiSubmit> {
    let input = state.input.trim().to_string();
    if is_terminal_control_fragment(&input) {
        clear_tui_input(state);
        reset_suggestion_selection(state);
        return None;
    }
    if state.pending_apply.is_some() {
        clear_tui_input(state);
        reset_suggestion_selection(state);
        return Some(TuiSubmit::ApplyDecision(input));
    }

    if is_proposal_review_active(state) {
        clear_tui_input(state);
        reset_suggestion_selection(state);
        if is_proposal_revision_input_active(state) {
            if input.is_empty() {
                push_proposal_review_action_hint(state);
                return None;
            }
            return revise_pending_proposal_from_feedback(state, input);
        }
        push_proposal_review_action_hint(state);
        return None;
    }

    if state.pending_agent_new_file_override {
        clear_tui_input(state);
        reset_suggestion_selection(state);
        let messages = update_pending_agent_new_file_plan_path(state, &input);
        for message in &messages {
            state.active_context.push_tui_message(message);
        }
        state.messages.extend(messages);
        pin_chat_to_latest(state);
        return None;
    }

    // 空 Enter かつ Agent plan 保留中は、選択中の plan/action を実行する。これは plan/action
    // selection であって書き込み承認ではない(read-only preview 生成や入力やり直しに進む)。
    if input.is_empty() && has_pending_agent_plan(state) {
        clear_tui_input(state);
        reset_suggestion_selection(state);
        let action = state.pending_agent_action_selection;
        return run_pending_agent_action(state, action);
    }

    if input.is_empty() {
        return None;
    }

    if input.starts_with('/') {
        state.result_panel = None;
        clear_tui_input(state);
        reset_suggestion_selection(state);
        if input.contains('\n') {
            return Some(TuiSubmit::Command(TuiCommand::Error(
                "Multiline slash commands are not supported yet.\nKeep slash commands on one line."
                    .to_string(),
            )));
        }
        return Some(parse_tui_command(&input));
    }

    let model_state = model_selection_state(&state.runtime);
    if model_state != TuiModelSelectionState::Selected {
        state
            .messages
            .push(TuiMessage::Error(format_tui_send_blocked_without_model(
                &model_state,
            )));
        pin_chat_to_latest(state);
        return None;
    }

    state.result_panel = None;
    clear_tui_input(state);
    reset_suggestion_selection(state);

    if state.interaction_mode == TuiInteractionMode::Agent && has_pending_agent_plan(state) {
        let message = TuiMessage::Action(format_tui_agent_plan_action_hint_for_state(state));
        state.active_context.push_tui_message(&message);
        state.messages.push(message);
        pin_chat_to_latest(state);
        return None;
    }

    let active_context = state.active_context.provider_context();
    let user_message = TuiMessage::User(input.clone());
    state.active_context.push_tui_message(&user_message);
    state.messages.push(user_message);
    if state.interaction_mode == TuiInteractionMode::Agent {
        state.loading = true;
        pin_chat_to_latest(state);
        let background = build_agent_planner_background(&chat_background_from_state(state));
        return Some(TuiSubmit::AgentPlan(TuiAgentPlanSubmit {
            input,
            active_context,
            background,
        }));
    }

    let ask_context = ask_file_context_from_prompt(&input, &state.runtime.policy);
    for message in &ask_context.messages {
        state.active_context.push_tui_message(message);
    }
    state.messages.extend(ask_context.messages);
    state.loading = true;
    pin_chat_to_latest(state);
    Some(TuiSubmit::Chat(TuiChatSubmit {
        prompt: input,
        active_context,
        prompt_mode: ChatPromptMode::Ask,
        file_contexts: ask_context.file_contexts,
        file_context_records: ask_context.file_context_records,
        workspace_overview: ask_context.workspace_overview,
    }))
}

/// 端末が貼り付けた制御シーケンス断片(ESC 始まりや SGR マウスレポート `[<...M`)を
/// 通常入力の submit と誤認しないための判定。該当時は submit せず破棄する。
pub(super) fn is_terminal_control_fragment(input: &str) -> bool {
    let value = input.trim();
    if value.is_empty() {
        return false;
    }
    if value.starts_with('\u{1b}') {
        return true;
    }
    let Some(body) = value.strip_prefix("[<") else {
        return false;
    };
    let Some(body) = body.strip_suffix('M').or_else(|| body.strip_suffix('m')) else {
        return false;
    };
    !body.is_empty()
        && body
            .chars()
            .all(|character| character.is_ascii_digit() || character == ';')
}

/// Chat submit にだけ active conversation context を後付けする。他種の submit は素通し。
pub(super) fn tui_submit_with_active_context(
    submit: TuiSubmit,
    active_context: Option<String>,
) -> TuiSubmit {
    match submit {
        TuiSubmit::Chat(mut chat) => {
            chat.active_context = active_context;
            TuiSubmit::Chat(chat)
        }
        other => other,
    }
}

/// Agent の保留 plan(既存編集 / 新規作成 / destination clarification)が立っているか。
pub(super) fn has_pending_agent_plan(state: &TuiState) -> bool {
    state.pending_agent_existing_edit_plan.is_some()
        || state.pending_agent_new_file_plan.is_some()
        || state.pending_agent_destination_clarification.is_some()
}

/// pending proposal と review panel が両方立っている = write permission gate を表示中か。
pub(super) fn is_proposal_review_active(state: &TuiState) -> bool {
    state.pending_proposal.is_some() && state.proposal_review.is_some()
}

/// write permission gate(Proposal review の y/n、または apply 承認プロンプト)が
/// 画面を占有しているか。guard の正の入口(FD-2)— gate 系 state が増えたらここに足す。
pub(super) fn is_permission_gate_active(state: &TuiState) -> bool {
    is_proposal_review_active(state) || state.pending_apply.is_some()
}

/// Agent の plan / review / preview 生成フローのいずれかが進行・保留中か。
/// guard の正の入口(FD-2)。resume picker guard と、新規 guard(CP1 §1 等)がここを使う。
pub(super) fn has_blocking_agent_flow(state: &TuiState) -> bool {
    has_pending_agent_plan(state)
        || state.agent_plan_in_flight.is_some()
        || state.agent_propose_in_flight.is_some()
        || state.agent_generation.is_some()
}

/// 単一 overlay(picker 系)が開いているか。picker が増えたらここだけ伸ばす(FD-2)。
pub(super) fn is_any_overlay_open(state: &TuiState) -> bool {
    state.model_route_picker.is_some() || state.resume_picker.is_some()
}

/// provider 往復 worker のいずれかが進行中か。guard の正の入口(FD-2)。
/// 現行コードに同一集合の既存 guard は無く、新規 guard(CP1 §1 等)がここを使う。
#[allow(dead_code)]
pub(super) fn is_any_provider_call_in_flight(state: &TuiState) -> bool {
    state.chat_in_flight.is_some()
        || state.agent_plan_in_flight.is_some()
        || state.agent_propose_in_flight.is_some()
}

/// proposal review が revision feedback 入力モードか。この間の Enter は書き込みではなく送信。
pub(super) fn is_proposal_revision_input_active(state: &TuiState) -> bool {
    matches!(
        state.proposal_review.as_ref().map(|review| review.mode),
        Some(TuiProposalReviewMode::RevisionInput)
    )
}

/// apply が blocked(例: git 非対応 workspace)状態か。blocked 中は y/Enter でも書き込まない。
pub(super) fn is_proposal_apply_blocked(state: &TuiState) -> bool {
    matches!(
        state
            .proposal_review
            .as_ref()
            .map(|review| &review.apply_status),
        Some(TuiProposalApplyStatus::Blocked { .. })
    )
}

/// 進行中の preview 生成を中断してよい状態か(interruptible かつ propose が in-flight)。
pub(super) fn is_agent_generation_interruptible(state: &TuiState) -> bool {
    state.agent_generation.as_ref().is_some_and(|generation| {
        generation.interruptible && state.agent_propose_in_flight.is_some()
    })
}

/// 進行中の planner 呼び出しを中断してよい状態か。
pub(super) fn is_agent_planning_interruptible(state: &TuiState) -> bool {
    state.agent_plan_in_flight.is_some()
}

/// Esc / Ctrl+C を running state の interrupt に割り当てる。
/// 実際に中断できる時(planning / generation が interruptible)だけ処理して true を返す。
/// `esc to interrupt` の表示と実挙動を一致させるための門番(できない時は握らない)。
pub(super) fn handle_running_interrupt_key(
    state: &mut TuiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    let is_ctrl_c = matches!(code, KeyCode::Char('c')) && modifiers.contains(KeyModifiers::CONTROL);
    let is_esc = matches!(code, KeyCode::Esc) && modifiers.is_empty();
    if (is_ctrl_c || is_esc) && is_agent_planning_interruptible(state) {
        interrupt_agent_planning(state);
        return true;
    }
    if (is_ctrl_c || is_esc) && is_agent_generation_interruptible(state) {
        interrupt_agent_generation(state);
        return true;
    }
    // /compact は常に中断可能(context 無傷で worker を捨てるだけ)。
    if (is_ctrl_c || is_esc) && state.compact_in_flight.is_some() {
        interrupt_compact(state);
        return true;
    }
    false
}

/// 「writing...」進行表示を出してよいキーかを判定する。
/// 表示条件は実際に書き込む条件(handle_proposal_review_key の y / save-Enter)と
/// 厳密に一致させる。ここがズレると「書かないのに writing」「書くのに無表示」に
/// なって安全表示が嘘になるため、両者は常に同じ条件に保つ。
pub(super) fn should_show_write_progress_for_key(
    state: &TuiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    if modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::ALT) {
        return false;
    }
    let enter_confirms_save = matches!(code, KeyCode::Enter)
        && state
            .proposal_review
            .as_ref()
            .is_some_and(|review| review.selected_action == 0);
    (matches!(code, KeyCode::Char('y') | KeyCode::Char('Y')) || enter_confirms_save)
        && is_proposal_review_active(state)
        && !is_proposal_revision_input_active(state)
        && !is_proposal_apply_blocked(state)
}

/// 書き込み開始の進行表示(writing...)を立てる。対象表示は pending proposal の安全パス要約。
pub(super) fn start_write_progress(state: &mut TuiState) {
    let target_summary = state
        .pending_proposal
        .as_ref()
        .map(|pending| safe_paths_summary(&pending.requested_files))
        .unwrap_or_else(|| "proposal".to_string());
    state.write_progress = Some(TuiWriteProgress {
        target_summary,
        started_at: Instant::now(),
    });
}

/// Proposal review パネルのキー処理 = ファイル書き込みの permission gate。
/// `y`/`Y` だけが実際の書き込み(run_tui_proposal_review_apply)を起動する。
/// `Enter` は「save」アクションが選択され(selected_action == 0)かつ blocked で
/// ない時だけ書き込みに等しい。それ以外の Enter / `n` / 他キーは書き込まない。
/// ここは origin で唯一 external state を変える経路なので、この承認条件
/// (明示パネル内の y / 選択済み save の Enter)を緩めない。
pub(super) fn handle_proposal_review_key(
    state: &mut TuiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> (bool, Option<TuiSubmit>) {
    if modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::ALT) {
        return (false, None);
    }

    if is_proposal_revision_input_active(state) {
        return match code {
            KeyCode::Esc => {
                reject_pending_proposal_review(state);
                (true, None)
            }
            _ => (false, None),
        };
    }

    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if is_proposal_apply_blocked(state) {
                push_apply_blocked_message(state);
            } else {
                run_tui_proposal_review_apply(state);
            }
            (true, None)
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            reject_pending_proposal_review(state);
            (true, None)
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if let Some(review) = state.proposal_review.as_mut() {
                review.mode = TuiProposalReviewMode::RevisionInput;
            }
            let message = TuiMessage::Action(
                "Type revision feedback, then press Enter. No files will be changed.".to_string(),
            );
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
            pin_chat_to_latest(state);
            (true, None)
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            run_tui_diff(state);
            (true, None)
        }
        KeyCode::Char('c') | KeyCode::Char('C') if is_proposal_apply_blocked(state) => {
            clear_pending_proposal_review(state);
            (true, None)
        }
        KeyCode::Char('?') => {
            state.show_advanced_actions = !state.show_advanced_actions;
            (true, None)
        }
        KeyCode::Up => {
            move_proposal_review_selection(state, -1);
            (true, None)
        }
        KeyCode::Down => {
            move_proposal_review_selection(state, 1);
            (true, None)
        }
        KeyCode::Enter => {
            clear_tui_input(state);
            reset_suggestion_selection(state);
            if is_proposal_apply_blocked(state) {
                push_proposal_review_action_hint(state);
            } else if state
                .proposal_review
                .as_ref()
                .is_some_and(|review| review.selected_action == 0)
            {
                run_tui_proposal_review_apply(state);
            } else {
                reject_pending_proposal_review(state);
            }
            (true, None)
        }
        KeyCode::Char(_) => {
            push_proposal_review_action_hint(state);
            (true, None)
        }
        _ => (true, None),
    }
}

/// proposal review の選択(save / cancel の2択)を delta だけ動かす。blocked 中は動かさない。
/// この選択自体は書き込みではなく、確定は handle_proposal_review_key の承認に委ねる。
pub(super) fn move_proposal_review_selection(state: &mut TuiState, delta: isize) {
    if is_proposal_apply_blocked(state) {
        return;
    }
    let Some(review) = state.proposal_review.as_mut() else {
        return;
    };
    review.selected_action = if delta.is_negative() {
        review.selected_action.saturating_sub(delta.unsigned_abs())
    } else {
        review.selected_action.saturating_add(delta as usize).min(1)
    };
}

/// Agent review パネルのキー処理 = plan/action selection であって write 承認ではない。
/// 数字キーは即時に run_pending_agent_action を呼ぶが、起動するのは preview 生成
/// (read-only)・target 調整・cancel のいずれかだけで、ここから直接ファイルが
/// 書かれることはない。書き込み承認は別の Proposal review(y/n gate)に限る。
pub(super) fn handle_pending_agent_review_key(
    state: &mut TuiState,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> (bool, Option<TuiSubmit>) {
    if modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::ALT) {
        return (false, None);
    }

    match code {
        KeyCode::Char(c) if c.is_ascii_digit() => {
            reset_selection_buffer(state);
            clear_tui_input(state);
            reset_suggestion_selection(state);
            let count = pending_agent_action_count(state);
            let action = c
                .to_digit(10)
                .and_then(|digit| usize::try_from(digit).ok())
                .and_then(|digit| digit.checked_sub(1));
            if let Some(action) = action.filter(|action| *action < count) {
                (true, run_pending_agent_action(state, action))
            } else {
                push_agent_plan_action_hint(state);
                (true, None)
            }
        }
        KeyCode::Backspace => {
            state.selection_buffer.pop();
            let count = pending_agent_action_count(state);
            if let Some(index) = style::selection_buffer_highlight(&state.selection_buffer, count) {
                state.pending_agent_action_selection = index;
            }
            (true, None)
        }
        KeyCode::Up => {
            reset_selection_buffer(state);
            move_pending_agent_action_selection(state, -1);
            (true, None)
        }
        KeyCode::Down => {
            reset_selection_buffer(state);
            move_pending_agent_action_selection(state, 1);
            (true, None)
        }
        KeyCode::Enter => {
            reset_selection_buffer(state);
            clear_tui_input(state);
            reset_suggestion_selection(state);
            let action = state.pending_agent_action_selection;
            (true, run_pending_agent_action(state, action))
        }
        KeyCode::Char(_) => {
            push_agent_plan_action_hint(state);
            (true, None)
        }
        _ => (true, None),
    }
}

/// Agent review の選択肢を実行に移す。
/// 0 = preview 生成へ進む、1 = target 変更(新規作成時)/ 指示の手直し、2 = cancel。
/// destination clarification 中は4択側(run_pending_destination_clarification_action)へ
/// 委譲する。いずれも read-only な次段(preview)か入力やり直しで、
/// external state は変えない。
pub(super) fn run_pending_agent_action(state: &mut TuiState, action: usize) -> Option<TuiSubmit> {
    let max = pending_agent_action_count(state).saturating_sub(1);
    state.pending_agent_action_selection = action.min(max);
    if state.pending_agent_destination_clarification.is_some() {
        return run_pending_destination_clarification_action(
            state,
            state.pending_agent_action_selection,
        );
    }
    match state.pending_agent_action_selection {
        0 => confirm_pending_agent_plan(state),
        1 if state.pending_agent_new_file_plan.is_some() => enter_agent_target_override(state),
        1 => return_agent_review_to_composer(state),
        2 => {
            cancel_pending_agent_plan(state);
            None
        }
        _ => {
            push_agent_plan_action_hint(state);
            None
        }
    }
}

/// Agent review の選択 index を delta だけ動かし上限で clamp する。これは plan/action selection。
pub(super) fn move_pending_agent_action_selection(state: &mut TuiState, delta: isize) {
    let max = pending_agent_action_count(state).saturating_sub(1);
    state.pending_agent_action_selection = if delta.is_negative() {
        state
            .pending_agent_action_selection
            .saturating_sub(delta.unsigned_abs())
    } else {
        state
            .pending_agent_action_selection
            .saturating_add(delta as usize)
            .min(max)
    };
}

/// 現在の Agent review が何択かを返す(destination clarification は4、通常 plan は3、無ければ0)。
/// 選択の clamp や数字キー入力の上限判定に使う。
pub(super) fn pending_agent_action_count(state: &TuiState) -> usize {
    if state.pending_agent_destination_clarification.is_some() {
        4
    } else if has_pending_agent_plan(state) {
        3
    } else {
        0
    }
}

/// 「行き先不明」時の4択(0=既存更新へ / 1=新規作成へ / 2=custom instruction / 3=cancel)を実行する。
/// いずれも composer へ戻して再入力させるか cancel するだけで、provider 呼び出しも書き込みもしない。
pub(super) fn run_pending_destination_clarification_action(
    state: &mut TuiState,
    action: usize,
) -> Option<TuiSubmit> {
    match action {
        0 => return_destination_clarification_to_composer(
            state,
            "Add one existing file path to update, then press Enter.",
            None,
        ),
        1 => return_destination_clarification_to_composer(
            state,
            "Add one new safe relative Markdown path, then press Enter.",
            Some("Create a new Markdown file at "),
        ),
        2 => return_destination_clarification_to_composer(
            state,
            "Type a custom instruction, then press Enter.",
            None,
        ),
        3 => {
            cancel_pending_agent_plan(state);
            None
        }
        _ => {
            push_agent_plan_action_hint(state);
            None
        }
    }
}

/// destination clarification を畳み、元プロンプト(+任意の prefix)を composer に戻して再入力を促す。
/// provider は呼ばずファイルも変えない。clarification 4択の共通の戻し処理。
pub(super) fn return_destination_clarification_to_composer(
    state: &mut TuiState,
    instruction: &str,
    prefix: Option<&str>,
) -> Option<TuiSubmit> {
    let prompt = state
        .pending_agent_destination_clarification
        .take()
        .map(|clarification| clarification.original_prompt);
    state.pending_agent_rationale = None;
    state.pending_agent_action_selection = 0;
    reset_selection_buffer(state);
    if let Some(prompt) = prompt {
        state.input = if let Some(prefix) = prefix {
            format!("{prompt}\n\n{prefix}")
        } else {
            format!("{prompt} ")
        };
        state.input_cursor = state.input.len();
    }
    let message = TuiMessage::Action(format!(
        "{instruction}\nNo provider preview was generated. No files were changed."
    ));
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
    None
}

/// 選択された Agent plan を preview 生成段へ進める(action 0)。
/// 既存編集なら AgentPropose、新規作成なら AgentProposeNewFile を submit する。
/// 生成される preview は read-only で、この時点ではまだ書き込まない("No files have been changed")。
/// 実書き込みはこの後の proposal review(y/n gate)を通った時だけ。
pub(super) fn confirm_pending_agent_plan(state: &mut TuiState) -> Option<TuiSubmit> {
    if let Some(plan) = state.pending_agent_existing_edit_plan.take() {
        state.pending_agent_rationale = None;
        let message = TuiMessage::Action(format!(
            "Generating update preview...\nTarget: {}\nNo files have been changed.",
            redact_sensitive_text(&plan.target_path)
        ));
        state.active_context.push_tui_message(&message);
        state.messages.push(message);
        state.agent_generation = Some(TuiAgentGeneration {
            target_path: plan.target_path.clone(),
            kind: TuiPreviewKind::Update,
            started_at: Instant::now(),
            interruptible: true,
        });
        state.result_panel = None;
        state.write_progress = None;
        pin_chat_to_latest(state);
        return Some(TuiSubmit::Command(TuiCommand::AgentPropose {
            path: plan.target_path,
            instruction: plan.original_prompt,
        }));
    }

    let plan = state.pending_agent_new_file_plan.take()?;
    state.pending_agent_rationale = None;
    state.pending_agent_new_file_override = false;
    let message = TuiMessage::Action(format!(
        "Generating create preview...\nTarget: {}\nNo files have been changed.",
        redact_sensitive_text(&plan.target_path)
    ));
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    state.agent_generation = Some(TuiAgentGeneration {
        target_path: plan.target_path.clone(),
        kind: TuiPreviewKind::Create,
        started_at: Instant::now(),
        interruptible: true,
    });
    state.result_panel = None;
    state.write_progress = None;
    pin_chat_to_latest(state);
    Some(TuiSubmit::Command(TuiCommand::AgentProposeNewFile {
        path: plan.target_path,
        instruction: plan.original_prompt,
        context_files: plan.context_files,
    }))
}

/// 新規作成 plan の出力パスを composer で差し替えるモードに入る(action 1)。
/// 新規作成 plan が無ければ通常の composer 復帰にフォールバックする。
pub(super) fn enter_agent_target_override(state: &mut TuiState) -> Option<TuiSubmit> {
    if state.pending_agent_new_file_plan.is_none() {
        return return_agent_review_to_composer(state);
    }
    state.pending_agent_new_file_override = true;
    state.pending_agent_action_selection = 1;
    let message =
        TuiMessage::Action("Type a new relative output path, then press Enter.".to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
    None
}

/// 保留 plan を畳んで元の指示を composer に戻し、指示し直しを促す(既存編集時の action 1)。
/// provider は未呼び出しでファイルも不変。
pub(super) fn return_agent_review_to_composer(state: &mut TuiState) -> Option<TuiSubmit> {
    let prompt = state
        .pending_agent_existing_edit_plan
        .take()
        .map(|plan| plan.original_prompt)
        .or_else(|| {
            state
                .pending_agent_new_file_plan
                .take()
                .map(|plan| plan.original_prompt)
        });
    state.pending_agent_new_file_override = false;
    state.pending_agent_rationale = None;
    state.pending_agent_action_selection = 0;
    if let Some(prompt) = prompt {
        state.input = prompt;
        state.input_cursor = state.input.len();
    }
    let message = TuiMessage::Action(
        "Type a new instruction, then press Enter. No provider was called. No files were changed."
            .to_string(),
    );
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
    None
}

/// 現在の Agent review に対する操作ヒントを表示する(無効キーや範囲外選択時の案内)。
pub(super) fn push_agent_plan_action_hint(state: &mut TuiState) {
    let message = TuiMessage::Action(format_tui_agent_plan_action_hint_for_state(state));
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// composer で入力された新規ファイルパスを検証し、安全な NewFile の時だけ plan の target を差し替える。
/// 危険パス / 既存衝突 / 親ディレクトリ無しは理由付きで差し戻し、書き込みはしない。
/// new-file の narrow safe exception(single / safe / parent exists)をここでも崩さない安全境界。
pub(super) fn update_pending_agent_new_file_plan_path(
    state: &mut TuiState,
    input: &str,
) -> Vec<TuiMessage> {
    let Some(mut plan) = state.pending_agent_new_file_plan.take() else {
        return Vec::new();
    };
    let workspace = match Workspace::from_current_dir() {
        Ok(workspace) => workspace,
        Err(error) => {
            state.pending_agent_new_file_plan = Some(plan);
            return vec![TuiMessage::Error(format!(
                "Agent mode could not validate target path: {}",
                redact_sensitive_text(&error.to_string())
            ))];
        }
    };

    match agent_new_file_target_for_path(&workspace, input, &state.runtime.policy) {
        AgentPathTarget::NewFile { path } => {
            plan.target_path = path;
            let message = TuiMessage::Action(format!(
                "Agent review target updated: {}",
                redact_sensitive_text(&plan.target_path)
            ));
            state.pending_agent_new_file_plan = Some(plan);
            state.pending_agent_new_file_override = false;
            state.pending_agent_action_selection = 0;
            vec![message]
        }
        AgentPathTarget::Rejected { path, reason } | AgentPathTarget::External { path, reason } => {
            let category = agent_target_rejection_category(&reason);
            let message = TuiMessage::Error(format!(
                "Agent mode could not use `{}` as the new-file target: {}\n{}\nType another safe relative path, then press Enter. Press Esc to return to review.",
                redact_sensitive_text(&path),
                category,
                redact_sensitive_text(&reason),
            ));
            state.pending_agent_new_file_plan = Some(plan);
            vec![message]
        }
        AgentPathTarget::Included { path, .. } => {
            let message = TuiMessage::Error(format!(
                "Agent mode could not use `{}` as the new-file target: path already exists\nfile already exists\nType another safe relative path, then press Enter. Press Esc to return to review.",
                redact_sensitive_text(&path),
            ));
            state.pending_agent_new_file_plan = Some(plan);
            vec![message]
        }
        AgentPathTarget::Missing { path } => {
            let message = TuiMessage::Error(format!(
                "Agent mode could not use `{}` as the new-file target: parent directory missing\nparent directory does not exist\nType another safe relative path, then press Enter. Press Esc to return to review.",
                redact_sensitive_text(&path),
            ));
            state.pending_agent_new_file_plan = Some(plan);
            vec![message]
        }
    }
}

/// target 拒否理由を、ユーザー向けの短いカテゴリ文言に丸める表示用ヘルパ。
pub(super) fn agent_target_rejection_category(reason: &str) -> &'static str {
    if reason.contains("outside") {
        "outside current directory"
    } else if reason.contains("hidden") || reason.contains("secret-like") {
        "hidden/secret-like path"
    } else if reason.contains("already exists") {
        "path already exists"
    } else {
        "invalid path"
    }
}

/// パス差し替えモードを抜けて Agent review に戻る。plan 自体は保留のまま破棄しない。
pub(super) fn cancel_agent_target_override(state: &mut TuiState) {
    state.pending_agent_new_file_override = false;
    state.pending_agent_action_selection = 1;
    clear_tui_input(state);
    reset_suggestion_selection(state);
    let message =
        TuiMessage::Action("Target override cancelled. Agent review is still pending.".to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// 保留中の Agent plan を全て破棄し、cancelled 結果パネルを出す(action 2 / clarification の cancel)。
/// provider 未呼び出し・ファイル不変であることをメッセージでも明示する。
pub(super) fn cancel_pending_agent_plan(state: &mut TuiState) {
    state.pending_agent_existing_edit_plan = None;
    state.pending_agent_new_file_plan = None;
    state.pending_agent_destination_clarification = None;
    state.pending_agent_rationale = None;
    state.pending_agent_new_file_override = false;
    state.pending_agent_action_selection = 0;
    reset_selection_buffer(state);
    state.agent_generation = None;
    state.agent_propose_in_flight = None;
    state.write_progress = None;
    state.result_panel = Some(TuiResultPanel {
        kind: TuiResultKind::Cancelled,
        files: Vec::new(),
        reason: None,
    });
    let message = TuiMessage::Action(
        "Agent action cancelled. No provider was called. No files were changed.".to_string(),
    );
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// proposal review の操作ヒント(y save / n cancel / d diff / r revise、blocked 時は別文言)を表示する。
pub(super) fn push_proposal_review_action_hint(state: &mut TuiState) {
    let hint = if is_proposal_apply_blocked(state) {
        "Use n to cancel, d to show diff, r to revise, or c to clear."
    } else {
        "Use y to save, n to cancel, d to show diff, or r to revise."
    };
    let message = TuiMessage::Action(hint.to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// proposal review を拒否し、全保留(proposal / apply / review)を破棄して cancelled にする。
/// permission gate の `n` / 非 save Enter はここに来る = 書き込みではなく reject。
pub(super) fn reject_pending_proposal_review(state: &mut TuiState) {
    state.pending_proposal = None;
    state.pending_apply = None;
    state.proposal_review = None;
    reset_advanced_reveal(state);
    state.write_progress = None;
    state.result_panel = Some(TuiResultPanel {
        kind: TuiResultKind::Cancelled,
        files: Vec::new(),
        reason: None,
    });
    clear_tui_input(state);
    reset_suggestion_selection(state);
    refresh_status(state);
    let message = TuiMessage::Action("Cancelled\n\nNo files were changed.\n\nReady.".to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// blocked な proposal を片付ける(blocked 時の `c`)。書き込みはせず cancelled 表示にして状態を初期化する。
pub(super) fn clear_pending_proposal_review(state: &mut TuiState) {
    state.pending_proposal = None;
    state.pending_apply = None;
    state.proposal_review = None;
    reset_advanced_reveal(state);
    state.write_progress = None;
    state.result_panel = Some(TuiResultPanel {
        kind: TuiResultKind::Cancelled,
        files: Vec::new(),
        reason: None,
    });
    clear_tui_input(state);
    reset_suggestion_selection(state);
    refresh_status(state);
    let message = TuiMessage::Action("Cancelled\n\nNo files were changed.\n\nReady.".to_string());
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// apply が blocked の理由を提示する(blocked 中に y/Enter を押した時など)。
/// non-git workspace は safe single new-file のみ対応である旨の注記を添え、ファイルは変えない。
pub(super) fn push_apply_blocked_message(state: &mut TuiState) {
    let reason = state
        .proposal_review
        .as_ref()
        .and_then(|review| match &review.apply_status {
            TuiProposalApplyStatus::Blocked { reason } => Some(reason.clone()),
            TuiProposalApplyStatus::Available => None,
        })
        .unwrap_or_else(|| "git-backed apply is unavailable".to_string());
    let detail = if reason == "this workspace is not a Git repository" {
        "\nNon-git apply currently supports safe single new-file proposals only."
    } else {
        ""
    };
    state.write_progress = None;
    state.result_panel = Some(TuiResultPanel {
        kind: TuiResultKind::Blocked,
        files: Vec::new(),
        reason: Some(reason.clone()),
    });
    let message = TuiMessage::Error(format!(
        "Blocked\n\nReason:\n- {}{detail}\n\nNo files were changed.\n\nStatus: blocked",
        redact_sensitive_text(&reason)
    ));
    state.active_context.push_tui_message(&message);
    state.messages.push(message);
    pin_chat_to_latest(state);
}

/// revision feedback を元に修正版 proposal を作り直す(review の `r` 入力後の Enter)。
/// manual 由来は /propose 再実行を案内するに留め、Agent 由来は元指示に feedback を足して
/// AgentPropose / AgentProposeNewFile を再 submit する。いずれも承認前なのでファイルは変えない。
pub(super) fn revise_pending_proposal_from_feedback(
    state: &mut TuiState,
    feedback: String,
) -> Option<TuiSubmit> {
    let Some(review) = state.proposal_review.clone() else {
        return None;
    };

    match review.origin {
        TuiProposalReviewOrigin::Manual => {
            if let Some(review) = state.proposal_review.as_mut() {
                review.mode = TuiProposalReviewMode::Review;
            }
            let message = TuiMessage::Error(
                "Revision is available for Agent proposals. For manual proposals, run /propose again with the updated instruction."
                    .to_string(),
            );
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
            pin_chat_to_latest(state);
            None
        }
        TuiProposalReviewOrigin::AgentExisting {
            target_path,
            original_prompt,
        } => {
            state.proposal_review = None;
            state.pending_proposal = None;
            refresh_status(state);
            let message = TuiMessage::Action(
                "Revision requested.\nCreating a revised pending proposal...\nNo files will be changed before approval."
                    .to_string(),
            );
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
            pin_chat_to_latest(state);
            Some(TuiSubmit::Command(TuiCommand::AgentPropose {
                path: target_path,
                instruction: format!("{original_prompt}\n\nRevision feedback:\n{feedback}"),
            }))
        }
        TuiProposalReviewOrigin::AgentNewFile {
            target_path,
            original_prompt,
            context_files,
        } => {
            state.proposal_review = None;
            state.pending_proposal = None;
            refresh_status(state);
            let message = TuiMessage::Action(
                "Revision requested.\nCreating a revised pending proposal...\nNo files will be changed before approval."
                    .to_string(),
            );
            state.active_context.push_tui_message(&message);
            state.messages.push(message);
            pin_chat_to_latest(state);
            Some(TuiSubmit::Command(TuiCommand::AgentProposeNewFile {
                path: target_path,
                instruction: format!("{original_prompt}\n\nRevision feedback:\n{feedback}"),
                context_files,
            }))
        }
    }
}

/// composer に改行を挿入する(Ctrl+J)。pending apply 中は Enter が承認入力なので、
/// その間は改行を入れず承認フローを邪魔しない。
pub(super) fn insert_tui_newline(input: &mut String, cursor: &mut usize, has_pending_apply: bool) {
    if !has_pending_apply {
        *cursor = clamp_to_char_boundary(input, *cursor);
        input.insert(*cursor, '\n');
        *cursor = (*cursor).saturating_add(1);
    }
}
