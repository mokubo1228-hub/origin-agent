//! Agent memory harness(N1)。`~/.origin/memory/` の structured store を管理する。
//!
//! 安全契約(M-1〜M-8 / D-ME-1〜9):
//! - 書き先はこのモジュールが受け取る store dir 配下のみ(workspace・`~/.origin/memory.md`・
//!   config へ書く API は存在しない — M-1)。
//! - モデルは直接書かない。N1 の書き込みはユーザーの明示コマンド経由のみ(M-2 / D-ME-9)。
//! - never-store フィルタ(組み込み + user 追加)は無効化できない(M-3)。
//! - journal(observations.jsonl)は本文を持たない(id と文字数のみ — M-4)。
//! - 既定 off。undo / forget(tombstone)/ disable を常備(M-5)。
//! - 上限つき: entry 200 chars / preferences 全体 1,500 chars(M-7)。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{redact_sensitive_text, sha256_hex, timestamp_from_unix_seconds};

/// 1 entry の本文上限。
pub const MEMORY_ENTRY_MAX_CHARS: usize = 200;
/// preferences.md 全体の上限(M-7。超過は N3 の summarizer まで手動 forget で空ける)。
pub const MEMORY_PREFERENCES_MAX_CHARS: usize = 1_500;

const ENABLED_MARKER: &str = "enabled";
const PREFERENCES_FILE: &str = "preferences.md";
const IGNORED_FILE: &str = "ignored.md";
const JOURNAL_FILE: &str = "observations.jsonl";
const NEVER_STORE_FILE: &str = "never-store.txt";

/// 覚えている1件。text は validator 通過済み・redact 済みの単一行。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub text: String,
    pub kind: MemoryKind,
    pub created_at: String,
}

/// 記憶の種類。N1 で生成されるのは Preference のみ(他は将来 phase の先置き)。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryKind {
    Preference,
    ProjectNote,
    ProviderPreference,
    WorkflowPattern,
}

/// remember の結果。Refused* は「書かなかった理由」で、store は無傷。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryWriteOutcome {
    Written {
        entry: MemoryEntry,
        /// tombstone を明示 remember が打ち消した場合 true(notice に添える)。
        unforgotten: bool,
    },
    RefusedDisabled,
    RefusedSensitive,
    RefusedDuplicate,
    RefusedTooLong,
    RefusedFull,
}

/// 表示・注入用の読み取り結果。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub enabled: bool,
    pub entries: Vec<MemoryEntry>,
    pub chars_used: usize,
    pub last_operation: Option<String>,
}

/// 本番の store dir(`~/.origin/memory`)。テストは各関数に dir を注入する。
pub fn default_memory_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".origin").join("memory"))
}

/// store の現在状態を読む。壊れ行・未知行は無視して読み続ける(前方互換 — D-SYNC-4)。
pub fn load_snapshot(dir: &Path) -> MemorySnapshot {
    let enabled = dir.join(ENABLED_MARKER).exists();
    let body = fs::read_to_string(dir.join(PREFERENCES_FILE)).unwrap_or_default();
    let entries = parse_preferences(&body);
    let chars_used = body.chars().count();
    let last_operation = fs::read_to_string(dir.join(JOURNAL_FILE))
        .ok()
        .and_then(|journal| {
            journal.lines().rev().find_map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).ok()?;
                let op = value.get("op")?.as_str()?.to_string();
                let id = value.get("id").and_then(|id| id.as_str()).unwrap_or("-");
                Some(format!("{op} [{id}]"))
            })
        });
    MemorySnapshot {
        enabled,
        entries,
        chars_used,
        last_operation,
    }
}

/// harness を有効化する(dir 作成 + marker。explicit consent の記録)。
pub fn set_enabled(dir: &Path, enabled: bool) -> Result<(), String> {
    if enabled {
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        fs::write(dir.join(ENABLED_MARKER), "").map_err(|error| error.to_string())?;
        journal(dir, "enabled", None, 0, None);
    } else {
        // store は消さない(注入と書き込みが止まるだけ)。marker 不在なら何もしない。
        let marker = dir.join(ENABLED_MARKER);
        if marker.exists() {
            fs::remove_file(&marker).map_err(|error| error.to_string())?;
        }
        journal(dir, "disabled", None, 0, None);
    }
    Ok(())
}

/// 明示 remember(N1 唯一の書き込み経路)。validator を通った時だけ preferences に追記する。
/// tombstone 済み id はユーザーの最新の明示意図が勝つ(打ち消して書く — D-ME §7.1)。
pub fn remember(
    dir: &Path,
    text: &str,
    extra_never_store: &[String],
) -> Result<MemoryWriteOutcome, String> {
    let snapshot = load_snapshot(dir);
    if !snapshot.enabled {
        return Ok(MemoryWriteOutcome::RefusedDisabled);
    }

    let normalized = normalize_text(text);
    if normalized.is_empty() || normalized.chars().count() > MEMORY_ENTRY_MAX_CHARS {
        journal(
            dir,
            "refused",
            None,
            normalized.chars().count(),
            Some("too_long"),
        );
        return Ok(MemoryWriteOutcome::RefusedTooLong);
    }
    if is_sensitive(&normalized, dir, extra_never_store) {
        // 本文を journal にも echo しない(M-3/M-4)。
        journal(
            dir,
            "refused",
            None,
            normalized.chars().count(),
            Some("sensitive"),
        );
        return Ok(MemoryWriteOutcome::RefusedSensitive);
    }

    let id = entry_id(&normalized);
    if snapshot.entries.iter().any(|entry| entry.id == id) {
        journal(
            dir,
            "refused",
            Some(&id),
            normalized.chars().count(),
            Some("duplicate"),
        );
        return Ok(MemoryWriteOutcome::RefusedDuplicate);
    }

    let new_line = format_entry_line(&id, &normalized);
    if snapshot.chars_used + new_line.chars().count() + 1 > MEMORY_PREFERENCES_MAX_CHARS {
        journal(
            dir,
            "refused",
            Some(&id),
            normalized.chars().count(),
            Some("full"),
        );
        return Ok(MemoryWriteOutcome::RefusedFull);
    }

    let unforgotten = tombstone_active(dir, &id);
    if unforgotten {
        append_line(dir, IGNORED_FILE, &format!("~[{id}]"))?;
        journal(dir, "unforgotten", Some(&id), 0, None);
    }

    append_line(dir, PREFERENCES_FILE, &new_line)?;
    journal(dir, "written", Some(&id), normalized.chars().count(), None);

    Ok(MemoryWriteOutcome::Written {
        entry: MemoryEntry {
            id,
            text: normalized,
            kind: MemoryKind::Preference,
            created_at: now_timestamp(),
        },
        unforgotten,
    })
}

/// entry を削除し tombstone を残す(自動再学習の防止 — M-5)。
pub fn forget(dir: &Path, id: &str) -> Result<MemoryEntry, String> {
    let snapshot = load_snapshot(dir);
    let Some(entry) = snapshot
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
    else {
        return Err(format!(
            "no memory entry with id [{id}] — see /memory for ids"
        ));
    };
    rewrite_preferences_without(dir, id)?;
    append_line(dir, IGNORED_FILE, &format!("[{id}]"))?;
    journal(dir, "forgotten", Some(id), 0, None);
    Ok(entry)
}

/// 直近の written 1件を取り消す(journal 起点。対象が既に無ければ None)。
pub fn undo(dir: &Path) -> Result<Option<MemoryEntry>, String> {
    let journal_body = fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap_or_default();
    let snapshot = load_snapshot(dir);
    for line in journal_body.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match value.get("op").and_then(|op| op.as_str()) {
            Some("undone") => {
                // 直近が undo 済みならそれより前の written は対象にしない(深さ1)。
                return Ok(None);
            }
            Some("written") => {
                let Some(id) = value.get("id").and_then(|id| id.as_str()) else {
                    return Ok(None);
                };
                let Some(entry) = snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                    .cloned()
                else {
                    return Ok(None);
                };
                rewrite_preferences_without(dir, id)?;
                journal(dir, "undone", Some(id), 0, None);
                return Ok(Some(entry));
            }
            _ => continue,
        }
    }
    Ok(None)
}

/// 注入用の本文(entries を `- text` の行に整形)。enabled かつ1件以上の時だけ Some。
pub fn injection_body(snapshot: &MemorySnapshot) -> Option<String> {
    if !snapshot.enabled || snapshot.entries.is_empty() {
        return None;
    }
    Some(
        snapshot
            .entries
            .iter()
            .map(|entry| format!("- {}", entry.text))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

// ---- 内部実装 ---------------------------------------------------------------

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn entry_id(normalized: &str) -> String {
    sha256_hex(normalized.as_bytes())[..8].to_string()
}

fn format_entry_line(id: &str, text: &str) -> String {
    format!("- [{id}] {text}")
}

fn parse_preferences(body: &str) -> Vec<MemoryEntry> {
    body.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("- [")?;
            let (id, text) = rest.split_once("] ")?;
            if id.len() != 8 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            Some(MemoryEntry {
                id: id.to_string(),
                text: text.to_string(),
                kind: MemoryKind::Preference,
                created_at: String::new(),
            })
        })
        .collect()
}

/// tombstone が有効か(`[id]` があり、その後に `~[id]` の打ち消しが無い)。
fn tombstone_active(dir: &Path, id: &str) -> bool {
    let body = fs::read_to_string(dir.join(IGNORED_FILE)).unwrap_or_default();
    let mut active = false;
    for line in body.lines() {
        if line.trim() == format!("[{id}]") {
            active = true;
        } else if line.trim() == format!("~[{id}]") {
            active = false;
        }
    }
    active
}

/// 組み込み + user の never-store 判定(無効化不可 — M-3)。
fn is_sensitive(text: &str, dir: &Path, extra_never_store: &[String]) -> bool {
    // redaction が反応する文字列はそもそも保存しない(既存不変条件と同じ判定源)。
    if redact_sensitive_text(text) != text {
        return true;
    }
    let lower = text.to_lowercase();
    const BUILTIN: &[&str] = &[
        "api key",
        "api_key",
        "api-key",
        "authorization",
        "bearer",
        "password",
        "passphrase",
        "secret",
        "token",
        "credential",
        "private key",
        "openai_api_key",
    ];
    if BUILTIN.iter().any(|pattern| lower.contains(pattern)) {
        return true;
    }
    // 長い hex / base64 風の連続は鍵の疑いとして拒否する。
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' {
            run += 1;
            if run >= 32 && text.chars().filter(|c| c.is_ascii_hexdigit()).count() >= 32 {
                return true;
            }
            if run >= 40 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    // user 追加の never-store(policy の [memory] never_store と never-store.txt の union)。
    let user_file = fs::read_to_string(dir.join(NEVER_STORE_FILE)).unwrap_or_default();
    user_file
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .chain(extra_never_store.iter().map(String::as_str))
        .any(|pattern| !pattern.is_empty() && lower.contains(&pattern.to_lowercase()))
}

fn append_line(dir: &Path, file: &str, line: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join(file);
    let mut body = fs::read_to_string(&path).unwrap_or_default();
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(line);
    body.push('\n');
    write_atomic(&path, &body)
}

fn rewrite_preferences_without(dir: &Path, id: &str) -> Result<(), String> {
    let path = dir.join(PREFERENCES_FILE);
    let body = fs::read_to_string(&path).unwrap_or_default();
    let kept: Vec<&str> = body
        .lines()
        .filter(|line| !line.starts_with(&format!("- [{id}] ")))
        .collect();
    let mut new_body = kept.join("\n");
    if !new_body.is_empty() {
        new_body.push('\n');
    }
    write_atomic(&path, &new_body)
}

/// temp file + rename の atomic 置換(半端な書き込みを残さない)。
fn write_atomic(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body).map_err(|error| error.to_string())?;
    fs::rename(&tmp, path).map_err(|error| error.to_string())
}

/// journal 追記(失敗しても主操作は成立させる — best effort)。本文は書かない。
fn journal(dir: &Path, op: &str, id: Option<&str>, chars: usize, reason: Option<&str>) {
    let line = serde_json::json!({
        "at": now_timestamp(),
        "op": op,
        "id": id,
        "kind": "preference",
        "chars": chars,
        "reason": reason,
    });
    let _ = fs::create_dir_all(dir);
    let path = dir.join(JOURNAL_FILE);
    let mut body = fs::read_to_string(&path).unwrap_or_default();
    body.push_str(&line.to_string());
    body.push('\n');
    let _ = fs::write(&path, body);
}

fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    timestamp_from_unix_seconds(seconds).rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("origin-memory-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn enabled_store(name: &str) -> PathBuf {
        let dir = temp_store(name);
        set_enabled(&dir, true).unwrap();
        dir
    }

    #[test]
    fn memory_remember_writes_entry_and_journal() {
        let dir = enabled_store("write");
        let outcome = remember(&dir, "回答は簡潔に", &[]).unwrap();
        let MemoryWriteOutcome::Written { entry, unforgotten } = outcome else {
            panic!("should write");
        };
        assert!(!unforgotten);
        assert_eq!(entry.text, "回答は簡潔に");

        let body = fs::read_to_string(dir.join(PREFERENCES_FILE)).unwrap();
        assert!(body.contains(&format!("- [{}] 回答は簡潔に", entry.id)));
        let journal_body = fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
        assert!(journal_body.contains("\"op\":\"written\""));
        assert!(!journal_body.contains("回答は簡潔に"));
    }

    #[test]
    fn memory_remember_refuses_when_disabled() {
        let dir = temp_store("disabled");
        assert_eq!(
            remember(&dir, "note", &[]).unwrap(),
            MemoryWriteOutcome::RefusedDisabled
        );
        assert!(!dir.join(PREFERENCES_FILE).exists());
    }

    #[test]
    fn memory_remember_refuses_sensitive_patterns() {
        let dir = enabled_store("sensitive");
        for text in [
            "api_key=abc",
            "Authorization: something",
            "Bearer abc",
            "my password is hunter2",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ] {
            assert_eq!(
                remember(&dir, text, &[]).unwrap(),
                MemoryWriteOutcome::RefusedSensitive,
                "{text}"
            );
        }
        let journal_body = fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
        assert!(journal_body.contains("\"reason\":\"sensitive\""));
        assert!(!journal_body.contains("hunter2"));
        assert!(!dir.join(PREFERENCES_FILE).exists());
    }

    #[test]
    fn memory_remember_refuses_duplicate_and_too_long_and_full() {
        let dir = enabled_store("refusals");
        remember(&dir, "note one", &[]).unwrap();
        assert_eq!(
            remember(&dir, "note  one", &[]).unwrap(),
            MemoryWriteOutcome::RefusedDuplicate
        );
        assert_eq!(
            remember(&dir, &"x".repeat(201), &[]).unwrap(),
            MemoryWriteOutcome::RefusedTooLong
        );
        for index in 0..12 {
            let _ = remember(&dir, &format!("filler {index} {}", "word ".repeat(20)), &[]);
        }
        assert_eq!(
            remember(&dir, "another note ".repeat(12).trim(), &[]).unwrap(),
            MemoryWriteOutcome::RefusedFull
        );
    }

    #[test]
    fn memory_forget_removes_entry_and_writes_tombstone() {
        let dir = enabled_store("forget");
        let MemoryWriteOutcome::Written { entry, .. } = remember(&dir, "to forget", &[]).unwrap()
        else {
            panic!()
        };
        forget(&dir, &entry.id).unwrap();
        assert!(load_snapshot(&dir).entries.is_empty());
        let ignored = fs::read_to_string(dir.join(IGNORED_FILE)).unwrap();
        assert!(ignored.contains(&format!("[{}]", entry.id)));
        assert!(forget(&dir, "ffffffff").is_err());
    }

    #[test]
    fn memory_forget_prevents_relearn_but_explicit_remember_overrides() {
        let dir = enabled_store("override");
        let MemoryWriteOutcome::Written { entry, .. } = remember(&dir, "keep me", &[]).unwrap()
        else {
            panic!()
        };
        forget(&dir, &entry.id).unwrap();
        assert!(tombstone_active(&dir, &entry.id));

        let MemoryWriteOutcome::Written { unforgotten, .. } =
            remember(&dir, "keep me", &[]).unwrap()
        else {
            panic!()
        };
        assert!(unforgotten);
        assert!(!tombstone_active(&dir, &entry.id));
        let journal_body = fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
        assert!(journal_body.contains("\"op\":\"unforgotten\""));
    }

    #[test]
    fn memory_undo_removes_last_written_entry() {
        let dir = enabled_store("undo");
        remember(&dir, "first", &[]).unwrap();
        remember(&dir, "second", &[]).unwrap();
        let undone = undo(&dir).unwrap().expect("should undo");
        assert_eq!(undone.text, "second");
        assert_eq!(load_snapshot(&dir).entries.len(), 1);
        assert!(undo(&dir).unwrap().is_none());
    }

    #[test]
    fn memory_snapshot_tolerates_unknown_lines() {
        let dir = enabled_store("tolerant");
        remember(&dir, "known", &[]).unwrap();
        append_line(&dir, PREFERENCES_FILE, "# a future section").unwrap();
        append_line(&dir, PREFERENCES_FILE, "not an entry").unwrap();
        let snapshot = load_snapshot(&dir);
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].text, "known");
    }

    #[test]
    fn memory_never_store_user_patterns_apply() {
        let dir = enabled_store("user-patterns");
        fs::write(dir.join(NEVER_STORE_FILE), "社外秘\n").unwrap();
        assert_eq!(
            remember(&dir, "これは社外秘の話", &[]).unwrap(),
            MemoryWriteOutcome::RefusedSensitive
        );
        // policy 由来の追加パターン(P2)も同じ関門を通る。
        assert_eq!(
            remember(&dir, "codename phoenix", &["phoenix".to_string()]).unwrap(),
            MemoryWriteOutcome::RefusedSensitive
        );
    }

    #[test]
    fn memory_enable_disable_toggles_marker_only() {
        let dir = enabled_store("toggle");
        remember(&dir, "persists", &[]).unwrap();
        set_enabled(&dir, false).unwrap();
        let snapshot = load_snapshot(&dir);
        assert!(!snapshot.enabled);
        assert_eq!(snapshot.entries.len(), 1);
        assert!(injection_body(&snapshot).is_none());
        set_enabled(&dir, true).unwrap();
        assert!(injection_body(&load_snapshot(&dir)).is_some());
    }
}
