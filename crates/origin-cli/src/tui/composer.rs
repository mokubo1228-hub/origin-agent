//! composer の表示行・カーソル位置・入力編集 helper。
//! UTF-8 boundary と表示幅を分けて扱い、IME / 全角文字でも terminal cursor を壊さない。

use ratatui::layout::Position;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// 表示できる入力行数の上限。これを超えた古い行は畳んで省略表示にする。
pub(super) const MAX_INPUT_TEXT_LINES: u16 = 6;
/// affordance / spacer を含む composer 枠の固定行数。レイアウト高さ計算に使う。
pub(super) const COMPOSER_BORDER_LINES: u16 = 3;
/// 先頭行の prompt 記号。表示幅がカーソル x 起点になる。
const PROMPT_PREFIX: &str = "› ";
/// 2行目以降の継続 prefix。PROMPT_PREFIX と表示幅を揃え、行頭でカーソル列がずれないようにする。
pub(super) const PROMPT_CONTINUATION_PREFIX: &str = "  ";

/// 入力の論理行数(改行区切り)。空入力でも 1 を返す。
pub(super) fn composer_line_count(input: &str) -> u16 {
    if input.is_empty() {
        return 1;
    }
    input.split('\n').count().max(1) as u16
}

/// テスト用:既定 placeholder で表示行を組み立てる薄いラッパ。
#[cfg(test)]
pub(super) fn composer_lines(input: &str) -> Vec<Line<'static>> {
    composer_lines_with_placeholder(input, "Message or /command")
}

/// 入力を表示用の Line 列に変換する。空なら placeholder を出し、上限超過時は古い行を
/// 「earlier input hidden」に畳んで末尾だけ見せる。
pub(super) fn composer_lines_with_placeholder(
    input: &str,
    placeholder: &str,
) -> Vec<Line<'static>> {
    if input.is_empty() {
        return vec![prompt_line(placeholder, true)];
    }

    let input_lines = input.split('\n').collect::<Vec<_>>();
    if input_lines.len() > MAX_INPUT_TEXT_LINES as usize {
        let visible_tail_count = (MAX_INPUT_TEXT_LINES - 1) as usize;
        let start = input_lines.len().saturating_sub(visible_tail_count);
        let mut lines = vec![Line::from(format!(
            "{PROMPT_CONTINUATION_PREFIX}... earlier input hidden ..."
        ))];
        lines.extend(
            input_lines[start..]
                .iter()
                .map(|line| Line::from(format!("{PROMPT_CONTINUATION_PREFIX}{line}"))),
        );
        return lines;
    }

    input_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                prompt_line(line, false)
            } else {
                Line::from(format!("{PROMPT_CONTINUATION_PREFIX}{line}"))
            }
        })
        .collect()
}

/// 先頭行(または placeholder)を prompt prefix + 本文 span で組む。
/// placeholder は DIM、実入力は BOLD で見分けられるようにする。
fn prompt_line(text: &str, placeholder: bool) -> Line<'static> {
    let value = text.to_string();
    let input_style = if placeholder {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(vec![
        Span::raw(PROMPT_PREFIX.to_string()),
        Span::styled(value, input_style),
    ])
}

/// テスト用:既定 placeholder / affordance で composer セル行を組むラッパ。
#[cfg(test)]
pub(super) fn composer_cell_lines(input: &str, width: u16) -> Vec<Line<'static>> {
    composer_cell_lines_with_placeholder(
        input,
        width,
        "Message or /command",
        "Message or /command",
        true,
    )
}

/// composer 枠1つ分の行(affordance / spacer / 内容 / spacer)を width に合わせて作る。
/// is_live は入力受付状態の背景強調の有無を切り替える。
pub(super) fn composer_cell_lines_with_placeholder(
    input: &str,
    width: u16,
    placeholder: &str,
    affordance: &str,
    is_live: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(composer_affordance_line(affordance));
    lines.push(composer_spacer_line(width, is_live));
    lines.extend(
        composer_lines_with_placeholder(input, placeholder)
            .into_iter()
            .map(|line| composer_content_line(line, width, is_live)),
    );
    lines.push(composer_spacer_line(width, is_live));
    lines
}

/// composer 上部の操作ヒント行(DIM 表示)。
fn composer_affordance_line(text: &str) -> Line<'static> {
    Line::styled(
        format!("  {text}"),
        Style::default().add_modifier(Modifier::DIM),
    )
}

/// 内容の上下に挟む空行。live 時は幅いっぱいの背景色で composer 領域を示す。
fn composer_spacer_line(width: u16, is_live: bool) -> Line<'static> {
    let target_width = width.max(12).saturating_sub(1) as usize;
    Line::from(Span::styled(
        " ".repeat(target_width),
        if is_live {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        },
    ))
}

/// 1本の入力行を width 幅のセル行に整える。live 時は背景色を敷き、prompt prefix の
/// span だけ cyan/bold で強調し、右側を padding で埋める。
fn composer_content_line(line: Line<'static>, width: u16, is_live: bool) -> Line<'static> {
    let available_width = width.max(12) as usize;
    let target_width = available_width.saturating_sub(1);
    let used_width = line.width() + 2;
    let padding = " ".repeat(target_width.saturating_sub(used_width));
    let live_style = Style::default().bg(Color::DarkGray);
    let mut spans = vec![Span::styled(
        "  ",
        if is_live {
            live_style
        } else {
            Style::default()
        },
    )];
    spans.extend(line.spans.into_iter().enumerate().map(|(index, mut span)| {
        if is_live {
            span.style = span.style.bg(Color::DarkGray);
            if index == 0 && span.content.starts_with(PROMPT_PREFIX) {
                span.style = span.style.fg(Color::Cyan).add_modifier(Modifier::BOLD);
            }
        }
        if !is_live {
            span.style = span.style.add_modifier(Modifier::DIM);
        }
        span
    }));
    spans.push(Span::styled(
        padding,
        if is_live {
            live_style
        } else {
            Style::default()
        },
    ));
    Line::from(spans)
}

/// terminal カーソルを置くべき表示位置(列 x・行 y)を求める。折り畳み中の行や
/// 継続 prefix も考慮し、見た目の列に一致させる。
pub(super) fn composer_cursor_offset(input: &str, cursor: usize) -> Position {
    // terminal cursor は byte offset ではなく表示幅で置く必要があるため、
    // IME / 全角文字でも prompt prefix からの見た目の列を基準にする。
    let cursor = clamp_to_char_boundary(input, cursor);
    let line_count = composer_input_line_count(input);
    let cursor_line = composer_cursor_line(input, cursor);
    let hidden_lines = line_count > MAX_INPUT_TEXT_LINES as usize;
    let visible_start_index = if hidden_lines {
        line_count.saturating_sub((MAX_INPUT_TEXT_LINES - 1) as usize)
    } else {
        0
    };

    if hidden_lines && cursor_line.index < visible_start_index {
        let x = 2
            + display_width(PROMPT_CONTINUATION_PREFIX)
            + display_width("... earlier input hidden ...");
        return Position { x: x as u16, y: 0 };
    }

    let prefix = if hidden_lines || cursor_line.index > 0 {
        PROMPT_CONTINUATION_PREFIX
    } else {
        PROMPT_PREFIX
    };
    let visible_y = cursor_line.index.saturating_sub(visible_start_index);
    let y = if hidden_lines {
        visible_y + 1
    } else {
        visible_y
    } as u16;
    let text_before_cursor = &input[cursor_line.start..cursor];
    let x = 2 + display_width(prefix) + display_width(text_before_cursor);
    Position { x: x as u16, y }
}

/// カーソルが属する論理行の index と、その行頭の byte offset。
#[derive(Clone, Copy)]
struct ComposerCursorLine {
    index: usize,
    start: usize,
}

/// 入力の論理行数(空でも 1)。表示の折り畳み判定に使う内部版。
fn composer_input_line_count(input: &str) -> usize {
    input.split('\n').count().max(1)
}

/// カーソル位置から、属する行 index とその行頭 byte offset を割り出す。
fn composer_cursor_line(input: &str, cursor: usize) -> ComposerCursorLine {
    let before_cursor = &input[..cursor];
    let index = before_cursor.bytes().filter(|byte| *byte == b'\n').count();
    let start = before_cursor
        .rfind('\n')
        .map(|newline| newline.saturating_add(1))
        .unwrap_or(0);
    ComposerCursorLine { index, start }
}

/// 文字列の表示幅(端末の列数)。byte 長ではなく、全角を 2 として数える。
pub(super) fn display_width(text: &str) -> usize {
    Line::from(text.to_string()).width()
}

/// カーソル位置に1文字挿入する。挿入前に char boundary へ丸め、UTF-8 を壊さない。
pub(super) fn insert_input_char(input: &mut String, cursor: &mut usize, value: char) {
    let current = clamp_to_char_boundary(input, *cursor);
    input.insert(current, value);
    *cursor = current + value.len_utf8();
}

/// カーソル直前の1文字を消す。境界に丸めてから1 char 分 drain し、複合文字でも
/// byte 単位で割らない。
pub(super) fn backspace_input_char(input: &mut String, cursor: &mut usize) {
    let current = clamp_to_char_boundary(input, *cursor);
    if current == 0 {
        *cursor = 0;
        return;
    }
    let previous = previous_input_cursor_boundary(input, current);
    input.drain(previous..current);
    *cursor = previous;
}

/// カーソルを直前の char boundary へ1文字戻す。
pub(super) fn move_input_cursor_left(input: &str, cursor: &mut usize) {
    *cursor = previous_input_cursor_boundary(input, *cursor);
}

/// カーソルを次の char boundary へ1文字進める。
pub(super) fn move_input_cursor_right(input: &str, cursor: &mut usize) {
    *cursor = next_input_cursor_boundary(input, *cursor);
}

/// カーソルを1行上の同じ表示列へ移す。
pub(super) fn move_input_cursor_up(input: &str, cursor: &mut usize) {
    move_input_cursor_vertical(input, cursor, -1);
}

/// カーソルを1行下の同じ表示列へ移す。
pub(super) fn move_input_cursor_down(input: &str, cursor: &mut usize) {
    move_input_cursor_vertical(input, cursor, 1);
}

/// 上下移動の本体。目標行へ移しつつ、現在行の表示列をできるだけ保つ。
fn move_input_cursor_vertical(input: &str, cursor: &mut usize, delta: isize) {
    let lines = composer_logical_lines(input);
    if lines.is_empty() {
        *cursor = 0;
        return;
    }
    let current_index = composer_line_index_for_cursor(&lines, *cursor);
    let target_index = if delta.is_negative() {
        current_index.saturating_sub(delta.unsigned_abs())
    } else {
        (current_index + delta as usize).min(lines.len().saturating_sub(1))
    };
    if target_index == current_index {
        return;
    }

    let current_line = lines[current_index];
    let target_column = display_width(&input[current_line.start..(*cursor).min(current_line.end)]);
    // 上下移動は byte index ではなく表示列を保つ。全角文字をまたぐ時も
    // valid UTF-8 boundary に戻すことで composer の入力状態を壊さない。
    *cursor = cursor_boundary_for_display_column(input, lines[target_index], target_column);
}

/// cursor の直前にある char boundary を返す(先頭なら 0)。
fn previous_input_cursor_boundary(input: &str, cursor: usize) -> usize {
    let cursor = clamp_to_char_boundary(input, cursor);
    if cursor == 0 {
        return 0;
    }
    input[..cursor]
        .char_indices()
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// cursor の次にある char boundary を返す(末尾なら len)。
fn next_input_cursor_boundary(input: &str, cursor: usize) -> usize {
    let cursor = clamp_to_char_boundary(input, cursor);
    if cursor >= input.len() {
        return input.len();
    }
    input[cursor..]
        .char_indices()
        .nth(1)
        .map(|(offset, _)| cursor + offset)
        .unwrap_or(input.len())
}

/// cursor を入力長に収め、UTF-8 の char boundary まで後退させる安全ガード。
/// 全角 / IME 由来の中途半端な byte offset を、有効な境界に正してから使う。
pub(super) fn clamp_to_char_boundary(input: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(input.len());
    while cursor > 0 && !input.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

/// 1論理行の byte 範囲(行頭 start・行末 end、改行は含まない)。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ComposerLogicalLine {
    start: usize,
    end: usize,
}

/// 入力を改行で割り、各論理行の byte 範囲を列挙する(上下移動の基準)。
fn composer_logical_lines(input: &str) -> Vec<ComposerLogicalLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    for line in input.split('\n') {
        let end = start + line.len();
        lines.push(ComposerLogicalLine { start, end });
        start = end.saturating_add(1);
    }
    if lines.is_empty() {
        lines.push(ComposerLogicalLine { start: 0, end: 0 });
    }
    lines
}

/// cursor を含む論理行の index を返す。該当が無ければ最終行に寄せる。
fn composer_line_index_for_cursor(lines: &[ComposerLogicalLine], cursor: usize) -> usize {
    lines
        .iter()
        .position(|line| cursor >= line.start && cursor <= line.end)
        .unwrap_or_else(|| lines.len().saturating_sub(1))
}

/// 目標表示列に最も近い char boundary を行内から選ぶ。表示列 → byte offset の逆引きで、
/// 全角文字をまたいでも有効な境界だけを cursor に採用する。
fn cursor_boundary_for_display_column(
    input: &str,
    line: ComposerLogicalLine,
    target_column: usize,
) -> usize {
    let mut best_cursor = line.start;
    let mut best_distance = target_column;
    let mut current_width = 0usize;

    for (offset, ch) in input[line.start..line.end].char_indices() {
        let char_end = line.start + offset + ch.len_utf8();
        let next_width = current_width.saturating_add(UnicodeWidthChar::width(ch).unwrap_or(0));
        let next_distance = next_width.abs_diff(target_column);
        if next_distance < best_distance {
            best_cursor = char_end;
            best_distance = next_distance;
        }
        current_width = next_width;
    }

    best_cursor
}
