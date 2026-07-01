//! TUI の共有 style と selection/panel 部品。
//! approval や write 判定は持たず、選択状態を見た目で伝えるだけに留める。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::composer::display_width;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PanelTone {
    Neutral,
    Active,
    Ready,
    Blocked,
    Dim,
}

pub(super) fn rail_color(tone: PanelTone) -> Color {
    match tone {
        PanelTone::Neutral => Color::Gray,
        PanelTone::Active => Color::Cyan,
        PanelTone::Ready => Color::Green,
        PanelTone::Blocked => Color::Red,
        PanelTone::Dim => Color::DarkGray,
    }
}

pub(super) const RAIL: &str = "▌";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RowEmphasis {
    Header,
    Text,
    Muted,
    Selected,
    Accent,
}

pub(super) fn row(
    rail: Option<PanelTone>,
    lead: &str,
    label: &str,
    emphasis: RowEmphasis,
    width: u16,
) -> Line<'static> {
    let available_width = width.max(12) as usize;
    let rail_width = if rail.is_some() {
        display_width(RAIL) + 1
    } else {
        0
    };
    let content = fit_display_width(
        &format!("{lead}{label}"),
        available_width.saturating_sub(rail_width),
    );
    let content_width = display_width(&content);
    let padding = " ".repeat(available_width.saturating_sub(rail_width + content_width));
    let content_style = match emphasis {
        RowEmphasis::Header => rail
            .map(rail_color)
            .map(|color| Style::default().fg(color).add_modifier(Modifier::BOLD))
            .unwrap_or_else(|| Style::default().add_modifier(Modifier::BOLD)),
        RowEmphasis::Text => Style::default(),
        RowEmphasis::Muted => Style::default().fg(Color::DarkGray),
        RowEmphasis::Selected => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        RowEmphasis::Accent => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    };

    let mut spans = Vec::new();
    if let Some(tone) = rail {
        spans.push(Span::styled(
            RAIL.to_string(),
            Style::default().fg(rail_color(tone)),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(format!("{content}{padding}"), content_style));
    Line::from(spans)
}

fn fit_display_width(value: &str, max_width: usize) -> String {
    if display_width(value) <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width <= 3 {
        return value
            .chars()
            .scan(0usize, |used, ch| {
                let width = display_width(&ch.to_string());
                if used.saturating_add(width) > max_width {
                    None
                } else {
                    *used += width;
                    Some(ch)
                }
            })
            .collect();
    }

    let target = max_width - 3;
    let mut used = 0usize;
    let mut output = String::new();
    for ch in value.chars() {
        let width = display_width(&ch.to_string());
        if used.saturating_add(width) > target {
            break;
        }
        used += width;
        output.push(ch);
    }
    output.push_str("...");
    output
}

pub(super) fn rail_header(title: &str, tone: PanelTone, width: u16) -> Line<'static> {
    row(Some(tone), "", title, RowEmphasis::Header, width)
}

pub(super) fn rail_text(text: &str, tone: PanelTone, width: u16) -> Line<'static> {
    row(Some(tone), "", text, RowEmphasis::Text, width)
}

pub(super) fn rail_muted(text: &str, tone: PanelTone, width: u16) -> Line<'static> {
    row(Some(tone), "", text, RowEmphasis::Muted, width)
}

pub(super) fn rail_blank(tone: PanelTone, width: u16) -> Line<'static> {
    row(Some(tone), "", "", RowEmphasis::Text, width)
}

pub(super) fn rail_accent(text: &str, tone: PanelTone, width: u16) -> Line<'static> {
    row(Some(tone), "", text, RowEmphasis::Accent, width)
}

pub(super) fn more_actions_hint(revealed: bool, tone: PanelTone, width: u16) -> Line<'static> {
    let label = if revealed { "? hide" } else { "? more actions" };
    rail_muted(label, tone, width)
}

pub(super) fn footer_hint(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {text}"),
        Style::default().fg(Color::DarkGray),
    ))
}

/// Shared selectable row for non-panel lists (`/models`, command palette). No rail, no marker;
/// the selected row uses the canonical blue/cyan foreground accent.
pub(super) fn selection_row(lead: &str, label: &str, selected: bool, width: u16) -> Line<'static> {
    row(
        None,
        lead,
        label,
        if selected {
            RowEmphasis::Selected
        } else {
            RowEmphasis::Text
        },
        width,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ActionLead {
    Number(usize),
    Yes,
    No,
    #[allow(dead_code)]
    Custom(char),
}

impl ActionLead {
    fn render(&self) -> String {
        match self {
            Self::Number(value) => format!("{value}. "),
            Self::Yes => "y. ".to_string(),
            Self::No => "n. ".to_string(),
            Self::Custom(value) => format!("{value}. "),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActionRow {
    pub lead: ActionLead,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActionList {
    pub rows: Vec<ActionRow>,
    pub selected: usize,
}

pub(super) fn numbered_action_list(labels: &[&str], selected: usize) -> ActionList {
    ActionList {
        rows: labels
            .iter()
            .enumerate()
            .map(|(index, label)| ActionRow {
                lead: ActionLead::Number(index + 1),
                label: (*label).to_string(),
            })
            .collect(),
        selected,
    }
}

pub(super) fn permission_action_list(yes_label: Option<&str>, selected: usize) -> ActionList {
    let mut rows = Vec::new();
    if let Some(label) = yes_label {
        rows.push(ActionRow {
            lead: ActionLead::Yes,
            label: label.to_string(),
        });
    }
    rows.push(ActionRow {
        lead: ActionLead::No,
        label: "Cancel / no files changed".to_string(),
    });
    ActionList { rows, selected }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BodyLine {
    Text(String),
    Muted(String),
    Blank,
    Accent(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PanelView {
    pub tone: PanelTone,
    pub title: String,
    pub body: Vec<BodyLine>,
    pub actions: Option<ActionList>,
    pub advanced: Vec<BodyLine>,
    pub show_advanced: bool,
}

pub(super) fn render_panel(view: &PanelView, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![rail_header(&view.title, view.tone, width)];
    for body in &view.body {
        lines.push(render_body_line(body, view.tone, width));
    }
    if let Some(actions) = &view.actions {
        for (index, action) in actions.rows.iter().enumerate() {
            lines.push(row(
                Some(view.tone),
                &action.lead.render(),
                &action.label,
                if index == actions.selected {
                    RowEmphasis::Selected
                } else {
                    RowEmphasis::Muted
                },
                width,
            ));
        }
    }
    if !view.advanced.is_empty() {
        if view.show_advanced {
            for body in &view.advanced {
                lines.push(render_body_line(body, view.tone, width));
            }
        }
        lines.push(more_actions_hint(view.show_advanced, view.tone, width));
    }
    lines
}

fn render_body_line(body: &BodyLine, tone: PanelTone, width: u16) -> Line<'static> {
    match body {
        BodyLine::Text(text) => rail_text(text, tone, width),
        BodyLine::Muted(text) => rail_muted(text, tone, width),
        BodyLine::Blank => rail_blank(tone, width),
        BodyLine::Accent(text) => rail_accent(text, tone, width),
    }
}

pub(super) fn selection_buffer_highlight(buffer: &str, len: usize) -> Option<usize> {
    let value = buffer.parse::<usize>().ok()?;
    if (1..=len).contains(&value) {
        Some(value - 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Style};

    use super::{
        more_actions_hint, numbered_action_list, permission_action_list, rail_color, row,
        selection_buffer_highlight, ActionLead, PanelTone, RowEmphasis,
    };

    fn first_span_style(line: &ratatui::text::Line) -> Style {
        line.spans
            .first()
            .map(|span| span.style)
            .unwrap_or_default()
    }

    #[test]
    fn rail_colors_match_panel_tones() {
        assert_eq!(rail_color(PanelTone::Neutral), Color::Gray);
        assert_eq!(rail_color(PanelTone::Active), Color::Cyan);
        assert_eq!(rail_color(PanelTone::Ready), Color::Green);
        assert_eq!(rail_color(PanelTone::Blocked), Color::Red);
        assert_eq!(rail_color(PanelTone::Dim), Color::DarkGray);
    }

    #[test]
    fn selection_buffer_highlight_is_one_based_and_bounded() {
        assert_eq!(selection_buffer_highlight("1", 3), Some(0));
        assert_eq!(selection_buffer_highlight("12", 3), None);
        assert_eq!(selection_buffer_highlight("12", 20), Some(11));
        assert_eq!(selection_buffer_highlight("", 3), None);
        assert_eq!(selection_buffer_highlight("0", 3), None);
    }

    #[test]
    fn selected_row_uses_foreground_accent_without_old_markers() {
        let line = row(
            Some(PanelTone::Neutral),
            "y. ",
            "Yes",
            RowEmphasis::Selected,
            40,
        );
        let style = line.spans.get(2).map(|span| span.style).unwrap_or_default();

        assert_eq!(style.fg, Some(Color::Cyan));
        assert_eq!(style.bg, None);
        assert!(!line.to_string().contains('>'));
        assert!(!line.to_string().contains('›'));
    }

    #[test]
    fn row_truncates_long_content_to_requested_width() {
        let line = row(
            Some(PanelTone::Neutral),
            "",
            "very/long/path/that/should/not/wrap/in/the/interaction/pane.md",
            RowEmphasis::Text,
            32,
        );

        assert!(line.width() <= 32);
        assert!(line.to_string().contains("..."));
    }

    #[test]
    fn more_actions_hint_toggles_label() {
        assert_eq!(
            more_actions_hint(false, PanelTone::Neutral, 40)
                .to_string()
                .trim(),
            "▌ ? more actions"
        );
        assert_eq!(
            more_actions_hint(true, PanelTone::Neutral, 40)
                .to_string()
                .trim(),
            "▌ ? hide"
        );
    }

    #[test]
    fn action_list_helpers_build_expected_leads() {
        let numbered = numbered_action_list(&["One", "Two", "Three"], 0);
        assert_eq!(numbered.rows[0].lead, ActionLead::Number(1));
        assert_eq!(numbered.rows[1].lead, ActionLead::Number(2));
        assert_eq!(numbered.rows[2].lead, ActionLead::Number(3));

        let permission = permission_action_list(Some("Update file"), 0);
        assert_eq!(permission.rows[0].lead, ActionLead::Yes);
        assert_eq!(permission.rows[1].lead, ActionLead::No);

        let cancel_only = permission_action_list(None, 0);
        assert_eq!(cancel_only.rows.len(), 1);
        assert_eq!(cancel_only.rows[0].lead, ActionLead::No);
    }

    #[test]
    fn first_span_style_helper_reads_rail_style() {
        let line = row(
            Some(PanelTone::Active),
            "",
            "Header",
            RowEmphasis::Header,
            40,
        );
        assert_eq!(first_span_style(&line).fg, Some(Color::Cyan));
    }
}
