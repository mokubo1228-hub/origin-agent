//! `/resume` resume picker の表示行と選択状態。
//! 候補は origin-session の shared 型(redact 済み)をそのまま表示するだけで、
//! ここから provider 呼び出し・ファイル変更・状態復元は一切起こさない(read-only UI)。

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use origin_session::{ResumeCandidate, ResumeOutcome};

use super::{fit_to_width, truncate_middle};

/// picker が同時に表示する候補の上限。ローダ側も同じ件数で切る(契約 D-RS-3)。
pub(super) const RESUME_PICKER_MAX_ROWS: usize = 7;

/// resume picker の表示・選択状態。候補一覧と選択位置、数字入力バッファを持つ。
/// 選択はまだ何もしない(Enter で lightweight resume に合流するまで read-only)。
pub(super) struct ResumePicker {
    pub(super) candidates: Vec<ResumeCandidate>,
    pub(super) selected: usize,
    pub(super) buffer: String,
}

/// 1 候補が占める表示行の範囲 [start, end)。scroll 開始位置の計算に使う。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResumeCandidateLineRange {
    start: usize,
    end: usize,
}

/// テスト用:行範囲を捨てて表示行だけ取り出す薄いラッパ。
#[cfg(test)]
pub(super) fn resume_picker_lines(picker: &ResumePicker, width: u16) -> Vec<Line<'static>> {
    resume_picker_lines_with_ranges(picker, width).0
}

/// 表示高さに収まる範囲だけ picker 行を返す。全行が収まるならそのまま、
/// 溢れる時は選択中候補が見える位置から窓を切り出す(model picker と同じ手口)。
pub(super) fn resume_picker_visible_lines(
    picker: &ResumePicker,
    width: u16,
    visible_height: u16,
) -> Vec<Line<'static>> {
    let (lines, ranges) = resume_picker_lines_with_ranges(picker, width);
    let visible_height = visible_height as usize;
    if visible_height == 0 || lines.len() <= visible_height {
        return lines;
    }

    let start = resume_picker_scroll_start(&ranges, picker.selected, visible_height);
    lines.into_iter().skip(start).take(visible_height).collect()
}

/// 選択中候補が表示窓に収まるよう scroll の開始行を決める。
fn resume_picker_scroll_start(
    ranges: &[ResumeCandidateLineRange],
    selected_index: usize,
    visible_height: usize,
) -> usize {
    if visible_height == 0 || ranges.is_empty() {
        return 0;
    }

    let selected = ranges
        .get(selected_index.min(ranges.len().saturating_sub(1)))
        .copied()
        .unwrap_or(ResumeCandidateLineRange { start: 0, end: 0 });
    if selected.end.saturating_sub(selected.start) >= visible_height {
        return selected.start;
    }

    selected
        .end
        .saturating_sub(visible_height)
        .min(selected.start)
}

/// picker 全体の表示行と、各候補の行範囲を組み立てる中心関数。
/// 1候補 = 2行(番号+相対時刻+intent / dim のメタ行)。選択行は cyan + bold のみで示し、
/// literal marker は使わない。末尾に read-only 契約の2行を常に出す。
fn resume_picker_lines_with_ranges(
    picker: &ResumePicker,
    width: u16,
) -> (Vec<Line<'static>>, Vec<ResumeCandidateLineRange>) {
    let mut lines = vec![Line::from("Resume from a recent session"), Line::from("")];
    let mut ranges = Vec::new();
    let now = SystemTime::now();
    let visible_count = picker.candidates.len().min(RESUME_PICKER_MAX_ROWS);
    let intent_width = (width.max(24) as usize).saturating_sub(17).max(12);

    for (index, candidate) in picker
        .candidates
        .iter()
        .take(RESUME_PICKER_MAX_ROWS)
        .enumerate()
    {
        let selected = index == picker.selected.min(visible_count.saturating_sub(1));
        let range_start = lines.len();
        let time_label = relative_time_label(&candidate.created_at, now);
        lines.push(super::style::selection_row(
            &format!("{}. ", index + 1),
            &format!(
                "{time_label:<10} {}",
                fit_to_width(&candidate.intent, intent_width)
            ),
            selected,
            width,
        ));
        lines.push(resume_candidate_metadata_line(candidate, width));
        ranges.push(ResumeCandidateLineRange {
            start: range_start,
            end: lines.len(),
        });
    }

    lines.push(Line::from(""));
    if picker.candidates.len() > RESUME_PICKER_MAX_ROWS {
        let hidden = picker.candidates.len() - RESUME_PICKER_MAX_ROWS;
        lines.push(dim_note_line(&format!(
            "{hidden} more — /sessions or `origin sessions` for full history"
        )));
    }
    lines.push(dim_note_line(
        "Resume uses the selected session as read-only background only.",
    ));
    lines.push(dim_note_line(
        "Nothing is restored: no proposal, no diffs, no files.",
    ));
    (lines, ranges)
}

/// 候補の補助情報(種別 · 結果 · model)を dim の1行に整形する。
fn resume_candidate_metadata_line(candidate: &ResumeCandidate, width: u16) -> Line<'static> {
    let metadata_width = (width.max(24) as usize).saturating_sub(4).max(12);
    let metadata = format!(
        "{} · {} · {}",
        candidate.kind,
        resume_outcome_label(candidate),
        truncate_middle(&candidate.model, 20)
    );
    Line::from(vec![
        Span::raw("    "),
        Span::styled(
            fit_to_width(&metadata, metadata_width),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ])
}

/// 末尾の契約行・`N more` 行に使う dim の注釈行。新しい色は導入しない。
fn dim_note_line(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            text.to_string(),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ])
}

/// 候補の outcome + files を「あの時保存したっけ」に直接答える1語ラベルへ合成する。
pub(super) fn resume_outcome_label(candidate: &ResumeCandidate) -> String {
    match candidate.outcome {
        ResumeOutcome::Applied => match candidate.files.as_slice() {
            [] => "saved".to_string(),
            [file] => format!("saved {file}"),
            [first, rest @ ..] => format!("saved {first} +{}", rest.len()),
        },
        ResumeOutcome::Rejected => "rejected".to_string(),
        ResumeOutcome::Cancelled => "cancelled / no files changed".to_string(),
        ResumeOutcome::ReadOnly => "read-only".to_string(),
        ResumeOutcome::Error => "error".to_string(),
        ResumeOutcome::Unknown => "unknown".to_string(),
    }
}

/// created_at(SessionStarted が書く `YYYY-MM-DDTHH:MM:SSZ`)を相対時刻ラベルへ写像する。
/// now は引数で受ける(テストは固定時刻を注入する。SystemTime::now() 依存の assertion を書かない)。
pub(super) fn relative_time_label(created_at: &str, now: SystemTime) -> String {
    let Some(created_seconds) = parse_rfc3339_utc_seconds(created_at) else {
        return created_at.chars().take(10).collect();
    };
    let now_seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let elapsed = now_seconds.saturating_sub(created_seconds).max(0);
    if elapsed < 3_600 {
        format!("{}m ago", elapsed / 60)
    } else if elapsed < 86_400 {
        format!("{}h ago", elapsed / 3_600)
    } else if elapsed < 172_800 {
        "yesterday".to_string()
    } else if elapsed < 604_800 {
        format!("{}d ago", elapsed / 86_400)
    } else {
        created_at.chars().take(10).collect()
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` 固定書式を unix 秒へ手書き parse する(依存 crate は増やさない)。
/// 書式が合わなければ None(呼び出し側が生文字列 fallback を使う)。
fn parse_rfc3339_utc_seconds(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return None;
    }
    let year = value[0..4].parse::<i64>().ok()?;
    let month = value[5..7].parse::<i64>().ok()?;
    let day = value[8..10].parse::<i64>().ok()?;
    let hour = value[11..13].parse::<i64>().ok()?;
    let minute = value[14..16].parse::<i64>().ok()?;
    let second = value[17..19].parse::<i64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// 暦日から unix epoch 起点の日数を求める(origin-session の civil_from_days の逆変換)。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = if month <= 2 { year - 1 } else { year };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let yoe = adjusted_year - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
