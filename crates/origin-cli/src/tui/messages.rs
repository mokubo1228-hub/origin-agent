//! chat message を表示用 block/line に整形する helper。
//! transcript や proposal state は変更せず、secret redaction 済みの表示単位だけを返す。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use origin_session::redact_sensitive_text;

use super::style::{self, PanelTone};
use super::TuiMessage;

/// assistant 返信の本文先頭に置くリードマーカー。
const ASSISTANT_REPLY_MARKER: &str = "✦";
/// 会話行(you / assistant)のリード列の固定幅。本文の左揃えを揃えるため。
const CHAT_LEAD_WIDTH: usize = 6;
/// system 行(action / waiting / error)の小文字ラベル列の固定幅。
const SYSTEM_LABEL_WIDTH: usize = 9;
/// 上(過去)方向にまだ隠れた出力があることを示すスクロール indicator。PgUp で遡る。
pub(super) const CHAT_OLDER_INDICATOR: &str = "↑ earlier output · PgUp";
/// 下(新しい)方向にまだ隠れた出力があることを示すスクロール indicator。PgDn で進む。
pub(super) const CHAT_NEWER_INDICATOR: &str = "↓ newer output · PgDn";

/// テスト用:block 構造を捨てて表示行を平坦化した一覧を得るショートカット。
#[cfg(test)]
pub(super) fn chat_lines(messages: &[TuiMessage], loading: bool) -> Vec<Line<'static>> {
    flatten_message_blocks(chat_line_blocks(messages, loading))
}

/// 各 TuiMessage を1 block(複数行)に整形した一覧を返す。message ごとの境界を
/// block 単位で保つことで、後段の scroll / crop が message の途中で切らずに済む。
/// loading 中は waiting block を末尾に足し、message が空なら ready の placeholder を1つ出す。
pub(super) fn chat_line_blocks(messages: &[TuiMessage], loading: bool) -> Vec<Vec<Line<'static>>> {
    let mut blocks = messages
        .iter()
        .map(tui_message_block)
        .collect::<Vec<Vec<Line<'static>>>>();
    if loading {
        blocks.push(tui_message_block(&TuiMessage::Waiting(
            "waiting for model...".to_string(),
        )));
    }
    if blocks.is_empty() {
        let mut block = Vec::new();
        push_message_block(
            &mut block,
            "ACTION",
            MessageBlockStyle::Action,
            "TUI workflow ready",
        );
        blocks.push(block);
    }
    blocks
}

/// 1つの TuiMessage を、種別ごとのラベル / スタイルを付けた表示行群へ変換する。
pub(super) fn tui_message_block(message: &TuiMessage) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match message {
        TuiMessage::User(text) => {
            push_message_block(&mut lines, "YOU", MessageBlockStyle::User, text)
        }
        TuiMessage::Assistant(text) => {
            push_message_block(&mut lines, "ASSISTANT", MessageBlockStyle::Assistant, text)
        }
        TuiMessage::Action(text) => {
            push_message_block(&mut lines, "ACTION", MessageBlockStyle::Action, text)
        }
        TuiMessage::Waiting(text) => {
            push_message_block(&mut lines, "WAITING", MessageBlockStyle::Waiting, text)
        }
        TuiMessage::Error(text) => {
            push_message_block(&mut lines, "ERROR", MessageBlockStyle::Error, text)
        }
    }
    lines
}

/// 表示領域(幅・高さ)に収まる chat 行を、scroll offset を考慮して切り出す。
/// 全行が収まるなら全部、溢れる時だけ scroll / indicator 付きの窓に絞る。
/// transcript 自体は変えず、表示用の行スライスを返すだけ(secret は block 整形時に redact 済み)。
pub(super) fn chat_visible_lines(
    messages: &[TuiMessage],
    loading: bool,
    visible_width: u16,
    visible_height: u16,
    scroll_offset: usize,
) -> Vec<Line<'static>> {
    let max_lines = visible_height as usize;
    let blocks = chat_line_blocks(messages, loading);
    let mut lines = flatten_message_blocks(blocks.clone());
    // 行数が画面に満たない時は先頭に空行を1つ入れ、出力を下端へ寄せる。
    if !messages.is_empty() && lines.len() < max_lines {
        lines.insert(0, Line::from(""));
    }
    let wrapped_lines = wrap_chat_lines(lines, visible_width);
    if wrapped_lines.len() <= max_lines {
        return wrapped_lines;
    }
    scrollable_chat_lines(
        blocks,
        wrapped_lines,
        max_lines,
        scroll_offset,
        visible_width,
    )
}

/// 溢れている時の表示窓を組む。offset 0 は最新側に貼り付き、上に older indicator を
/// 1行足す。それ以外は scroll_chat_lines に委ねて任意位置の窓を作る。
fn scrollable_chat_lines(
    blocks: Vec<Vec<Line<'static>>>,
    lines: Vec<Line<'static>>,
    max_lines: usize,
    scroll_offset: usize,
    visible_width: u16,
) -> Vec<Line<'static>> {
    if max_lines == 0 {
        return Vec::new();
    }

    if scroll_offset == 0 {
        if max_lines == 1 {
            return vec![chat_hidden_indicator_line(CHAT_OLDER_INDICATOR)];
        }
        let mut visible = tail_message_blocks_wrapped(blocks, max_lines - 1, visible_width);
        visible.insert(0, chat_hidden_indicator_line(CHAT_OLDER_INDICATOR));
        return visible;
    }

    scroll_chat_lines(lines, max_lines, scroll_offset)
}

/// scroll offset から表示窓の [start, end) を計算し、必要なら上下に older / newer
/// indicator を差し込む。最後に max_lines を超えないよう切り詰める。
fn scroll_chat_lines(
    lines: Vec<Line<'static>>,
    max_lines: usize,
    scroll_offset: usize,
) -> Vec<Line<'static>> {
    if max_lines == 0 || lines.is_empty() {
        return Vec::new();
    }

    let total = lines.len();
    let offset = scroll_offset.min(total.saturating_sub(1));
    let mut end = total.saturating_sub(offset).max(1);
    if end < total {
        let top_body_capacity = max_lines.saturating_sub(1).max(1);
        end = end.max(top_body_capacity.min(total));
    }
    // newer 側に隠れ行があれば indicator 1行分、本文の枠を空ける。
    let has_newer = end < total;
    let newer_lines = usize::from(has_newer);
    let mut body_capacity = max_lines.saturating_sub(newer_lines).max(1);
    let mut start = end.saturating_sub(body_capacity);
    let has_older = start > 0;
    // older 側にも隠れ行があれば、さらに indicator 1行分を引いて start を取り直す。
    if has_older && body_capacity > 1 {
        body_capacity = max_lines.saturating_sub(newer_lines + 1).max(1);
        start = end.saturating_sub(body_capacity);
    }

    let mut visible = Vec::new();
    if start > 0 {
        visible.push(chat_hidden_indicator_line(CHAT_OLDER_INDICATOR));
    }
    visible.extend(lines.into_iter().skip(start).take(end - start));
    if has_newer && visible.len() < max_lines {
        visible.push(chat_hidden_indicator_line(CHAT_NEWER_INDICATOR));
    }
    visible.truncate(max_lines);
    visible
}

/// 隠れ出力 indicator を dim グレーの1行に整形する小ヘルパ。
fn chat_hidden_indicator_line(text: &str) -> Line<'static> {
    Line::styled(
        format!("  {text}"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    )
}

/// 各行を表示幅で折り返す。幅は最低1を保証し、元の style / 整列は wrap_chat_line が引き継ぐ。
pub(super) fn wrap_chat_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let max_width = usize::from(width).max(1);
    lines
        .into_iter()
        .flat_map(|line| wrap_chat_line(line, max_width))
        .collect()
}

/// 1行を表示幅で折り返す。span の style を保ったまま、文字の表示幅(全角考慮)で区切る。
/// 幅内に収まる行はそのまま返す。
fn wrap_chat_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    if line.width() <= max_width {
        return vec![line];
    }

    let line_style = line.style;
    let alignment = line.alignment;
    let mut wrapped = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;

    for span in line.spans {
        let span_style = span.style;
        let mut chunk = String::new();
        for ch in span.content.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            // 現在行に次の文字を足すと幅を超える時点で改行し、貯めた span を1行に確定する。
            if current_width > 0 && current_width + ch_width > max_width {
                if !chunk.is_empty() {
                    current_spans.push(Span::styled(std::mem::take(&mut chunk), span_style));
                }
                wrapped.push(line_from_wrapped_spans(
                    std::mem::take(&mut current_spans),
                    line_style,
                    alignment,
                ));
                current_width = 0;
            }
            chunk.push(ch);
            current_width = current_width.saturating_add(ch_width);
        }
        if !chunk.is_empty() {
            current_spans.push(Span::styled(chunk, span_style));
        }
    }

    if current_spans.is_empty() {
        wrapped.push(line_from_wrapped_spans(Vec::new(), line_style, alignment));
    } else {
        wrapped.push(line_from_wrapped_spans(
            current_spans,
            line_style,
            alignment,
        ));
    }
    wrapped
}

/// 折り返しで生まれた span 列に、元行の style と alignment を移して Line にする。
fn line_from_wrapped_spans(
    spans: Vec<Span<'static>>,
    style: Style,
    alignment: Option<ratatui::layout::Alignment>,
) -> Line<'static> {
    let mut line = Line::from(spans);
    line.style = style;
    line.alignment = alignment;
    line
}

/// block 群を1本の行列に連結する。block の境目に空行を1つ挟んで視覚的に区切る。
pub(super) fn flatten_message_blocks(blocks: Vec<Vec<Line<'static>>>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, block) in blocks.into_iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        lines.extend(block);
    }
    lines
}

/// 末尾(最新)から block 単位で max_lines に収まるだけ拾い、入り切らない先頭 block は
/// tail を crop する。block 境界を尊重しつつ最新側を優先的に見せるため。
fn tail_message_blocks_wrapped(
    blocks: Vec<Vec<Line<'static>>>,
    max_lines: usize,
    visible_width: u16,
) -> Vec<Line<'static>> {
    if max_lines == 0 {
        return Vec::new();
    }

    let mut remaining = max_lines;
    let mut visible_blocks = Vec::new();
    for block in blocks.into_iter().rev() {
        let wrapped_block = wrap_chat_lines(block.clone(), visible_width);
        let separator = usize::from(!visible_blocks.is_empty());
        let needed = separator + wrapped_block.len();
        if needed <= remaining {
            visible_blocks.push(wrapped_block);
            remaining -= needed;
            continue;
        }

        if remaining > separator {
            visible_blocks.push(crop_wrapped_message_block_tail(
                block,
                remaining - separator,
                visible_width,
            ));
        }
        break;
    }

    visible_blocks.reverse();
    flatten_message_blocks(visible_blocks)
}

/// 1 block が max_lines に入らない時の切り詰め。状態ヘッダ行があれば先頭1行を残し、
/// 残りを末尾側から見せる(どの message かのラベルを失わないため)。
fn crop_wrapped_message_block_tail(
    block: Vec<Line<'static>>,
    max_lines: usize,
    visible_width: u16,
) -> Vec<Line<'static>> {
    if max_lines == 0 {
        return Vec::new();
    }
    let has_header = message_block_has_visible_state_header(&block);
    let wrapped = wrap_chat_lines(block.clone(), visible_width);
    if wrapped.len() <= max_lines {
        return wrapped;
    }
    if !has_header {
        let tail_start = wrapped.len().saturating_sub(max_lines);
        return wrapped.into_iter().skip(tail_start).collect();
    }
    if max_lines == 1 {
        return wrap_chat_lines(vec![block[0].clone()], visible_width)
            .into_iter()
            .take(1)
            .collect();
    }

    let mut cropped = wrap_chat_lines(vec![block[0].clone()], visible_width);
    cropped.truncate(1);
    let tail_count = max_lines.saturating_sub(cropped.len());
    let tail_start = wrapped.len().saturating_sub(tail_count);
    cropped.extend(wrapped.into_iter().skip(tail_start));
    cropped.truncate(max_lines);
    cropped
}

/// block の先頭が RAIL 付きの system 状態ヘッダ行かどうか(crop 時にヘッダを残す判定)。
fn message_block_has_visible_state_header(block: &[Line<'static>]) -> bool {
    let Some(first) = block.first() else {
        return false;
    };
    first
        .spans
        .first()
        .is_some_and(|span| span.content.as_ref() == style::RAIL)
}

/// message の表示分類。ラベル・配色・先頭マーカーの出し分けに使う表示専用の区分。
#[derive(Clone, Copy)]
enum MessageBlockStyle {
    User,
    Assistant,
    Action,
    Waiting,
    Error,
}

/// 1 message 分の表示行を lines に追加する。ここで必ず redact_sensitive_text を通し、
/// secret を含む生テキストを表示単位に持ち込まない(安全境界)。
/// 先頭行にだけラベル / マーカーを付け、続く行は本文だけを並べる。
fn push_message_block(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    style: MessageBlockStyle,
    text: &str,
) {
    let text = redact_sensitive_text(text);
    let display_lines = message_display_lines(style, &text);
    let mut parts = display_lines.into_iter();
    if let Some(first) = parts.next() {
        lines.push(styled_message_body_line(style, label, first, true));
    } else {
        lines.push(styled_message_body_line(style, label, "", true));
    }
    for line in parts {
        lines.push(styled_message_body_line(style, label, line, false));
    }
}

/// 本文を行に割る。assistant だけは先頭の空行を落として返信が詰まって見えるようにし、
/// 全て空なら1つの空行を返す。
fn message_display_lines<'a>(style: MessageBlockStyle, text: &'a str) -> Vec<&'a str> {
    let lines = text.lines();
    let display_lines = if matches!(style, MessageBlockStyle::Assistant) {
        lines
            .skip_while(|line| line.trim().is_empty())
            .collect::<Vec<_>>()
    } else {
        lines.collect::<Vec<_>>()
    };

    if display_lines.is_empty() {
        vec![""]
    } else {
        display_lines
    }
}

/// 表示分類に応じて、その1行を会話行(you / assistant)か system 行へ振り分けて整形する。
fn styled_message_body_line(
    style: MessageBlockStyle,
    label: &str,
    line: &str,
    first_body_line: bool,
) -> Line<'static> {
    match style {
        MessageBlockStyle::User => user_message_line(line),
        MessageBlockStyle::Assistant => assistant_message_line(line, first_body_line),
        MessageBlockStyle::Action | MessageBlockStyle::Waiting | MessageBlockStyle::Error => {
            system_message_line(style, label, line, first_body_line)
        }
    }
}

/// user 行:`you` リード付きの会話行として整形する。
fn user_message_line(line: &str) -> Line<'static> {
    conversation_message_line(
        "you",
        line,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
        message_block_style(MessageBlockStyle::User),
    )
}

/// assistant 行:本文先頭行にだけ reply marker を付けた会話行として整形する。
fn assistant_message_line(line: &str, first_body_line: bool) -> Line<'static> {
    let lead = if first_body_line {
        ASSISTANT_REPLY_MARKER
    } else {
        ""
    };
    conversation_message_line(
        lead,
        line,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
        message_block_style(MessageBlockStyle::Assistant),
    )
}

/// 会話行(you / assistant)共通の整形。リード列を固定幅で空けてから本文 span を並べる。
fn conversation_message_line(
    lead: &str,
    line: &str,
    lead_style: Style,
    text_style: Style,
) -> Line<'static> {
    let lead_width = lead.chars().count();
    let gap = CHAT_LEAD_WIDTH.saturating_sub(lead_width);
    let mut spans = vec![
        Span::styled(lead.to_string(), lead_style),
        Span::styled(" ".repeat(gap), text_style),
    ];
    if !line.is_empty() {
        spans.push(Span::styled(line.to_string(), text_style));
    }
    Line::from(spans)
}

/// system 行(action / waiting / error)の整形。tone に応じた RAIL と小文字ラベルを
/// 先頭行にだけ付け、続行はラベルを空にして本文を揃える。
fn system_message_line(
    style: MessageBlockStyle,
    label: &str,
    line: &str,
    first_body_line: bool,
) -> Line<'static> {
    let tone = message_block_tone(style);
    let label_style = Style::default()
        .fg(style::rail_color(tone))
        .add_modifier(Modifier::DIM);
    let text_style = message_block_style(style);
    let display_label = if first_body_line {
        label.to_ascii_lowercase()
    } else {
        String::new()
    };
    let mut spans = vec![
        Span::styled(
            style::RAIL.to_string(),
            Style::default().fg(style::rail_color(tone)),
        ),
        Span::raw(" "),
        Span::styled(format!("{display_label:<SYSTEM_LABEL_WIDTH$}"), label_style),
    ];
    if !line.is_empty() {
        spans.push(Span::styled(line.to_string(), text_style));
    }
    Line::from(spans)
}

/// 表示分類を panel tone(配色トーン)へ写像する。error は blocked、waiting は dim。
fn message_block_tone(style: MessageBlockStyle) -> PanelTone {
    match style {
        MessageBlockStyle::Action => PanelTone::Neutral,
        MessageBlockStyle::Waiting => PanelTone::Dim,
        MessageBlockStyle::Error => PanelTone::Blocked,
        MessageBlockStyle::User | MessageBlockStyle::Assistant => PanelTone::Neutral,
    }
}

/// 表示分類ごとの本文テキストスタイル。会話行は白、system 行は既定色に委ねる。
fn message_block_style(style: MessageBlockStyle) -> Style {
    match style {
        MessageBlockStyle::User => Style::default().fg(Color::White),
        MessageBlockStyle::Assistant => Style::default().fg(Color::White),
        MessageBlockStyle::Action | MessageBlockStyle::Waiting | MessageBlockStyle::Error => {
            Style::default()
        }
    }
}
