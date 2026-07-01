//! provider/model route picker の表示行と選択状態。
//! unavailable route は選べないまま表示し、model switching は session-local に留める。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use origin_session::redact_sensitive_text;

use super::{fit_to_width, ModelRouteEntry};

/// route picker の表示・選択状態。利用可能な route 一覧と現在の選択位置、
/// 数字入力バッファを持つ。選択は session-local で、設定ファイルへは書き戻さない。
pub(super) struct ModelRoutePicker {
    pub(super) routes: Vec<ModelRouteEntry>,
    pub(super) selected: usize,
    pub(super) numeric_buffer: String,
}

/// テスト用:picker の表示行を文字列へ落として比較しやすくするヘルパ。
#[cfg(test)]
pub(super) fn format_tui_model_routes(
    routes: &[ModelRouteEntry],
    selected_index: usize,
    width: u16,
) -> String {
    model_route_picker_lines(routes, selected_index, width)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 1 route が占める表示行の範囲 [start, end)。scroll 開始位置の計算に使う。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModelRouteLineRange {
    start: usize,
    end: usize,
}

/// テスト用:行範囲を捨てて表示行だけ取り出す薄いラッパ。
#[cfg(test)]
pub(super) fn model_route_picker_lines(
    routes: &[ModelRouteEntry],
    selected_index: usize,
    width: u16,
) -> Vec<Line<'static>> {
    model_route_picker_lines_with_ranges(routes, selected_index, width).0
}

/// 表示高さに収まる範囲だけ picker 行を返す。全行が収まるならそのまま、
/// 溢れる時は選択中 route が見える位置から窓を切り出す。
pub(super) fn model_route_picker_visible_lines(
    routes: &[ModelRouteEntry],
    selected_index: usize,
    width: u16,
    visible_height: u16,
) -> Vec<Line<'static>> {
    let (lines, ranges) = model_route_picker_lines_with_ranges(routes, selected_index, width);
    let visible_height = visible_height as usize;
    if visible_height == 0 || lines.len() <= visible_height {
        return lines;
    }

    let start = model_route_picker_scroll_start(&ranges, selected_index, visible_height);
    lines.into_iter().skip(start).take(visible_height).collect()
}

/// 選択中 route が表示窓に収まるよう scroll の開始行を決める。route が窓より高い時は
/// その先頭から、そうでなければ末尾が見える範囲で start を選ぶ。
fn model_route_picker_scroll_start(
    ranges: &[ModelRouteLineRange],
    selected_index: usize,
    visible_height: usize,
) -> usize {
    if visible_height == 0 || ranges.is_empty() {
        return 0;
    }

    let selected = ranges
        .get(selected_index.min(ranges.len().saturating_sub(1)))
        .copied()
        .unwrap_or(ModelRouteLineRange { start: 0, end: 0 });
    if selected.end.saturating_sub(selected.start) >= visible_height {
        return selected.start;
    }

    selected
        .end
        .saturating_sub(visible_height)
        .min(selected.start)
}

/// picker 全体の表示行と、各 route の行範囲を組み立てる中心関数。
/// provider ごとに見出しを1度だけ出し、選択不可かつ model 未設定の route は番号を振らず
/// `unavailable` 行として表示する(選べないまま見せる)。
/// route 文字列は redact_sensitive_text を通してから載せる(安全境界)。
fn model_route_picker_lines_with_ranges(
    routes: &[ModelRouteEntry],
    selected_index: usize,
    width: u16,
) -> (Vec<Line<'static>>, Vec<ModelRouteLineRange>) {
    let mut lines = vec![Line::from("Model routes"), Line::from("")];
    if routes.is_empty() {
        lines.push(Line::from("No configured model routes found."));
        lines.push(Line::from(
            "Use /model set <model-id> to set a model for the current provider.",
        ));
        return (lines, Vec::new());
    }

    let route_width = (width.max(24) as usize).saturating_sub(6).max(12);
    let mut rendered_provider: Option<&str> = None;
    let mut ranges = Vec::with_capacity(routes.len());
    let mut number = 0usize;
    for (index, route) in routes.iter().enumerate() {
        if rendered_provider != Some(route.provider_id.as_str()) {
            if rendered_provider.is_some() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(format!(
                "{} — {}",
                model_route_provider_display_name(&route.provider_id),
                route.route_label
            )));
            lines.push(Line::from(""));
            rendered_provider = Some(route.provider_id.as_str());
        } else if index > 0 {
            lines.push(Line::from(""));
        }
        let selected = index == selected_index.min(routes.len().saturating_sub(1));
        let range_start = lines.len();
        // 選択不可かつ model 未設定の route は番号を振らず、選べない理由だけ dim で見せる。
        if !route.selectable && route.model_id.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("   "),
                Span::styled(
                    format!("unavailable: {}", route.availability_reason()),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]));
            ranges.push(ModelRouteLineRange {
                start: range_start,
                end: lines.len(),
            });
            continue;
        }
        // 選択可能な route にだけ表示用の連番を振る(unavailable 行は番号を持たない)。
        number += 1;
        lines.push(super::style::selection_row(
            &format!("{number}. "),
            &fit_to_width(&redact_sensitive_text(&route.route), route_width),
            selected,
            width,
        ));
        lines.push(model_route_metadata_line(route, width));
        ranges.push(ModelRouteLineRange {
            start: range_start,
            end: lines.len(),
        });
    }
    (lines, ranges)
}

/// route の補助情報(source・availability・current か)を dim の1行に整形する。
fn model_route_metadata_line(route: &ModelRouteEntry, width: u16) -> Line<'static> {
    let metadata_width = (width.max(24) as usize).saturating_sub(4).max(12);
    let metadata = format!(
        "{} · {} · current: {}",
        route.source,
        route.availability,
        if route.current { "yes" } else { "no" }
    );
    Line::from(vec![
        Span::raw("    "),
        Span::styled(
            fit_to_width(&metadata, metadata_width),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ])
}

impl ModelRouteEntry {
    /// `unavailable: ` 接頭辞を外した理由文だけを redact して返す表示用ヘルパ。
    fn availability_reason(&self) -> String {
        redact_sensitive_text(self.availability.trim_start_matches("unavailable: ").trim())
    }
}

/// provider id を表示名へ写像する(未知 id はそのまま見せる)。
fn model_route_provider_display_name(provider_id: &str) -> &str {
    match provider_id {
        "lmstudio" => "LM Studio",
        "openai" => "OpenAI",
        _ => provider_id,
    }
}
