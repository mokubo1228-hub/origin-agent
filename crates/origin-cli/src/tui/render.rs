//! パネル描画と user-facing な行ビルダ。
//! 表示は read-only とし、permission gate や worker 状態の変更は input/workflow 側に置く。
use super::*;

/// frame 全体を header / chat / interaction の3ペインに縦分割して描く最上位の描画入口。
/// ここは read-only:状態は読むだけで、permission gate や worker 状態は触らない。
pub(super) fn render(frame: &mut Frame<'_>, state: &TuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height(
                &state.status,
                state.pending_apply.is_some(),
                state.interaction_mode,
            )),
            Constraint::Min(5),
            Constraint::Length(interaction_pane_height(state)),
        ])
        .split(frame.area());

    frame.render_widget(
        header_pane(
            &state.status,
            state.pending_apply.is_some(),
            state.interaction_mode,
        ),
        chunks[0],
    );
    frame.render_widget(
        chat_pane(
            state,
            &state.status,
            &state.messages,
            false,
            state.chat_scroll_offset,
            chunks[1].width,
            chunks[1].height,
        ),
        chunks[1],
    );
    frame.render_widget(interaction_pane(state, chunks[2].width), chunks[2]);
    frame.set_cursor_position(composer_cursor_position(state, chunks[2]));
}

/// render と同じ3分割を再計算し、中央の chat ペイン Rect だけ返す。
/// scroll 計算など「chat 領域の寸法」が要る側が、レイアウトを揃えて参照するためのヘルパ。
pub(super) fn chat_area(area: Rect, state: &TuiState) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height(
                &state.status,
                state.pending_apply.is_some(),
                state.interaction_mode,
            )),
            Constraint::Min(5),
            Constraint::Length(interaction_pane_height(state)),
        ])
        .split(area);
    chunks[1]
}

/// header 行を Paragraph に包むだけのラッパ。
pub(super) fn header_pane(
    status: &StatusSnapshot,
    has_pending_apply: bool,
    interaction_mode: TuiInteractionMode,
) -> Paragraph<'static> {
    Paragraph::new(header_lines(status, has_pending_apply, interaction_mode))
}

/// header の行数をそのまま高さ(行)として返す。レイアウト制約に使う。
pub(super) fn header_height(
    status: &StatusSnapshot,
    has_pending_apply: bool,
    interaction_mode: TuiInteractionMode,
) -> u16 {
    header_lines(status, has_pending_apply, interaction_mode).len() as u16
}

/// chat 本文ペインを組む。model route picker > 空状態 > 通常メッセージ の優先で描き分ける。
pub(super) fn chat_pane(
    state: &TuiState,
    _status: &StatusSnapshot,
    messages: &[TuiMessage],
    loading: bool,
    scroll_offset: usize,
    visible_width: u16,
    visible_height: u16,
) -> Paragraph<'static> {
    if let Some(picker) = &state.model_route_picker {
        return Paragraph::new(model_route_picker_visible_lines(
            &picker.routes,
            picker.selected,
            visible_width,
            visible_height,
        ))
        .wrap(Wrap { trim: false });
    }

    if messages.is_empty() && !loading {
        return Paragraph::new(empty_state_lines(visible_height)).wrap(Wrap { trim: false });
    }
    Paragraph::new(chat_visible_lines(
        messages,
        loading,
        visible_width,
        visible_height,
        scroll_offset,
    ))
}

/// 空状態に出す origin マスコットの ASCII art。
pub(super) const ORIGIN_MASCOT: &[&str] = &[
    "                     █▄█",
    "             ▄▄▄      ██",
    "           ▄██       ██▀",
    "         ▄█████▄   ▄██▀",
    "      ▄▄█████████████▀",
    "  ▄█████████████▀▀▀█▀",
    "▄███████████████▀▝ █",
    "████████████████▄▄▄▀",
    " ████████████▀███▀██▄▄",
    "  ▀▀▀ ██▀ ▀▀▀▀▄",
];

/// メッセージが1件も無い時の初期画面。十分な高さがある時だけマスコットを添える。
pub(super) fn empty_state_lines(visible_height: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // 縦の余白が足りない端末ではマスコットを省く(+6 は前後の空行とコピー行ぶん)。
    let show_mascot = visible_height >= ORIGIN_MASCOT.len() as u16 + 6;

    if show_mascot {
        lines.push(Line::from(""));
        for line in ORIGIN_MASCOT {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                Style::default().fg(Color::Cyan),
            )));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from("  origin-agent — local-first AI workbench"));
    lines.push(Line::from(""));
    lines.push(style::footer_hint("Type a message to start."));
    lines
}

/// 下部の操作ペイン(command palette / review / composer / footer)を Paragraph に包む。
pub(super) fn interaction_pane(state: &TuiState, width: u16) -> Paragraph<'static> {
    Paragraph::new(interaction_lines(state, width))
}

/// 下部ペインの総高さを、表示中の構成要素(palette・review・composer 枠・本文・footer)の
/// 行数を足して算出する。render の Length 制約と一致させるため、計算をここに集約する。
pub(super) fn interaction_pane_height(state: &TuiState) -> u16 {
    let composer_lines = composer_line_count(&state.input).min(MAX_INPUT_TEXT_LINES);
    let footer_lines = 1;
    let command_palette_lines = command_palette_line_count(state);
    let review_panel_lines = review_panel_line_count(state, 80);
    command_palette_lines
        + review_panel_lines
        + COMPOSER_BORDER_LINES
        + composer_lines
        + footer_lines
}

/// slash 候補パレットが占める行数(タイトル1行+候補行)。非表示なら0。
pub(super) fn command_palette_line_count(state: &TuiState) -> u16 {
    if should_show_slash_suggestions(state) {
        let suggestion_lines = slash_suggestions(
            &state.input,
            state.pending_proposal.is_some(),
            state.pending_apply.is_some(),
        )
        .len()
        .max(1) as u16;
        1 + suggestion_lines
    } else {
        0
    }
}

/// 下部ペインの全行を順に積む:slash 候補 → review パネル → composer → footer ヒント。
pub(super) fn interaction_lines(state: &TuiState, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if should_show_slash_suggestions(state) {
        lines.extend(command_palette_lines(state, width));
    }

    lines.extend(review_panel_lines(state, width));
    let (affordance, is_live_composer) = composer_affordance(state);
    lines.extend(composer_cell_lines_with_placeholder(
        &state.input,
        width,
        composer_placeholder(state),
        affordance,
        is_live_composer,
    ));
    lines.push(style::footer_hint(footer_text(
        &state.input,
        state.pending_proposal.is_some(),
        state.pending_apply.is_some(),
        is_proposal_review_active(state),
        is_proposal_revision_input_active(state),
        is_proposal_apply_blocked(state),
        state.agent_generation.is_some(),
        state.write_progress.is_some(),
        state.chat_in_flight.is_some(),
        state.model_route_picker.is_some(),
        has_pending_agent_plan(state),
        state.pending_agent_new_file_override,
        state.show_advanced_actions,
        state.interaction_mode,
    )));
    lines
}

/// slash 候補ブロック。proposal 保留中はタイトルを "Next actions" に切り替える。
pub(super) fn command_palette_lines(state: &TuiState, width: u16) -> Vec<Line<'static>> {
    let has_pending_proposal = state.pending_proposal.is_some();
    let title = if has_pending_proposal {
        "Next actions"
    } else {
        "Commands"
    };
    let mut lines = vec![style::footer_hint(title)];
    lines.extend(slash_suggestion_lines(
        &state.input,
        has_pending_proposal,
        state.pending_apply.is_some(),
        state.suggestion_selection,
        width,
    ));
    lines
}

/// review パネルの行数(高さ計算用)。実体の行を組んで数えるだけ。
pub(super) fn review_panel_line_count(state: &TuiState, width: u16) -> u16 {
    review_panel_lines(state, width).len() as u16
}

/// composer 直上に出す単一パネルを、状態の優先順で1つだけ選ぶ。in-flight(planning /
/// generation / chat / write)を最優先し、次に proposal review・agent plan・clarification・
/// 結果の順。どれにも該当しなければ空(パネル無し)。表示の選択のみで状態は変えない。
pub(super) fn review_panel_lines(state: &TuiState, width: u16) -> Vec<Line<'static>> {
    if let Some(in_flight) = state.agent_plan_in_flight.as_ref() {
        return agent_planning_panel_lines(in_flight.started_at, width);
    }

    if let Some(generation) = state.agent_generation.as_ref() {
        let can_interrupt = generation.interruptible && state.agent_propose_in_flight.is_some();
        return agent_generation_panel_lines(generation, can_interrupt, width);
    }

    if let Some(in_flight) = state.chat_in_flight.as_ref() {
        return chat_generation_panel_lines(in_flight.started_at, width);
    }

    if let Some(progress) = state.write_progress.as_ref() {
        return write_progress_panel_lines(progress, width);
    }

    if let Some(review) = state.proposal_review.as_ref() {
        if state.pending_proposal.is_some() {
            return proposal_review_panel_lines(
                state.pending_proposal.as_ref().expect("checked above"),
                review,
                state.show_advanced_actions,
                width,
            );
        }
    }

    if let Some(plan) = state.pending_agent_new_file_plan.as_ref() {
        return agent_new_file_review_panel_lines(
            plan,
            state.pending_agent_rationale.as_deref(),
            state.pending_agent_new_file_override,
            state.pending_agent_action_selection,
            width,
        );
    }

    if let Some(clarification) = state.pending_agent_destination_clarification.as_ref() {
        return agent_destination_clarification_panel_lines(
            clarification,
            state.pending_agent_rationale.as_deref(),
            state.pending_agent_action_selection,
            width,
        );
    }

    if let Some(plan) = state.pending_agent_existing_edit_plan.as_ref() {
        return agent_existing_edit_review_panel_lines(
            plan,
            state.pending_agent_rationale.as_deref(),
            state.pending_agent_action_selection,
            width,
        );
    }

    if let Some(result) = state.result_panel.as_ref() {
        return result_panel_lines(result, width);
    }

    Vec::new()
}

/// スピナー・ラベル・経過秒(+任意 hint)を1行にまとめた進捗インジケータ行。
pub(super) fn working_indicator_line(
    label: &str,
    started_at: Instant,
    hint: Option<&str>,
    _width: u16,
) -> style::BodyLine {
    let marker = spinner_marker(started_at);
    let elapsed = started_at.elapsed().as_secs();
    let suffix = hint
        .map(|hint| format!("{elapsed}s • {hint}"))
        .unwrap_or_else(|| format!("{elapsed}s"));
    style::BodyLine::Accent(format!("{marker} {label} ({suffix})"))
}

/// 経過時間から 250ms 周期で円スピナー文字を選ぶアニメ用の純粋関数。
pub(super) fn spinner_marker(started_at: Instant) -> &'static str {
    match (started_at.elapsed().as_millis() / 250) % 4 {
        0 => "○",
        1 => "◔",
        2 => "◑",
        _ => "◕",
    }
}

/// 既存ファイル更新の review パネル。まだ書き込みは起きておらず、preview 生成か cancel を
/// 選ぶ段。"No files will be changed yet." を明示して permission gate の手前であることを示す。
pub(super) fn agent_existing_edit_review_panel_lines(
    plan: &PendingAgentExistingEditPlan,
    rationale: Option<&str>,
    selected_action: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let mut body = vec![
        style::BodyLine::Text("Update file preview".to_string()),
        style::BodyLine::Blank,
        style::BodyLine::Muted("Update:".to_string()),
        style::BodyLine::Text(format!("  {}", redact_sensitive_text(&plan.target_path))),
        style::BodyLine::Muted(format!(
            "Using: {}",
            compact_context_description(&plan.read_context_description)
        )),
        style::BodyLine::Blank,
    ];
    push_agent_rationale_lines(&mut body, rationale);
    body.extend([
        style::BodyLine::Text("This will generate an update preview.".to_string()),
        style::BodyLine::Text("No files will be changed yet.".to_string()),
        style::BodyLine::Blank,
    ]);
    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Neutral,
            title: "Choose next action".to_string(),
            body,
            actions: Some(style::numbered_action_list(
                &[
                    "Generate update preview",
                    "Other / custom instruction",
                    "Cancel",
                ],
                selected_action,
            )),
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// 新規ファイル作成の review パネル。override 中は出力パスの手入力欄に切り替え、通常は
/// preview 生成 / パス編集 / cancel の番号メニューを出す。ここでも書き込みはまだ起きない。
pub(super) fn agent_new_file_review_panel_lines(
    plan: &PendingAgentNewFilePlan,
    rationale: Option<&str>,
    is_override: bool,
    selected_action: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let mut body = vec![
        style::BodyLine::Text("Create file preview".to_string()),
        style::BodyLine::Blank,
        style::BodyLine::Muted("Create:".to_string()),
        style::BodyLine::Text(format!("  {}", redact_sensitive_text(&plan.target_path))),
        style::BodyLine::Muted(format!(
            "Using: {}",
            context_files_summary(&plan.context_files)
        )),
        style::BodyLine::Blank,
    ];
    push_agent_rationale_lines(&mut body, rationale);
    body.extend([
        style::BodyLine::Text("This will generate a create preview.".to_string()),
        style::BodyLine::Text("No files will be changed yet.".to_string()),
        style::BodyLine::Blank,
    ]);
    let actions = if is_override {
        body.push(style::BodyLine::Text(
            "Type a new relative output path, then press Enter.".to_string(),
        ));
        None
    } else {
        Some(style::numbered_action_list(
            &["Generate create preview", "Edit output path", "Cancel"],
            selected_action,
        ))
    };

    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Neutral,
            title: "Choose next action".to_string(),
            body,
            actions,
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// 行き先を決められない時の destination clarification パネル。preview も書き込みも無いまま、
/// 「どこへ向かうか」を選ぶ固定選択肢を出す。
pub(super) fn agent_destination_clarification_panel_lines(
    clarification: &PendingAgentDestinationClarification,
    rationale: Option<&str>,
    selected_action: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let mut body = vec![
        style::BodyLine::Text("Clarification needed".to_string()),
        style::BodyLine::Blank,
        style::BodyLine::Muted("Question:".to_string()),
        style::BodyLine::Text(format!(
            "  {}",
            redact_sensitive_text(&clarification.question)
        )),
        style::BodyLine::Blank,
    ];
    push_agent_rationale_lines(&mut body, rationale);
    body.extend([
        style::BodyLine::Text("No provider preview was generated.".to_string()),
        style::BodyLine::Text("No files have been changed.".to_string()),
        style::BodyLine::Blank,
    ]);
    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Neutral,
            title: "Choose destination".to_string(),
            body,
            actions: Some(style::numbered_action_list(
                &[
                    "Specify existing file for update preview",
                    "Create new Markdown preview",
                    "Other / custom instruction",
                    "Cancel",
                ],
                selected_action,
            )),
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// rationale があれば redact して body に追記する。空・空白なら何も足さない。
pub(super) fn push_agent_rationale_lines(body: &mut Vec<style::BodyLine>, rationale: Option<&str>) {
    let Some(rationale) = rationale.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    body.push(style::BodyLine::Muted("Rationale:".to_string()));
    body.push(style::BodyLine::Text(format!(
        "  {}",
        redact_sensitive_text(rationale)
    )));
    body.push(style::BodyLine::Blank);
}

/// 参照 context の説明を短くする。4件以上はパスを並べず件数表記にして横幅を抑える。
pub(super) fn compact_context_description(description: &str) -> String {
    let trimmed = description.trim();
    if trimmed.is_empty() {
        return "current file".to_string();
    }
    let count = trimmed
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .count();
    if count > 3 {
        format!("{count} context files")
    } else {
        redact_sensitive_text(trimmed)
    }
}

/// 参照ファイル群の1行要約。0件は "safe context"、1件はパス、複数は件数。
pub(super) fn context_files_summary(files: &[origin_tools::ReadFile]) -> String {
    match files {
        [] => "safe context".to_string(),
        [file] => redact_sensitive_text(&file.relative_path),
        _ => format!("{} context files", files.len()),
    }
}

/// preview 生成中の進捗パネル。interrupt 可能な時だけ "esc/Ctrl+C to interrupt" を出し
/// (文言と実挙動を一致させる)、save approval までファイルは変わらない旨を明示する。
pub(super) fn agent_generation_panel_lines(
    generation: &TuiAgentGeneration,
    can_interrupt: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let title = match generation.kind {
        TuiPreviewKind::Create => "Generating create preview",
        TuiPreviewKind::Update => "Generating update preview",
        TuiPreviewKind::Generic => "Generating preview",
    };
    let hint = if can_interrupt {
        Some("esc/Ctrl+C to interrupt")
    } else {
        None
    };
    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Active,
            title: title.to_string(),
            body: vec![
                working_indicator_line(title, generation.started_at, hint, width),
                style::BodyLine::Blank,
                style::BodyLine::Text(format!(
                    "Target: {}",
                    redact_sensitive_text(&generation.target_path)
                )),
                style::BodyLine::Blank,
                style::BodyLine::Text("Waiting for provider...".to_string()),
                style::BodyLine::Text("No files will be changed until save approval.".to_string()),
            ],
            actions: None,
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// Agent action を planning 中の進捗パネル。provider 応答待ちで、ファイルは変わらない。
pub(super) fn agent_planning_panel_lines(started_at: Instant, width: u16) -> Vec<Line<'static>> {
    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Active,
            title: "Planning Agent action".to_string(),
            body: vec![
                working_indicator_line(
                    "planning...",
                    started_at,
                    Some("esc/Ctrl+C to interrupt"),
                    width,
                ),
                style::BodyLine::Blank,
                style::BodyLine::Text("Waiting for provider...".to_string()),
                style::BodyLine::Text("No files will be changed.".to_string()),
            ],
            actions: None,
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// 通常 chat の model 応答待ちパネル。ファイル不変で、draft は編集可・Enter は待つ旨を示す。
pub(super) fn chat_generation_panel_lines(started_at: Instant, width: u16) -> Vec<Line<'static>> {
    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Active,
            title: "Waiting for model".to_string(),
            body: vec![
                working_indicator_line("Waiting for model", started_at, None, width),
                style::BodyLine::Blank,
                style::BodyLine::Text("No files will be changed.".to_string()),
                style::BodyLine::Muted("Draft editable · Enter waits".to_string()),
            ],
            actions: None,
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// 保存(check → write)進行中パネル。検証(git apply --check)を通った時だけ変わる旨を明示する。
pub(super) fn write_progress_panel_lines(
    progress: &TuiWriteProgress,
    width: u16,
) -> Vec<Line<'static>> {
    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Active,
            title: "Saving".to_string(),
            body: vec![
                working_indicator_line("Checking patch", progress.started_at, None, width),
                working_indicator_line("Writing file", progress.started_at, None, width),
                style::BodyLine::Blank,
                style::BodyLine::Text(format!(
                    "Target: {}",
                    redact_sensitive_text(&progress.target_summary)
                )),
                style::BodyLine::Blank,
                style::BodyLine::Text(
                    "The file will change only if validation passes.".to_string(),
                ),
            ],
            actions: None,
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// proposal の permission gate パネル。generic な action 選択ではなく save / create / update の
/// 承認 UI として扱う。blocked なら専用パネルへ委譲し、それ以外は preview 種別に応じた文言と
/// y/n + Up/Down + Enter の承認メニューを出す。RevisionInput 時は承認ではなく修正入力に切替。
pub(super) fn proposal_review_panel_lines(
    pending: &PendingProposal,
    review: &TuiProposalReview,
    show_advanced: bool,
    width: u16,
) -> Vec<Line<'static>> {
    if let Some(lines) = blocked_proposal_panel_lines(pending, review, show_advanced, width) {
        return lines;
    }

    let kind = proposal_preview_kind(pending, review);
    let title = match kind {
        TuiPreviewKind::Create => "Create preview ready",
        TuiPreviewKind::Update => "Update preview ready",
        TuiPreviewKind::Generic => "Preview ready",
    };
    let target_label = match kind {
        TuiPreviewKind::Create => "Create",
        TuiPreviewKind::Update => "Update",
        TuiPreviewKind::Generic => "Target",
    };
    let save_question = match kind {
        TuiPreviewKind::Create => "Save this file?",
        TuiPreviewKind::Update => "Save this update?",
        TuiPreviewKind::Generic => "Save this change?",
    };
    let save_label = match kind {
        TuiPreviewKind::Create => "Create file",
        TuiPreviewKind::Update => "Update file",
        TuiPreviewKind::Generic => "Save change",
    };
    let effect_line = match kind {
        TuiPreviewKind::Create => "This will create one file.",
        TuiPreviewKind::Update => "This will update one file.",
        TuiPreviewKind::Generic => "This will save the previewed change.",
    };

    let mut body = vec![
        style::BodyLine::Text(format!(
            "{target_label}: {}",
            safe_paths_summary(&pending.requested_files)
        )),
        style::BodyLine::Blank,
        style::BodyLine::Text(effect_line.to_string()),
        style::BodyLine::Text("No other files will be changed.".to_string()),
        style::BodyLine::Blank,
        style::BodyLine::Text(save_question.to_string()),
        style::BodyLine::Blank,
    ];
    let (actions, advanced) = match review.mode {
        TuiProposalReviewMode::Review => (
            Some(style::permission_action_list(
                Some(save_label),
                review.selected_action,
            )),
            vec![
                style::BodyLine::Muted("d diff".to_string()),
                style::BodyLine::Muted("r revise".to_string()),
            ],
        ),
        TuiProposalReviewMode::RevisionInput => {
            body.push(style::BodyLine::Text(
                "Type revision feedback, then press Enter.".to_string(),
            ));
            (None, Vec::new())
        }
    };

    style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Neutral,
            title: title.to_string(),
            body,
            actions,
            advanced,
            show_advanced,
        },
        width,
    )
}

/// apply がブロックされている(検証失敗など)時だけ Some を返す blocked パネル。
/// 理由を redact して見せ、ファイルは変えていないことを明示する。承認には進めない。
pub(super) fn blocked_proposal_panel_lines(
    pending: &PendingProposal,
    review: &TuiProposalReview,
    show_advanced: bool,
    width: u16,
) -> Option<Vec<Line<'static>>> {
    let reason = match &review.apply_status {
        TuiProposalApplyStatus::Blocked { reason } => Some(reason.clone()),
        TuiProposalApplyStatus::Available => match &pending.check_state {
            PendingCheckState::Failed(reason) => Some(reason.clone()),
            _ => None,
        },
    }?;
    let mut advanced = vec![
        style::BodyLine::Muted("d diff".to_string()),
        style::BodyLine::Muted("r revise".to_string()),
    ];
    if matches!(review.apply_status, TuiProposalApplyStatus::Blocked { .. }) {
        advanced.push(style::BodyLine::Muted("c clear".to_string()));
    }
    Some(style::render_panel(
        &style::PanelView {
            tone: style::PanelTone::Blocked,
            title: "Blocked".to_string(),
            body: vec![
                style::BodyLine::Text("Reason:".to_string()),
                style::BodyLine::Text(format!("- {}", redact_sensitive_text(&reason))),
                style::BodyLine::Blank,
                style::BodyLine::Text("No files were changed.".to_string()),
                style::BodyLine::Text("Status: blocked".to_string()),
                style::BodyLine::Blank,
            ],
            actions: Some(style::permission_action_list(None, 0)),
            advanced,
            show_advanced,
        },
        width,
    ))
}

/// proposal を Create / Update / Generic のどれとして見せるか判定する。
/// Agent 由来は origin から確定し、Manual は diff の形から推定する。
pub(super) fn proposal_preview_kind(
    pending: &PendingProposal,
    review: &TuiProposalReview,
) -> TuiPreviewKind {
    match &review.origin {
        TuiProposalReviewOrigin::AgentNewFile { .. } => TuiPreviewKind::Create,
        TuiProposalReviewOrigin::AgentExisting { .. } => TuiPreviewKind::Update,
        TuiProposalReviewOrigin::Manual => {
            // `--- /dev/null` を含む diff は新規作成(create)とみなす。
            if pending
                .diff
                .lines()
                .any(|line| line.trim() == "--- /dev/null")
            {
                TuiPreviewKind::Create
            } else if pending.requested_files.len() == 1 {
                TuiPreviewKind::Update
            } else {
                TuiPreviewKind::Generic
            }
        }
    }
}

/// 操作後の結果パネル(created / updated / cancelled / interrupted / blocked)。
/// 種別ごとに tone と本文を変え、変更ファイルや「他は変えていない」旨を示す。
pub(super) fn result_panel_lines(result: &TuiResultPanel, width: u16) -> Vec<Line<'static>> {
    let title = match result.kind {
        TuiResultKind::Created => "Created",
        TuiResultKind::Updated => "Updated",
        TuiResultKind::Cancelled => "Cancelled",
        TuiResultKind::Interrupted => "Interrupted",
        TuiResultKind::Blocked => "Blocked",
    };
    let tone = match result.kind {
        TuiResultKind::Created | TuiResultKind::Updated => style::PanelTone::Ready,
        TuiResultKind::Cancelled | TuiResultKind::Interrupted => style::PanelTone::Dim,
        TuiResultKind::Blocked => style::PanelTone::Blocked,
    };
    let mut body = Vec::new();
    match result.kind {
        TuiResultKind::Created | TuiResultKind::Updated => {
            body.push(style::BodyLine::Text("File:".to_string()));
            for file in &result.files {
                body.push(style::BodyLine::Text(format!(
                    "- {}",
                    redact_sensitive_text(file)
                )));
            }
            body.push(style::BodyLine::Blank);
            body.push(style::BodyLine::Text(
                "No other files were changed.".to_string(),
            ));
            body.push(style::BodyLine::Blank);
            body.push(style::BodyLine::Text("Ready.".to_string()));
        }
        TuiResultKind::Cancelled | TuiResultKind::Interrupted => {
            body.push(style::BodyLine::Text("No files were changed.".to_string()));
            body.push(style::BodyLine::Blank);
            body.push(style::BodyLine::Text("Ready.".to_string()));
        }
        TuiResultKind::Blocked => {
            body.push(style::BodyLine::Text("Reason:".to_string()));
            if let Some(reason) = &result.reason {
                body.push(style::BodyLine::Text(format!(
                    "- {}",
                    redact_sensitive_text(reason)
                )));
            } else {
                body.push(style::BodyLine::Text("- blocked".to_string()));
            }
            body.push(style::BodyLine::Blank);
            body.push(style::BodyLine::Text("No files were changed.".to_string()));
            body.push(style::BodyLine::Blank);
            body.push(style::BodyLine::Text("Status: blocked".to_string()));
        }
    }
    style::render_panel(
        &style::PanelView {
            tone,
            title: title.to_string(),
            body,
            actions: None,
            advanced: Vec::new(),
            show_advanced: false,
        },
        width,
    )
}

/// 対象パス群の1行要約。0件は "(none)"、1件はパス、複数は "先頭 +残数"。
pub(super) fn safe_paths_summary(paths: &[String]) -> String {
    match paths {
        [] => "(none)".to_string(),
        [path] => redact_sensitive_text(path),
        [first, rest @ ..] => format!("{} +{}", redact_sensitive_text(first), rest.len()),
    }
}

/// composer の placeholder。revision 入力・新規パス上書きなど、現在の入力の意味に合わせて出す。
pub(super) fn composer_placeholder(state: &TuiState) -> &'static str {
    if is_proposal_revision_input_active(state) {
        "Revision feedback"
    } else if state.pending_agent_new_file_override {
        "New relative output path"
    } else if state.model_route_picker.is_some()
        || is_proposal_apply_blocked(state)
        || is_proposal_review_active(state)
        || has_pending_agent_plan(state)
    {
        ""
    } else if state.agent_plan_in_flight.is_some()
        || state.agent_generation.is_some()
        || state.write_progress.is_some()
    {
        "Message or /command"
    } else if state.result_panel.is_some() {
        "Message or /command"
    } else {
        "Message or /command"
    }
}

/// composer 直下の操作ヒントと「ライブ composer か」フラグを、状態の優先順で1つ選ぶ。
/// interrupt 系の文言は、実際に Esc/Ctrl+C が効く状態にだけ付ける。
pub(super) fn composer_affordance(state: &TuiState) -> (&'static str, bool) {
    if state.model_route_picker.is_some() {
        ("Selecting model · Up/Down · number · Enter · Esc", false)
    } else if is_proposal_apply_blocked(state) {
        if state.show_advanced_actions {
            ("Blocked review · n cancel · d/r/c · ? hide", false)
        } else {
            ("Blocked review · n cancel · ? more actions", false)
        }
    } else if is_proposal_review_active(state) && !is_proposal_revision_input_active(state) {
        if state.show_advanced_actions {
            ("Reviewing save · y/n · Up/Down · Enter · ? hide", false)
        } else {
            (
                "Reviewing save · y/n · Up/Down · Enter · ? more actions",
                false,
            )
        }
    } else if state.agent_plan_in_flight.is_some() {
        ("Planning · Esc/Ctrl+C interrupt", true)
    } else if has_pending_agent_plan(state) && !state.pending_agent_new_file_override {
        ("Choose action · number · Up/Down · Enter · Esc", false)
    } else if is_proposal_revision_input_active(state) {
        ("Type text · Enter submit · Esc cancel", true)
    } else if state.pending_agent_new_file_override {
        ("Type path · Enter submit · Esc cancel", true)
    } else {
        ("Message or /command", true)
    }
}

/// 端末カーソルの実座標を、上に積まれた palette / review の行数と composer 枠ぶん下げて算出する。
pub(super) fn composer_cursor_position(state: &TuiState, interaction_area: Rect) -> Position {
    let palette_lines = command_palette_line_count(state);
    let review_lines = review_panel_line_count(state, interaction_area.width);
    let cursor = composer_cursor_offset(&state.input, state.input_cursor);
    let max_x = interaction_area.width.saturating_sub(2).max(1);
    // composer は palette/review の下・枠の内側にあるので、その行数+枠ぶん(+2)下げて入力行頭に合わせる。
    Position {
        x: interaction_area.x.saturating_add(cursor.x.min(max_x)),
        y: interaction_area
            .y
            .saturating_add(palette_lines)
            .saturating_add(review_lines)
            .saturating_add(2)
            .saturating_add(cursor.y),
    }
}

/// 最下行の操作ヒント。多数の TUI 状態を優先順に評価し、今アクティブな状態の案内を1つ返す。
pub(super) fn footer_text(
    input: &str,
    has_pending_proposal: bool,
    has_pending_apply: bool,
    has_proposal_review: bool,
    has_proposal_revision_input: bool,
    has_proposal_apply_blocked: bool,
    has_agent_generation: bool,
    has_write_progress: bool,
    has_chat_in_flight: bool,
    has_model_route_picker: bool,
    has_pending_agent_plan: bool,
    has_agent_target_override: bool,
    advanced_revealed: bool,
    interaction_mode: TuiInteractionMode,
) -> &'static str {
    if has_model_route_picker {
        "Up/Down select · Enter use route · Esc cancel"
    } else if has_pending_apply {
        "Approval: y/yes apply | Enter/n/no/other reject | Ctrl+C quit"
    } else if has_agent_generation {
        "Generating preview · waiting for provider"
    } else if has_write_progress {
        "Saving · checking and writing"
    } else if has_proposal_revision_input {
        "Revision: type feedback · Enter revise proposal · Esc reject"
    } else if has_proposal_apply_blocked {
        if advanced_revealed {
            "Blocked · n cancel · d diff · r revise · ? hide"
        } else {
            "Blocked · n cancel · ? more actions"
        }
    } else if has_proposal_review {
        if advanced_revealed {
            "Up/Down select · Enter confirm · y/n shortcuts · ? hide"
        } else {
            "Up/Down select · Enter confirm · y/n shortcuts · ? more actions"
        }
    } else if has_agent_target_override {
        "Target override: Enter validate · Esc return to review"
    } else if has_pending_agent_plan {
        "Type a number or Up/Down · Enter confirm · Esc cancel"
    } else if has_chat_in_flight {
        "Waiting for model · draft editable · Enter waits · Ctrl+C quit"
    } else if input.trim_start().starts_with('/') && !input.contains('\n') {
        if slash_suggestions(input, has_pending_proposal, has_pending_apply).is_empty() {
            "Enter submit · no matching command"
        } else {
            "Enter/Tab complete · Up/Down choose"
        }
    } else if has_pending_proposal {
        "Proposal pending · y apply from review panel · /diff /check /apply fallback"
    } else if interaction_mode == TuiInteractionMode::Agent {
        "MODE: AGENT · review panels for proposals · no auto-apply"
    } else {
        "Enter submit · Ctrl+J newline · / commands · q quit"
    }
}

/// header の表示行を組む。provider / model / source / git / mode を redact して1行目に、
/// pending があれば2行目に出す。model 名は truncate_middle で幅を抑える。
pub(super) fn header_lines(
    status: &StatusSnapshot,
    has_pending_apply: bool,
    interaction_mode: TuiInteractionMode,
) -> Vec<Line<'static>> {
    let provider = redact_sensitive_text(&status.provider_id);
    let model = redact_sensitive_text(status.model_id.as_deref().unwrap_or("unresolved"));
    let source = redact_sensitive_text(&status.model_source);
    let git_state = redact_sensitive_text(&status.git_state);
    let mut lines = vec![Line::from(format!(
        "origin-agent | provider: {} | model: {} ({}) | git: {} | mode: {}",
        provider,
        truncate_middle(&model, 44),
        source,
        git_state,
        interaction_mode.label()
    ))];
    let pending = pending_indicator_line(status, has_pending_apply);
    if !pending.is_empty() {
        lines.push(Line::from(pending));
    }
    lines
}

/// header 2行目の pending 表示。approval 待ちかどうかで案内文言を変え、none なら空文字を返す。
pub(super) fn pending_indicator_line(status: &StatusSnapshot, has_pending_apply: bool) -> String {
    if status.pending.status == "none" {
        return String::new();
    }

    let target = compact_pending_target(&status.pending);
    if has_pending_apply {
        return format!("approval: y/yes apply | Enter/n/no/other reject | target: {target}");
    }

    let checked = status.pending.checked.as_deref().unwrap_or("unknown");
    format!("pending: {target} | checked: {checked} | next: /diff /check /apply /clear")
}

/// pending proposal の対象パスを1つに要約する(requested を優先し、無ければ proposed)。
pub(super) fn compact_pending_target(pending: &PendingProposalSnapshot) -> String {
    let files = if pending.requested_files.is_empty() {
        &pending.proposed_files
    } else {
        &pending.requested_files
    };

    match files.as_slice() {
        [] => "proposal".to_string(),
        [file] => redact_sensitive_text(file),
        [first, rest @ ..] => format!("{} +{}", redact_sensitive_text(first), rest.len()),
    }
}

/// /help が出すコマンド一覧。日常コマンド・advanced/direct・手動 diff の fallback を段で分ける。
pub(super) fn tui_help_text() -> String {
    [
        "TUI commands:",
        "",
        "Daily:",
        "  /models      Pick a model route.",
        "  /mode        Show Ask/Agent mode (toggle).",
        "  /resume      Resume a past session (planned — not implemented yet).",
        "  /status      Show provider, model, config, session, cwd, and git state.",
        "  /help        Show this command list.",
        "  /exit        Quit the TUI.",
        "",
        "Advanced / direct:",
        "  /mode ask | /mode agent     Set mode explicitly.",
        "  /model | /model list        Show / list model routes (/model list = alias for /models).",
        "  /model set <model-id>       Switch model on the current provider (session-local).",
        "  /model set <provider>/<model-id>   Switch provider + model together (session-local).",
        "  /provider | /provider list  Show / list providers (diagnostics).",
        "  /provider set <provider-id> Switch provider (session-local).",
        "  /sessions                   Show recent saved sessions.",
        "  /quit                       Quit the TUI.",
        "  Switching is session-local and never rewrites ~/.origin/config.toml.",
        "",
        "Fallback / direct edit (manual diff control):",
        "  /propose <file> <instruction...>   Preview a diff for one explicit file. Nothing is applied.",
        "  /diff        Redisplay the pending proposal diff.",
        "  /check       Run git apply --check for the pending proposal. Nothing is applied.",
        "  /apply       Re-run git apply --check, then ask for approval. Only y/yes applies.",
        "  /clear       Clear visible messages and discard any pending proposal.",
        "",
        "Chat:",
        "  Ask mode (default) sends non-slash input as read-only chat; mentioned safe paths are added as context.",
        "  Agent mode shows a review panel before any proposal or write. No auto-apply.",
        "  Enter accepts a selected slash suggestion; otherwise it submits. Ctrl+J inserts a newline.",
        "  PageUp/PageDown scroll message history. Native terminal selection/copy works; mouse wheel is not captured.",
        "",
        "Exit: /exit, /quit, q, Esc, or Ctrl+C.",
    ]
    .join("\n")
}

/// /resume 未実装の案内文(代替の CLI 手順を示す)。
pub(super) fn tui_resume_planned_text() -> String {
    "/resume is planned for a future phase. No session was restored.\n\
For now: run `origin sessions` to browse saved sessions,\n\
or start with `origin chat --resume latest \"<your prompt>\"`."
        .to_string()
}

/// /propose の使い方(preview のみ・単一ファイル)を示す usage 文。
pub(super) fn tui_propose_usage() -> String {
    "Usage: /propose <file> <instruction...>\nTUI /propose is preview-only and single-file for now."
        .to_string()
}

/// Agent が「更新には単一の target が要る」と伝える文言(明示 target 要求へ委譲)。
pub(super) fn format_tui_agent_target_required() -> String {
    format_tui_agent_explicit_edit_target_required()
}

/// destination clarification を開いた旨と「preview も変更も無い」ことを伝える文言。
pub(super) fn format_tui_agent_destination_clarification_opened() -> String {
    "Destination clarification opened.\nChoose where Agent should go next. No preview was generated. No files were changed."
        .to_string()
}

/// planner 失敗の通知。理由を redact し、preview も変更も無いことを明示する。
pub(super) fn format_tui_agent_planner_error(error: &str) -> String {
    format!(
        "Agent planner failed.\nReason: {}\nNo preview was generated. No files were changed.",
        redact_sensitive_text(error)
    )
}

/// 更新先が1つ要る旨と入力例を示す案内。provider は呼んでいないことも明示する。
pub(super) fn format_tui_agent_explicit_edit_target_required() -> String {
    "Need more information\n\
I can update a file, but I need one target path.\n\
Example: docs/notes.md を整理して\n\
No provider was called."
        .to_string()
}

/// read-only 回答の開始通知。変更せず、何を読んで答えるかと model 待ちを示す。
pub(super) fn format_tui_agent_read_only_plan(result: &AskFileContextResult) -> String {
    let read_context = format_agent_read_context_description(result);
    format!(
        "Answering without file changes\n\
No files will be changed.\n\
Using: {read_context}\n\
Waiting for model..."
    )
}

/// read-only 回答で参照する context の説明文。workspace overview があれば件数で、無ければ
/// ファイル一覧から組む。
pub(super) fn format_agent_read_context_description(result: &AskFileContextResult) -> String {
    if let Some(overview) = &result.workspace_overview {
        format!(
            "bounded workspace overview ({} files read, {} paths listed)",
            overview.files.len(),
            overview.listed_entries
        )
    } else {
        format_agent_read_context_from_files(&result.file_contexts, &result.file_context_records)
    }
}

/// 参照ファイル一覧の説明文。included が無ければ skip 数を、あれば redact 済みパスを並べる。
pub(super) fn format_agent_read_context_from_files(
    files: &[ChatFileContext],
    records: &[ChatFileContextRecord],
) -> String {
    if files.is_empty() {
        let rejected = records
            .iter()
            .filter(|record| record.status != "included")
            .count();
        if rejected == 0 {
            "none selected".to_string()
        } else {
            format!("none usable ({rejected} skipped)")
        }
    } else {
        files
            .iter()
            .map(|file| redact_sensitive_text(&file.path))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// 現在の pending 状態(clarification か new-file か)に応じた番号操作ヒント文を選ぶ。
pub(super) fn format_tui_agent_plan_action_hint_for_state(state: &TuiState) -> String {
    if state.pending_agent_destination_clarification.is_some() {
        "Destination clarification pending.\nPress 1 to specify an existing file, 2 to create new Markdown, 3 for Other / custom instruction, or 4 to cancel.\nNumbers run immediately. Up/Down moves highlight. Enter confirms the highlighted action. Esc cancels."
            .to_string()
    } else {
        format_tui_agent_plan_action_hint(state.pending_agent_new_file_plan.is_some())
    }
}

/// Agent action review の番号操作ヒント。target を変えられるか(新規作成か)で文言を分ける。
pub(super) fn format_tui_agent_plan_action_hint(can_change_target: bool) -> String {
    if can_change_target {
        "Agent action review pending.\nPress 1 to generate a create preview, 2 to edit output path, or 3 to cancel.\nNumbers run immediately. Up/Down moves highlight. Enter confirms the highlighted action. Esc cancels."
            .to_string()
    } else {
        "Agent action review pending.\nPress 1 to generate an update preview, 2 for Other / custom instruction, or 3 to cancel.\nNumbers run immediately. Up/Down moves highlight. Enter confirms the highlighted action. Esc cancels."
            .to_string()
    }
}

/// chat 失敗の通知。理由を redact し、次の確認コマンドを案内する。
pub(super) fn format_tui_chat_error(error: &str) -> String {
    format!(
        "Chat request failed.\nReason: {}\nNext: check /status, /provider, and /model, then try again.",
        redact_sensitive_text(error)
    )
}

/// session 一覧取得失敗の通知。理由を redact し、CLI での代替手順を案内する。
pub(super) fn format_tui_sessions_error(error: &str) -> String {
    format!(
        "Could not list sessions.\nReason: {}\nNext: use plain CLI `origin sessions` for details.",
        redact_sensitive_text(error)
    )
}

/// /status の本文。runtime / provider / routing / model / config / workspace / pending を整形する。
/// pending が none 以外の時だけ checked・instruction・files を redact して追記する。
pub(super) fn format_tui_status(status: &StatusSnapshot) -> String {
    let routing = provider_routing_hint(status);
    let mut output = format!(
        "TUI status\n\nRuntime:\n  mode:     {}\n  command:  {}\n  cwd:      {}\n  git:      {}\n\nProvider:\n  id:       {}\n  kind:     {}\n  base_url: {}\n  auth:     {}\n\nRouting / Tradeoff:\n  route:      {}\n  privacy:    {}\n  quota/cost: {}\n  best for:   {}\n\nModel:\n  id:       {}\n  source:   {}\n\nConfig:\n  path:     {}\n  sessions: {}\n\nWorkspace:\n  workspace instructions: {}\n\nPending proposal:\n  status:   {}",
        status.mode,
        status.command,
        status.cwd,
        status.git_state,
        status.provider_id,
        status.provider_kind,
        status.base_url,
        status.auth_label,
        routing.route,
        routing.privacy,
        routing.quota_cost,
        routing.best_for,
        status.model_id.as_deref().unwrap_or("unresolved"),
        status.model_source,
        status.config_path,
        status.sessions_root,
        format_workspace_instructions_status(status),
        status.pending.status
    );
    if status.pending.status != "none" {
        if let Some(checked) = &status.pending.checked {
            output.push_str(&format!("\n  checked:  {checked}"));
        }
        if let Some(instruction) = &status.pending.instruction {
            output.push_str(&format!(
                "\n  instruction: {}",
                redact_sensitive_text(instruction)
            ));
        }
        if !status.pending.requested_files.is_empty() {
            output.push_str("\n  requested:");
            for file in &status.pending.requested_files {
                output.push_str(&format!("\n    - {}", redact_sensitive_text(file)));
            }
        }
        if !status.pending.proposed_files.is_empty() {
            output.push_str("\n  proposed:");
            for file in &status.pending.proposed_files {
                output.push_str(&format!("\n    - {}", redact_sensitive_text(file)));
            }
        }
    }
    output
}

/// ORIGIN.md workspace instructions の status 表示(loaded はサイズ、skipped は理由、他は none)。
pub(super) fn format_workspace_instructions_status(status: &StatusSnapshot) -> String {
    match status.workspace_instructions.status.as_str() {
        "loaded" => format!(
            "{} (loaded, {})",
            chat_workflow::WORKSPACE_INSTRUCTIONS_FILE,
            format_bytes(status.workspace_instructions.bytes.unwrap_or(0))
        ),
        "skipped" => {
            let reason = status
                .workspace_instructions
                .reason
                .as_deref()
                .unwrap_or("unknown");
            format!(
                "{} (skipped, {})",
                chat_workflow::WORKSPACE_INSTRUCTIONS_FILE,
                redact_sensitive_text(reason)
            )
        }
        _ => "none".to_string(),
    }
}

/// バイト数を 1KB 以上なら KB、未満なら bytes で表示する小ヘルパ。
pub(super) fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 {
        let kib = bytes as f64 / 1024.0;
        format!("{kib:.1} KB")
    } else {
        format!("{bytes} bytes")
    }
}
