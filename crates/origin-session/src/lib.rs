use origin_protocol::session::SessionEvent;
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: String,
    pub cwd: PathBuf,
    pub path: PathBuf,
}

pub struct SessionWriter {
    metadata: SessionMetadata,
    file: File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub session_id: String,
    pub created_at: String,
    pub command: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub summary: SessionSummary,
    pub events: Vec<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeContext {
    pub source_session_id: String,
    pub source_path: PathBuf,
    pub context: String,
}

/// resume picker が並べる1候補。UI 非依存の shared engine 型として置き、
/// ratatui 等の表示型・相対時刻ラベルは含めない(表示は各 surface 側で作る)。
/// 全 field は格納前に redact 済み。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResumeCandidate {
    pub session_id: String,
    pub source_path: PathBuf,
    pub created_at: String,
    pub kind: String,
    pub intent: String,
    pub files: Vec<String>,
    pub outcome: ResumeOutcome,
    pub model: String,
}

/// 候補 session の終端結果。Unknown は「transcript を読めなかった」プレースホルダ専用で、
/// イベント走査からは出ない。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ResumeOutcome {
    Applied,
    Rejected,
    Cancelled,
    ReadOnly,
    Error,
    Unknown,
}

#[derive(Debug)]
pub enum SessionError {
    HomeNotFound,
    Time(std::time::SystemTimeError),
    CreateDir {
        path: PathBuf,
        source: io::Error,
    },
    Open {
        path: PathBuf,
        source: io::Error,
    },
    ReadDir {
        path: PathBuf,
        source: io::Error,
    },
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    Serialize(serde_json::Error),
    Write {
        path: PathBuf,
        source: io::Error,
    },
    NotFound {
        session_id: String,
    },
    NoSessions,
}

impl SessionWriter {
    pub fn create(command: &str) -> Result<Self, SessionError> {
        let cwd = env::current_dir().map_err(|source| SessionError::Open {
            path: PathBuf::from("."),
            source,
        })?;
        let root = default_sessions_root()?;
        Self::create_in(root, cwd, command, SystemTime::now())
    }

    pub fn create_in(
        root: PathBuf,
        cwd: PathBuf,
        command: &str,
        now: SystemTime,
    ) -> Result<Self, SessionError> {
        let timestamp = timestamp_from_system_time(now)?;
        let id = generate_session_id(&timestamp);
        let path = session_file_path(&root, &timestamp, &id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| SessionError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|source| SessionError::Open {
                path: path.clone(),
                source,
            })?;
        let metadata = SessionMetadata {
            id,
            created_at: timestamp.rfc3339(),
            cwd,
            path,
        };
        let mut writer = Self { metadata, file };
        writer.record(SessionEvent::SessionStarted {
            session_id: writer.metadata.id.clone(),
            created_at: writer.metadata.created_at.clone(),
            cwd: writer.metadata.cwd.display().to_string(),
        })?;
        writer.record(SessionEvent::CommandStarted {
            command: command.to_string(),
        })?;
        Ok(writer)
    }

    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    pub fn path(&self) -> &Path {
        &self.metadata.path
    }

    pub fn record(&mut self, event: SessionEvent) -> Result<(), SessionError> {
        let value = serde_json::to_value(&event).map_err(SessionError::Serialize)?;
        let line = serde_json::to_string(&redact_value(value)).map_err(SessionError::Serialize)?;
        writeln!(self.file, "{line}").map_err(|source| SessionError::Write {
            path: self.metadata.path.clone(),
            source,
        })
    }
}

pub fn default_sessions_root() -> Result<PathBuf, SessionError> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(default_sessions_root_from_home)
        .ok_or(SessionError::HomeNotFound)
}

fn default_sessions_root_from_home(home: PathBuf) -> PathBuf {
    home.join(".origin").join("sessions")
}

pub fn list_sessions() -> Result<Vec<SessionSummary>, SessionError> {
    list_sessions_in(&default_sessions_root()?)
}

pub fn list_sessions_in(root: &Path) -> Result<Vec<SessionSummary>, SessionError> {
    let mut summaries = Vec::new();
    for path in discover_session_files(root)? {
        summaries.push(read_session(&path)?.summary);
    }
    summaries.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.session_id.cmp(&left.session_id))
    });
    Ok(summaries)
}

pub fn read_latest_session() -> Result<SessionRecord, SessionError> {
    read_latest_session_in(&default_sessions_root()?)
}

pub fn read_latest_session_in(root: &Path) -> Result<SessionRecord, SessionError> {
    let latest = list_sessions_in(root)?
        .into_iter()
        .next()
        .ok_or(SessionError::NoSessions)?;
    read_session(&latest.path)
}

pub fn read_session_by_id(session_id: &str) -> Result<SessionRecord, SessionError> {
    read_session_by_id_in(&default_sessions_root()?, session_id)
}

pub fn read_session_by_id_in(root: &Path, session_id: &str) -> Result<SessionRecord, SessionError> {
    for path in discover_session_files(root)? {
        if path.file_stem().and_then(|stem| stem.to_str()) == Some(session_id) {
            return read_session(&path);
        }
    }

    Err(SessionError::NotFound {
        session_id: session_id.to_string(),
    })
}

pub fn build_resume_context(target: &str) -> Result<ResumeContext, SessionError> {
    build_resume_context_in(&default_sessions_root()?, target)
}

pub fn build_resume_context_in(root: &Path, target: &str) -> Result<ResumeContext, SessionError> {
    build_resume_context_excluding_in(root, target, None)
}

pub fn build_resume_context_excluding(
    target: &str,
    excluded_path: &Path,
) -> Result<ResumeContext, SessionError> {
    build_resume_context_excluding_in(&default_sessions_root()?, target, Some(excluded_path))
}

pub fn build_resume_context_excluding_in(
    root: &Path,
    target: &str,
    excluded_path: Option<&Path>,
) -> Result<ResumeContext, SessionError> {
    let record = if target == "latest" {
        let latest = list_sessions_in(root)?
            .into_iter()
            .filter(|summary| {
                excluded_path
                    .map(|path| summary.path != path)
                    .unwrap_or(true)
            })
            .next()
            .ok_or(SessionError::NoSessions)?;
        read_session(&latest.path)?
    } else {
        read_session_by_id_in(root, target)?
    };

    Ok(resume_context_from_record(&record))
}

/// default root 版。実体は resume_candidates_in。
pub fn resume_candidates(limit: usize) -> Result<Vec<ResumeCandidate>, SessionError> {
    resume_candidates_in(&default_sessions_root()?, limit)
}

/// 直近 limit 件の resume 候補を新しい順に返す。read-only(何も書かない・provider を呼ばない)。
/// - root 不在はエラーではなく空。
/// - session ファイル名は timestamp 始まりなので、path 降順 = 新しい順として
///   先頭 limit 件だけを読む(全ファイルを読まない)。
/// - 読めない transcript は隠さず、その位置に `(unreadable session)` プレースホルダを残す
///   (隠すと「latest が picker に出ない」混乱を生むため)。
pub fn resume_candidates_in(
    root: &Path,
    limit: usize,
) -> Result<Vec<ResumeCandidate>, SessionError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    for path in discover_session_files(root)?.into_iter().rev().take(limit) {
        match read_session(&path) {
            Ok(record) => candidates.push(resume_candidate_from_record(&record)),
            Err(_) => candidates.push(unreadable_resume_candidate(&path)),
        }
    }
    Ok(candidates)
}

/// intent(代表 user 発話)の上限文字数。1行に畳んでからこの長さで末尾切りする。
const RESUME_CANDIDATE_INTENT_MAX_CHARS: usize = 80;
/// files に載せる読み取り path の最大件数(patch_applied が無い場合)。
const RESUME_CANDIDATE_MAX_READ_FILES: usize = 3;

/// 1 transcript から resume 候補を導出する(イベント走査 + redact。
/// resume_context_from_record と同じ手法の候補版)。
pub fn resume_candidate_from_record(record: &SessionRecord) -> ResumeCandidate {
    let mut intent: Option<String> = None;
    let mut applied_files: Vec<String> = Vec::new();
    let mut read_files: Vec<String> = Vec::new();
    let mut patch_applied = false;
    let mut patch_rejected = false;
    let mut approval_rejected = false;
    let mut command_failed = false;
    let mut diff_proposed = false;

    for event in &record.events {
        match event_type(event) {
            Some("user_message") => {
                if intent.is_none() {
                    if let Some(text) = string_field(event, "text") {
                        intent = Some(text.to_string());
                    }
                }
            }
            Some("file_read") | Some("file_context") => {
                if let Some(path) = string_field(event, "path") {
                    if !read_files.iter().any(|existing| existing == path) {
                        read_files.push(path.to_string());
                    }
                }
            }
            Some("patch_applied") => {
                patch_applied = true;
                if let Some(files) = event.get("files").and_then(|value| value.as_array()) {
                    for file in files.iter().filter_map(|value| value.as_str()) {
                        applied_files.push(file.to_string());
                    }
                }
            }
            Some("patch_rejected") => patch_rejected = true,
            Some("approval_decision") => {
                if let Some(decision) = string_field(event, "decision") {
                    if decision.to_ascii_lowercase().contains("reject") {
                        approval_rejected = true;
                    }
                }
            }
            Some("command_failed") => command_failed = true,
            Some("diff_proposed") => diff_proposed = true,
            _ => {}
        }
    }

    let outcome = if patch_applied {
        ResumeOutcome::Applied
    } else if patch_rejected || approval_rejected {
        ResumeOutcome::Rejected
    } else if command_failed || record.summary.status == "error" {
        ResumeOutcome::Error
    } else if diff_proposed {
        ResumeOutcome::Cancelled
    } else {
        ResumeOutcome::ReadOnly
    };

    let files = if patch_applied && !applied_files.is_empty() {
        applied_files
    } else {
        read_files.truncate(RESUME_CANDIDATE_MAX_READ_FILES);
        read_files
    };

    ResumeCandidate {
        session_id: record.summary.session_id.clone(),
        source_path: record.summary.path.clone(),
        created_at: record.summary.created_at.clone(),
        kind: resume_candidate_kind(&record.summary.command),
        intent: resume_candidate_intent(intent.as_deref().unwrap_or("(no message)")),
        files: files
            .iter()
            .map(|file| redact_sensitive_text(file))
            .collect(),
        outcome,
        model: redact_sensitive_text(&record.summary.model),
    }
}

/// 読めなかった transcript のプレースホルダ候補。存在は隠さず、内容だけ unknown にする。
fn unreadable_resume_candidate(path: &Path) -> ResumeCandidate {
    ResumeCandidate {
        session_id: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown")
            .to_string(),
        source_path: path.to_path_buf(),
        created_at: "unknown".to_string(),
        kind: "unknown".to_string(),
        intent: "(unreadable session)".to_string(),
        files: Vec::new(),
        outcome: ResumeOutcome::Unknown,
        model: "unknown".to_string(),
    }
}

/// command 文字列を picker の kind 表示へ写像する(mode は使わない)。
fn resume_candidate_kind(command: &str) -> String {
    match command {
        "chat" => "chat".to_string(),
        "agent-plan" => "agent".to_string(),
        "propose" => "propose".to_string(),
        "apply" => "apply".to_string(),
        other => redact_sensitive_text(&other.to_ascii_lowercase()),
    }
}

/// 代表 intent を redact → 空白畳み込み → 先頭80 chars に整形する。
fn resume_candidate_intent(text: &str) -> String {
    let compact = redact_sensitive_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() > RESUME_CANDIDATE_INTENT_MAX_CHARS {
        compact
            .chars()
            .take(RESUME_CANDIDATE_INTENT_MAX_CHARS)
            .collect()
    } else {
        compact
    }
}

pub fn resume_context_from_record(record: &SessionRecord) -> ResumeContext {
    let mut user_messages = Vec::new();
    let mut files = Vec::new();
    let mut diff_proposed = false;
    let mut approval_decision = None;
    let mut patch_applied = false;
    let mut patch_rejected = false;

    for event in &record.events {
        match event_type(event) {
            Some("user_message") => {
                if let Some(text) = string_field(event, "text") {
                    user_messages.push(truncate_for_context(text, 500));
                }
            }
            Some("file_read") => {
                files.push(format!(
                    "{} bytes={} sha256={}",
                    string_field(event, "path").unwrap_or("unknown"),
                    number_field(event, "bytes").unwrap_or(0),
                    string_field(event, "sha256").unwrap_or("unknown")
                ));
            }
            Some("file_context") => {
                let path = string_field(event, "path").unwrap_or("unknown");
                let status = string_field(event, "status").unwrap_or("unknown");
                let bytes = number_field(event, "bytes");
                let sha256 = string_field(event, "sha256");
                let reason = string_field(event, "reason");
                let mut line = format!("{path} status={status}");
                if let Some(bytes) = bytes {
                    line.push_str(&format!(" bytes={bytes}"));
                }
                if let Some(sha256) = sha256 {
                    line.push_str(&format!(" sha256={sha256}"));
                }
                if let Some(reason) = reason {
                    line.push_str(&format!(" reason={reason}"));
                }
                files.push(line);
            }
            Some("workspace_overview") => {
                files.push(format!(
                    "workspace_overview root={} kind={} listed={} included={} skipped={}",
                    string_field(event, "root").unwrap_or("unknown"),
                    string_field(event, "workspace_kind").unwrap_or("unknown"),
                    number_field(event, "listed_entries").unwrap_or(0),
                    number_field(event, "included_files").unwrap_or(0),
                    number_field(event, "skipped_items").unwrap_or(0)
                ));
            }
            Some("diff_proposed") => {
                diff_proposed = true;
            }
            Some("approval_decision") => {
                approval_decision = string_field(event, "decision").map(str::to_string);
            }
            Some("patch_applied") => {
                patch_applied = true;
            }
            Some("patch_rejected") => {
                patch_rejected = true;
            }
            _ => {}
        }
    }

    let mut context = String::new();
    context.push_str("Previous session context:\n");
    push_context_line(
        &mut context,
        "session_id",
        record.summary.session_id.as_str(),
    );
    push_context_line(&mut context, "command", record.summary.command.as_str());
    push_context_line(&mut context, "provider", record.summary.provider.as_str());
    push_context_line(&mut context, "model", record.summary.model.as_str());
    if user_messages.is_empty() {
        push_context_line(&mut context, "user_message", "unknown");
    } else {
        for message in user_messages.iter().take(3) {
            push_context_line(&mut context, "user_message", message);
        }
    }
    context.push_str("- files:\n");
    if files.is_empty() {
        context.push_str("  - none\n");
    } else {
        for file in files.iter().take(12) {
            context.push_str("  - ");
            context.push_str(file);
            context.push('\n');
        }
    }
    push_context_line(
        &mut context,
        "diff_proposed",
        if diff_proposed { "yes" } else { "no" },
    );
    push_context_line(
        &mut context,
        "approval_decision",
        approval_decision.as_deref().unwrap_or("none"),
    );
    push_context_line(
        &mut context,
        "patch_applied",
        if patch_applied { "yes" } else { "no" },
    );
    push_context_line(
        &mut context,
        "patch_rejected",
        if patch_rejected { "yes" } else { "no" },
    );
    push_context_line(&mut context, "status", record.summary.status.as_str());
    context.push('\n');
    context.push_str(
        "Use this only as background context. Do not assume the current files are unchanged. Read the current files provided in this request.",
    );

    let context = truncate_for_context(&redact_sensitive_text(&context), 4_000);

    ResumeContext {
        source_session_id: record.summary.session_id.clone(),
        source_path: record.summary.path.clone(),
        context,
    }
}

pub fn read_session(path: &Path) -> Result<SessionRecord, SessionError> {
    let contents = fs::read_to_string(path).map_err(|source| SessionError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut events = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<Value>(line).map_err(|source| SessionError::Parse {
            path: path.to_path_buf(),
            line: index + 1,
            source,
        })?;
        events.push(redact_value(event));
    }

    Ok(SessionRecord {
        summary: summarize_session(path, &events),
        events,
    })
}

pub fn discover_session_files(root: &Path) -> Result<Vec<PathBuf>, SessionError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    discover_session_files_inner(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn discover_session_files_inner(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), SessionError> {
    for entry in fs::read_dir(root).map_err(|source| SessionError::ReadDir {
        path: root.to_path_buf(),
        source,
    })? {
        let path = entry
            .map_err(|source| SessionError::ReadDir {
                path: root.to_path_buf(),
                source,
            })?
            .path();
        if path.is_dir() {
            discover_session_files_inner(&path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn summarize_session(path: &Path, events: &[Value]) -> SessionSummary {
    let mut summary = SessionSummary {
        session_id: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown")
            .to_string(),
        created_at: "unknown".to_string(),
        command: "unknown".to_string(),
        provider: "unknown".to_string(),
        model: "unknown".to_string(),
        status: "unknown".to_string(),
        path: path.to_path_buf(),
    };

    for event in events {
        match event_type(event) {
            Some("session_started") => {
                if let Some(session_id) = string_field(event, "session_id") {
                    summary.session_id = session_id.to_string();
                }
                if let Some(created_at) = string_field(event, "created_at") {
                    summary.created_at = created_at.to_string();
                }
            }
            Some("command_started") => {
                if let Some(command) = string_field(event, "command") {
                    summary.command = command.to_string();
                }
            }
            Some("provider_resolved") => {
                if let Some(provider) = string_field(event, "provider") {
                    summary.provider = provider.to_string();
                }
            }
            Some("model_resolved") => {
                if let Some(model) = string_field(event, "model") {
                    summary.model = model.to_string();
                }
            }
            Some("session_finished") => {
                if let Some(status) = string_field(event, "status") {
                    summary.status = status.to_string();
                }
            }
            _ => {}
        }
    }

    summary
}

pub fn format_event(event: &Value) -> String {
    match event_type(event) {
        Some("session_started") => format!(
            "- session_started {} {}",
            string_field(event, "session_id").unwrap_or("unknown"),
            string_field(event, "created_at").unwrap_or("unknown")
        ),
        Some("command_started") => format!(
            "- command_started {}",
            string_field(event, "command").unwrap_or("unknown")
        ),
        Some("user_message") => format!(
            "- user_message {}",
            string_field(event, "text").unwrap_or("")
        ),
        Some("resumed_from") => format!(
            "- resumed_from {} {}",
            string_field(event, "source_session_id").unwrap_or("unknown"),
            string_field(event, "source_path").unwrap_or("unknown")
        ),
        Some("file_read") => format!(
            "- file_read {} bytes={} sha256={}",
            string_field(event, "path").unwrap_or("unknown"),
            number_field(event, "bytes").unwrap_or(0),
            string_field(event, "sha256").unwrap_or("unknown")
        ),
        Some("file_context") => {
            let mut line = format!(
                "- file_context mode={} path={} status={}",
                string_field(event, "mode").unwrap_or("unknown"),
                string_field(event, "path").unwrap_or("unknown"),
                string_field(event, "status").unwrap_or("unknown")
            );
            if let Some(bytes) = number_field(event, "bytes") {
                line.push_str(&format!(" bytes={bytes}"));
            }
            if let Some(sha256) = string_field(event, "sha256") {
                line.push_str(&format!(" sha256={sha256}"));
            }
            if let Some(reason) = string_field(event, "reason") {
                line.push_str(&format!(" reason={reason}"));
            }
            line
        }
        Some("workspace_overview") => format!(
            "- workspace_overview mode={} root={} kind={} listed={} included={} skipped={}",
            string_field(event, "mode").unwrap_or("unknown"),
            string_field(event, "root").unwrap_or("unknown"),
            string_field(event, "workspace_kind").unwrap_or("unknown"),
            number_field(event, "listed_entries").unwrap_or(0),
            number_field(event, "included_files").unwrap_or(0),
            number_field(event, "skipped_items").unwrap_or(0)
        ),
        Some("provider_resolved") => format!(
            "- provider_resolved {} {}",
            string_field(event, "provider").unwrap_or("unknown"),
            string_field(event, "base_url").unwrap_or("unknown")
        ),
        Some("model_resolved") => format!(
            "- model_resolved {}",
            string_field(event, "model").unwrap_or("unknown")
        ),
        Some("model_response") => format!(
            "- model_response {}",
            string_field(event, "text").unwrap_or("")
        ),
        Some("diff_proposed") => format!(
            "- diff_proposed {}",
            string_field(event, "diff").unwrap_or("")
        ),
        Some("diff_validation_succeeded") => format!(
            "- diff_validation_succeeded requested={} proposed={}",
            event.get("requested_files").unwrap_or(&Value::Null),
            event.get("proposed_files").unwrap_or(&Value::Null)
        ),
        Some("diff_validation_failed") => format!(
            "- diff_validation_failed requested={} proposed={} reason={}",
            event.get("requested_files").unwrap_or(&Value::Null),
            event.get("proposed_files").unwrap_or(&Value::Null),
            string_field(event, "reason").unwrap_or("unknown")
        ),
        Some("diff_integrity_succeeded") => format!(
            "- diff_integrity_succeeded files={} hunks={}",
            number_field(event, "file_count").unwrap_or(0),
            number_field(event, "hunk_count").unwrap_or(0)
        ),
        Some("diff_integrity_failed") => format!(
            "- diff_integrity_failed reason={}",
            string_field(event, "reason").unwrap_or("unknown")
        ),
        Some("diff_retry_requested") => format!(
            "- diff_retry_requested reason={}",
            string_field(event, "reason").unwrap_or("unknown")
        ),
        Some("diff_retry_started") => format!(
            "- diff_retry_started attempt={}",
            number_field(event, "attempt").unwrap_or(0)
        ),
        Some("diff_retry_succeeded") => format!(
            "- diff_retry_succeeded attempt={}",
            number_field(event, "attempt").unwrap_or(0)
        ),
        Some("diff_retry_failed") => format!(
            "- diff_retry_failed attempt={} reason={}",
            number_field(event, "attempt").unwrap_or(0),
            string_field(event, "reason").unwrap_or("unknown")
        ),
        Some("apply_check_retry_requested") => format!(
            "- apply_check_retry_requested error={}",
            string_field(event, "error").unwrap_or("unknown")
        ),
        Some("apply_check_retry_started") => format!(
            "- apply_check_retry_started attempt={}",
            number_field(event, "attempt").unwrap_or(0)
        ),
        Some("apply_check_retry_succeeded") => format!(
            "- apply_check_retry_succeeded attempt={}",
            number_field(event, "attempt").unwrap_or(0)
        ),
        Some("apply_check_retry_failed") => format!(
            "- apply_check_retry_failed attempt={} reason={}",
            number_field(event, "attempt").unwrap_or(0),
            string_field(event, "reason").unwrap_or("unknown")
        ),
        Some("git_apply_check_started") => "- git_apply_check_started".to_string(),
        Some("git_apply_check_succeeded") => "- git_apply_check_succeeded".to_string(),
        Some("git_apply_check_failed") => format!(
            "- git_apply_check_failed {}",
            string_field(event, "error").unwrap_or("")
        ),
        Some("approval_requested") => "- approval_requested".to_string(),
        Some("approval_decision") => format!(
            "- approval_decision {}",
            string_field(event, "decision").unwrap_or("unknown")
        ),
        Some("patch_applied") => format!(
            "- patch_applied {}",
            event.get("files").unwrap_or(&Value::Null)
        ),
        Some("patch_rejected") => "- patch_rejected".to_string(),
        Some("command_failed") => format!(
            "- command_failed {}",
            string_field(event, "error").unwrap_or("")
        ),
        Some("session_finished") => format!(
            "- session_finished {}",
            string_field(event, "status").unwrap_or("unknown")
        ),
        Some(kind) => format!("- {kind} {event}"),
        None => format!("- unknown_event {event}"),
    }
}

pub fn redact_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_sensitive_text(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = if is_sensitive_key(&key) {
                        Value::String("[redacted]".to_string())
                    } else {
                        redact_value(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        value => value,
    }
}

pub fn redact_sensitive_text(text: &str) -> String {
    if text.contains("Bearer") {
        return "[redacted]".to_string();
    }

    text.replace("Authorization", "[redacted]")
        .replace("OPENAI_API_KEY", "[redacted]")
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized == "authorization"
        || normalized == "api_key"
        || normalized == "api_key_env_value"
        || normalized == "token"
}

fn event_type(event: &Value) -> Option<&str> {
    string_field(event, "type")
}

fn string_field<'a>(event: &'a Value, field: &str) -> Option<&'a str> {
    event.get(field).and_then(|value| value.as_str())
}

fn number_field(event: &Value, field: &str) -> Option<u64> {
    event.get(field).and_then(|value| value.as_u64())
}

fn push_context_line(context: &mut String, label: &str, value: &str) {
    context.push_str("- ");
    context.push_str(label);
    context.push_str(": ");
    context.push_str(value);
    context.push('\n');
}

fn truncate_for_context(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }

    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...[truncated]");
    truncated
}

pub fn session_file_path(root: &Path, timestamp: &SessionTimestamp, session_id: &str) -> PathBuf {
    root.join(format!("{:04}", timestamp.year))
        .join(format!("{:02}", timestamp.month))
        .join(format!("{session_id}.jsonl"))
}

pub fn generate_session_id(timestamp: &SessionTimestamp) -> String {
    let suffix = rand::random::<u32>() & 0x00ff_ffff;
    format!("{}-{suffix:06x}", timestamp.compact())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = digest(&SHA256, bytes);
    let mut output = String::with_capacity(64);
    for byte in digest.as_ref() {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionTimestamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl SessionTimestamp {
    pub fn compact(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    pub fn rfc3339(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

pub fn timestamp_from_system_time(now: SystemTime) -> Result<SessionTimestamp, SessionError> {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_err(SessionError::Time)?
        .as_secs();
    Ok(timestamp_from_unix_seconds(seconds))
}

pub fn timestamp_from_unix_seconds(seconds: u64) -> SessionTimestamp {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    SessionTimestamp {
        year,
        month,
        day,
        hour: (seconds_of_day / 3_600) as u32,
        minute: ((seconds_of_day % 3_600) / 60) as u32,
        second: (seconds_of_day % 60) as u32,
    }
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeNotFound => write!(f, "Could not find HOME for session transcript storage."),
            Self::Time(error) => write!(f, "Could not generate session timestamp: {error}"),
            Self::CreateDir { path, source } => write!(
                f,
                "Could not create session directory `{}`: {source}",
                path.display()
            ),
            Self::Open { path, source } => {
                write!(
                    f,
                    "Could not open session transcript `{}`: {source}",
                    path.display()
                )
            }
            Self::ReadDir { path, source } => {
                write!(
                    f,
                    "Could not read session directory `{}`: {source}",
                    path.display()
                )
            }
            Self::Read { path, source } => {
                write!(
                    f,
                    "Could not read session transcript `{}`: {source}",
                    path.display()
                )
            }
            Self::Parse { path, line, source } => write!(
                f,
                "Could not parse session transcript `{}` at line {line}: {source}",
                path.display()
            ),
            Self::Serialize(error) => write!(f, "Could not serialize session event: {error}"),
            Self::Write { path, source } => {
                write!(
                    f,
                    "Could not write session transcript `{}`: {source}",
                    path.display()
                )
            }
            Self::NotFound { session_id } => {
                write!(f, "Session `{session_id}` was not found.")
            }
            Self::NoSessions => write!(f, "No sessions found."),
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Time(error) => Some(error),
            Self::CreateDir { source, .. } => Some(source),
            Self::Open { source, .. } => Some(source),
            Self::ReadDir { source, .. } => Some(source),
            Self::Read { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Serialize(error) => Some(error),
            Self::Write { source, .. } => Some(source),
            Self::HomeNotFound | Self::NotFound { .. } | Self::NoSessions => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use origin_protocol::session::SessionEvent;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use super::{
        build_resume_context_excluding_in, build_resume_context_in,
        default_sessions_root_from_home, discover_session_files, format_event, generate_session_id,
        list_sessions_in, read_latest_session_in, read_session, redact_sensitive_text,
        resume_candidates_in, resume_context_from_record, session_file_path, sha256_hex,
        timestamp_from_unix_seconds, ResumeOutcome, SessionTimestamp, SessionWriter,
    };

    #[test]
    fn generates_session_id_with_timestamp_and_short_suffix() {
        let timestamp = SessionTimestamp {
            year: 2026,
            month: 5,
            day: 5,
            hour: 1,
            minute: 23,
            second: 45,
        };
        let id = generate_session_id(&timestamp);

        assert!(id.starts_with("20260505-012345-"));
        assert_eq!(id.len(), "20260505-012345-a1b2c3".len());
    }

    #[test]
    fn builds_session_path_under_year_and_month() {
        let root = PathBuf::from("/tmp/sessions");
        let timestamp = SessionTimestamp {
            year: 2026,
            month: 5,
            day: 5,
            hour: 1,
            minute: 23,
            second: 45,
        };

        let path = session_file_path(&root, &timestamp, "20260505-012345-a1b2c3");

        assert_eq!(
            path,
            PathBuf::from("/tmp/sessions/2026/05/20260505-012345-a1b2c3.jsonl")
        );
    }

    #[test]
    fn default_sessions_root_uses_origin_directory() {
        let path = default_sessions_root_from_home(PathBuf::from("/home/user"));

        assert_eq!(path, PathBuf::from("/home/user/.origin/sessions"));
    }

    #[test]
    fn converts_unix_timestamp_to_utc_timestamp() {
        let timestamp = timestamp_from_unix_seconds(1_777_944_225);

        assert_eq!(timestamp.compact(), "20260505-012345");
        assert_eq!(timestamp.rfc3339(), "2026-05-05T01:23:45Z");
    }

    #[test]
    fn computes_sha256_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn writes_jsonl_events_without_api_key_fields() {
        let root = temp_dir("jsonl");
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let mut writer = SessionWriter::create_in(
            root.join("sessions"),
            cwd,
            "chat",
            UNIX_EPOCH + Duration::from_secs(1_777_944_225),
        )
        .unwrap();

        writer
            .record(SessionEvent::ProviderResolved {
                provider: "openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
            })
            .unwrap();
        writer
            .record(SessionEvent::ModelResolved {
                model: "openai-explicit".to_string(),
            })
            .unwrap();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert!(transcript.contains("\"type\":\"session_started\""));
        assert!(transcript.contains("\"type\":\"provider_resolved\""));
        assert!(transcript.contains("\"provider\":\"openai\""));
        assert!(!transcript.contains("OPENAI_API_KEY"));
        assert!(!transcript.contains("Authorization"));
        assert!(!transcript.contains("Bearer"));
    }

    #[test]
    fn writes_jsonl_events_with_redacted_sensitive_text_values() {
        let root = temp_dir("jsonl-redaction");
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let mut writer = SessionWriter::create_in(
            root.join("sessions"),
            cwd,
            "propose",
            UNIX_EPOCH + Duration::from_secs(1_777_944_226),
        )
        .unwrap();

        writer
            .record(SessionEvent::UserMessage {
                text: "Authorization: Bearer user-secret from OPENAI_API_KEY".to_string(),
            })
            .unwrap();
        writer
            .record(SessionEvent::FileRead {
                path: "README.md".to_string(),
                bytes: 123,
                sha256: "abc123".to_string(),
            })
            .unwrap();
        writer
            .record(SessionEvent::DiffProposed {
                diff: "--- a/README.md\n+++ b/README.md\n+Bearer diff-secret".to_string(),
            })
            .unwrap();
        writer
            .record(SessionEvent::GitApplyCheckFailed {
                error: "Authorization: Bearer check-secret".to_string(),
            })
            .unwrap();
        writer
            .record(SessionEvent::CommandFailed {
                error: "OPENAI_API_KEY provider failure".to_string(),
            })
            .unwrap();
        drop(writer);

        let transcript = fs::read_to_string(first_jsonl(root.join("sessions"))).unwrap();
        assert!(transcript.contains("\"type\":\"file_read\""));
        assert!(transcript.contains("\"path\":\"README.md\""));
        assert!(transcript.contains("\"bytes\":123"));
        assert!(transcript.contains("\"sha256\":\"abc123\""));
        assert!(transcript.contains("[redacted]"));
        assert!(!transcript.contains("Authorization"));
        assert!(!transcript.contains("Bearer"));
        assert!(!transcript.contains("OPENAI_API_KEY"));
        assert!(!transcript.contains("user-secret"));
        assert!(!transcript.contains("diff-secret"));
        assert!(!transcript.contains("check-secret"));
    }

    #[test]
    fn discovers_session_files_recursively() {
        let root = temp_dir("discover");
        write_jsonl(
            &root,
            "2026/05/20260505-010101-aaaaaa.jsonl",
            &[r#"{"type":"session_finished","status":"success"}"#],
        );
        write_jsonl(
            &root,
            "2026/06/20260601-010101-bbbbbb.jsonl",
            &[r#"{"type":"session_finished","status":"success"}"#],
        );

        let files = discover_session_files(&root).unwrap();

        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("20260505-010101-aaaaaa.jsonl"));
        assert!(files[1].ends_with("20260601-010101-bbbbbb.jsonl"));
    }

    #[test]
    fn extracts_session_summary_from_jsonl() {
        let root = temp_dir("summary");
        let path = write_jsonl(
            &root,
            "2026/05/20260505-010101-aaaaaa.jsonl",
            &[
                r#"{"type":"session_started","session_id":"20260505-010101-aaaaaa","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"chat"}"#,
                r#"{"type":"provider_resolved","provider":"lmstudio","base_url":"http://localhost:1234/v1"}"#,
                r#"{"type":"model_resolved","model":"qwen"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );

        let record = read_session(&path).unwrap();

        assert_eq!(record.summary.session_id, "20260505-010101-aaaaaa");
        assert_eq!(record.summary.created_at, "2026-05-05T01:01:01Z");
        assert_eq!(record.summary.command, "chat");
        assert_eq!(record.summary.provider, "lmstudio");
        assert_eq!(record.summary.model, "qwen");
        assert_eq!(record.summary.status, "success");
    }

    #[test]
    fn selects_latest_session_by_created_at() {
        let root = temp_dir("latest");
        write_jsonl(
            &root,
            "2026/05/older.jsonl",
            &[
                r#"{"type":"session_started","session_id":"older","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );
        write_jsonl(
            &root,
            "2026/05/newer.jsonl",
            &[
                r#"{"type":"session_started","session_id":"newer","created_at":"2026-05-05T02:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );

        let latest = read_latest_session_in(&root).unwrap();

        assert_eq!(latest.summary.session_id, "newer");
    }

    #[test]
    fn lists_sessions_newest_first() {
        let root = temp_dir("list");
        write_jsonl(
            &root,
            "2026/05/older.jsonl",
            &[
                r#"{"type":"session_started","session_id":"older","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );
        write_jsonl(
            &root,
            "2026/05/newer.jsonl",
            &[
                r#"{"type":"session_started","session_id":"newer","created_at":"2026-05-05T02:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );

        let summaries = list_sessions_in(&root).unwrap();

        assert_eq!(summaries[0].session_id, "newer");
        assert_eq!(summaries[1].session_id, "older");
    }

    #[test]
    fn redacts_sensitive_output_when_reading_events() {
        let root = temp_dir("redact");
        let path = write_jsonl(
            &root,
            "2026/05/secret.jsonl",
            &[
                r#"{"type":"command_failed","error":"Authorization: Bearer secret from OPENAI_API_KEY"}"#,
            ],
        );

        let record = read_session(&path).unwrap();
        let displayed = format_event(&record.events[0]);

        assert!(!displayed.contains("Authorization"));
        assert!(!displayed.contains("Bearer"));
        assert!(!displayed.contains("OPENAI_API_KEY"));
        assert!(!displayed.contains("secret"));
    }

    #[test]
    fn builds_resume_context_for_latest_session() {
        let root = temp_dir("resume-latest");
        write_jsonl(
            &root,
            "2026/05/older.jsonl",
            &[
                r#"{"type":"session_started","session_id":"older","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );
        write_jsonl(
            &root,
            "2026/05/newer.jsonl",
            &[
                r#"{"type":"session_started","session_id":"newer","created_at":"2026-05-05T02:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"propose"}"#,
                r#"{"type":"provider_resolved","provider":"lmstudio","base_url":"http://localhost:1234/v1"}"#,
                r#"{"type":"model_resolved","model":"qwen"}"#,
                r#"{"type":"user_message","text":"fix typo"}"#,
                r#"{"type":"file_read","path":"README.md","bytes":123,"sha256":"abc"}"#,
                r#"{"type":"diff_proposed","diff":"--- a/README.md\n+++ b/README.md"}"#,
                r#"{"type":"approval_decision","decision":"rejected"}"#,
                r#"{"type":"patch_rejected"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );

        let resume = build_resume_context_in(&root, "latest").unwrap();

        assert_eq!(resume.source_session_id, "newer");
        assert!(resume.context.contains("- command: propose"));
        assert!(resume.context.contains("- user_message: fix typo"));
        assert!(resume.context.contains("README.md bytes=123 sha256=abc"));
        assert!(resume.context.contains("- diff_proposed: yes"));
        assert!(resume.context.contains("- approval_decision: rejected"));
        assert!(resume.context.contains("- patch_rejected: yes"));
    }

    #[test]
    fn latest_resume_can_exclude_current_session_path() {
        let root = temp_dir("resume-latest-excluding");
        let previous = write_jsonl(
            &root,
            "2026/05/previous.jsonl",
            &[
                r#"{"type":"session_started","session_id":"previous","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"chat"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );
        let current = write_jsonl(
            &root,
            "2026/05/current.jsonl",
            &[
                r#"{"type":"session_started","session_id":"current","created_at":"2026-05-05T02:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"chat"}"#,
            ],
        );

        let resume = build_resume_context_excluding_in(&root, "latest", Some(&current)).unwrap();

        assert_eq!(resume.source_session_id, "previous");
        assert_eq!(resume.source_path, previous);
    }

    #[test]
    fn builds_resume_context_by_session_id() {
        let root = temp_dir("resume-id");
        write_jsonl(
            &root,
            "2026/05/target.jsonl",
            &[
                r#"{"type":"session_started","session_id":"target","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"chat"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );

        let resume = build_resume_context_in(&root, "target").unwrap();

        assert_eq!(resume.source_session_id, "target");
        assert!(resume.context.contains("- command: chat"));
    }

    #[test]
    fn missing_resume_session_returns_not_found() {
        let root = temp_dir("resume-missing");
        let error = build_resume_context_in(&root, "missing").unwrap_err();

        assert_eq!(error.to_string(), "Session `missing` was not found.");
    }

    #[test]
    fn resume_context_redacts_sensitive_text_and_stays_bounded() {
        let root = temp_dir("resume-redact");
        let path = write_jsonl(
            &root,
            "2026/05/sensitive.jsonl",
            &[
                r#"{"type":"session_started","session_id":"sensitive","created_at":"2026-05-05T01:01:01Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"chat"}"#,
                r#"{"type":"provider_resolved","provider":"openai","base_url":"https://api.openai.com/v1"}"#,
                r#"{"type":"model_resolved","model":"zai-org-glm-5-1"}"#,
                r#"{"type":"user_message","text":"Authorization: Bearer topsecret from OPENAI_API_KEY"}"#,
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );
        let record = read_session(&path).unwrap();

        let resume = resume_context_from_record(&record);

        assert!(resume.context.chars().count() <= 4_014);
        assert!(!resume.context.contains("Authorization"));
        assert!(!resume.context.contains("Bearer"));
        assert!(!resume.context.contains("OPENAI_API_KEY"));
        assert!(!resume.context.contains("topsecret"));
    }

    #[test]
    fn formats_resumed_from_event() {
        let event = serde_json::json!({
            "type": "resumed_from",
            "source_session_id": "session-1",
            "source_path": "/sessions/session-1.jsonl",
            "context": "Previous session context"
        });

        assert_eq!(
            format_event(&event),
            "- resumed_from session-1 /sessions/session-1.jsonl"
        );
    }

    #[test]
    fn formats_diff_validation_events() {
        let succeeded = serde_json::json!({
            "type": "diff_validation_succeeded",
            "requested_files": ["README.md"],
            "proposed_files": ["README.md"]
        });
        let failed = serde_json::json!({
            "type": "diff_validation_failed",
            "requested_files": ["README.md"],
            "proposed_files": ["Cargo.toml"],
            "reason": "model proposed changes outside the requested file scope"
        });

        assert!(format_event(&succeeded).contains("diff_validation_succeeded"));
        assert!(format_event(&failed).contains("diff_validation_failed"));
        assert!(format_event(&failed).contains("Cargo.toml"));
    }

    #[test]
    fn formats_diff_integrity_events() {
        let succeeded = serde_json::json!({
            "type": "diff_integrity_succeeded",
            "file_count": 1,
            "hunk_count": 2
        });
        let failed = serde_json::json!({
            "type": "diff_integrity_failed",
            "reason": "hunk header line count does not match hunk body"
        });

        assert_eq!(
            format_event(&succeeded),
            "- diff_integrity_succeeded files=1 hunks=2"
        );
        assert!(format_event(&failed).contains("diff_integrity_failed"));
        assert!(format_event(&failed).contains("line count"));
    }

    #[test]
    fn formats_apply_check_retry_events() {
        let requested = serde_json::json!({
            "type": "apply_check_retry_requested",
            "error": "patch does not apply"
        });
        let started = serde_json::json!({
            "type": "apply_check_retry_started",
            "attempt": 1
        });
        let succeeded = serde_json::json!({
            "type": "apply_check_retry_succeeded",
            "attempt": 1
        });
        let failed = serde_json::json!({
            "type": "apply_check_retry_failed",
            "attempt": 1,
            "reason": "git apply --check failed"
        });

        assert!(format_event(&requested).contains("apply_check_retry_requested"));
        assert_eq!(
            format_event(&started),
            "- apply_check_retry_started attempt=1"
        );
        assert_eq!(
            format_event(&succeeded),
            "- apply_check_retry_succeeded attempt=1"
        );
        assert!(format_event(&failed).contains("apply_check_retry_failed"));
        assert!(format_event(&failed).contains("git apply --check failed"));
    }

    #[test]
    fn formats_diff_retry_events() {
        let requested = serde_json::json!({
            "type": "diff_retry_requested",
            "reason": "invalid unified diff"
        });
        let started = serde_json::json!({
            "type": "diff_retry_started",
            "attempt": 1
        });
        let succeeded = serde_json::json!({
            "type": "diff_retry_succeeded",
            "attempt": 1
        });
        let failed = serde_json::json!({
            "type": "diff_retry_failed",
            "attempt": 1,
            "reason": "still invalid"
        });

        assert!(format_event(&requested).contains("invalid unified diff"));
        assert_eq!(format_event(&started), "- diff_retry_started attempt=1");
        assert_eq!(format_event(&succeeded), "- diff_retry_succeeded attempt=1");
        assert!(format_event(&failed).contains("still invalid"));
    }

    #[test]
    fn redacts_sensitive_text() {
        assert_eq!(
            redact_sensitive_text("Authorization: token"),
            "[redacted]: token"
        );
        assert_eq!(redact_sensitive_text("Bearer secret"), "[redacted]");
    }

    /// resume_candidates テスト用の session transcript を、実運用と同じ
    /// `YYYY/MM/YYYYMMDD-HHMMSS-suffix.jsonl` 配置で書く(path 順 = 時系列を保つ)。
    fn write_candidate_session(root: &PathBuf, index: u32, extra_events: &[&str]) -> String {
        let session_id = format!("20260505-01{index:02}00-aaaa{index:02}");
        let mut lines = vec![
            format!(
                r#"{{"type":"session_started","session_id":"{session_id}","created_at":"2026-05-05T01:{index:02}:00Z","cwd":"/repo"}}"#
            ),
            r#"{"type":"command_started","command":"chat"}"#.to_string(),
            r#"{"type":"model_resolved","model":"qwen"}"#.to_string(),
            format!(r#"{{"type":"user_message","text":"prompt {index}"}}"#),
        ];
        lines.extend(extra_events.iter().map(|line| (*line).to_string()));
        lines.push(r#"{"type":"session_finished","status":"success"}"#.to_string());
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        write_jsonl(root, &format!("2026/05/{session_id}.jsonl"), &line_refs);
        session_id
    }

    #[test]
    fn resume_candidates_lists_recent_sessions_newest_first() {
        let root = temp_dir("candidates-recent");
        let first = write_candidate_session(&root, 1, &[]);
        let second = write_candidate_session(&root, 2, &[]);
        let third = write_candidate_session(&root, 3, &[]);

        let candidates = resume_candidates_in(&root, 7).unwrap();

        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].session_id, third);
        assert_eq!(candidates[1].session_id, second);
        assert_eq!(candidates[2].session_id, first);
        assert_eq!(candidates[0].kind, "chat");
        assert_eq!(candidates[0].model, "qwen");
        assert_eq!(candidates[0].created_at, "2026-05-05T01:03:00Z");
        assert!(candidates[0]
            .source_path
            .ends_with(format!("{third}.jsonl")));
    }

    #[test]
    fn resume_candidates_limits_to_requested_count() {
        let root = temp_dir("candidates-limit");
        for index in 1..=9 {
            write_candidate_session(&root, index, &[]);
        }

        let candidates = resume_candidates_in(&root, 7).unwrap();

        assert_eq!(candidates.len(), 7);
        // 直近7件(9,8,...,3)だけが対象で、古い側(1,2)は含まれない。
        assert_eq!(candidates[0].session_id, "20260505-010900-aaaa09");
        assert_eq!(candidates[6].session_id, "20260505-010300-aaaa03");
    }

    #[test]
    fn resume_candidates_derives_outcomes() {
        let root = temp_dir("candidates-outcomes");
        write_candidate_session(
            &root,
            1,
            &[
                r#"{"type":"diff_proposed","diff":"--- a/README.md"}"#,
                r#"{"type":"approval_decision","decision":"approved"}"#,
                r#"{"type":"patch_applied","files":["README.md"]}"#,
            ],
        );
        write_candidate_session(
            &root,
            2,
            &[
                r#"{"type":"diff_proposed","diff":"--- a/README.md"}"#,
                r#"{"type":"approval_decision","decision":"rejected"}"#,
                r#"{"type":"patch_rejected"}"#,
            ],
        );
        write_candidate_session(&root, 3, &[r#"{"type":"command_failed","error":"boom"}"#]);
        write_candidate_session(&root, 4, &[r#"{"type":"diff_proposed","diff":"--- a/x"}"#]);
        write_candidate_session(&root, 5, &[]);

        let candidates = resume_candidates_in(&root, 7).unwrap();

        // 新しい順: 5=chat-only, 4=diff-only, 3=error, 2=rejected, 1=applied。
        assert_eq!(candidates[0].outcome, ResumeOutcome::ReadOnly);
        assert_eq!(candidates[1].outcome, ResumeOutcome::Cancelled);
        assert_eq!(candidates[2].outcome, ResumeOutcome::Error);
        assert_eq!(candidates[3].outcome, ResumeOutcome::Rejected);
        assert_eq!(candidates[4].outcome, ResumeOutcome::Applied);
        assert_eq!(candidates[4].files, vec!["README.md".to_string()]);
    }

    #[test]
    fn resume_candidates_redacts_intent_and_truncates() {
        let root = temp_dir("candidates-redact");
        let long_tail = "a".repeat(200);
        write_jsonl(
            &root,
            "2026/05/20260505-010100-aaaa01.jsonl",
            &[
                r#"{"type":"session_started","session_id":"20260505-010100-aaaa01","created_at":"2026-05-05T01:01:00Z","cwd":"/repo"}"#,
                r#"{"type":"command_started","command":"chat"}"#,
                &format!(
                    r#"{{"type":"user_message","text":"Authorization: Bearer topsecret\nline two {long_tail}"}}"#
                ),
                r#"{"type":"session_finished","status":"success"}"#,
            ],
        );

        let candidates = resume_candidates_in(&root, 7).unwrap();

        assert_eq!(candidates.len(), 1);
        let intent = &candidates[0].intent;
        assert!(!intent.contains("Bearer"));
        assert!(!intent.contains("topsecret"));
        assert!(intent.chars().count() <= 80);
        assert!(!intent.contains('\n'));
    }

    #[test]
    fn resume_candidates_marks_unreadable_session() {
        let root = temp_dir("candidates-unreadable");
        write_candidate_session(&root, 1, &[]);
        write_jsonl(
            &root,
            "2026/05/20260505-010200-broken.jsonl",
            &["{ this is not json"],
        );
        write_candidate_session(&root, 3, &[]);

        let candidates = resume_candidates_in(&root, 7).unwrap();

        assert_eq!(candidates.len(), 3);
        // 壊れたファイルはその位置(2番目に新しい)に placeholder として残る。
        assert_eq!(candidates[1].session_id, "20260505-010200-broken");
        assert_eq!(candidates[1].intent, "(unreadable session)");
        assert_eq!(candidates[1].outcome, ResumeOutcome::Unknown);
        assert_eq!(candidates[0].session_id, "20260505-010300-aaaa03");
        assert_eq!(candidates[2].session_id, "20260505-010100-aaaa01");
    }

    #[test]
    fn resume_candidates_missing_root_returns_empty() {
        let root = temp_dir("candidates-missing-root");

        let candidates = resume_candidates_in(&root, 7).unwrap();

        assert!(candidates.is_empty());
    }

    fn first_jsonl(root: PathBuf) -> PathBuf {
        let year = fs::read_dir(root).unwrap().next().unwrap().unwrap().path();
        let month = fs::read_dir(year).unwrap().next().unwrap().unwrap().path();
        fs::read_dir(month).unwrap().next().unwrap().unwrap().path()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("origin-session-{name}-{nanos}"))
    }

    fn write_jsonl(root: &PathBuf, relative_path: &str, lines: &[&str]) -> PathBuf {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }
}
