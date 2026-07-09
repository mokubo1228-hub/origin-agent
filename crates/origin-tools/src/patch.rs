use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::path::{Component, Path};

pub fn changed_files_from_diff(diff: &str) -> Vec<String> {
    let mut files = Vec::new();

    for line in diff.lines() {
        let Some(path) = line.strip_prefix("+++ ") else {
            continue;
        };
        let path = path.trim();
        if path == "/dev/null" {
            continue;
        }
        let path = path.strip_prefix("b/").unwrap_or(path);
        if !files.iter().any(|existing| existing == path) {
            files.push(path.to_string());
        }
    }

    files
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffValidationReport {
    pub requested_files: Vec<String>,
    pub proposed_files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffValidationError {
    pub requested_files: Vec<String>,
    pub proposed_files: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffIntegrityReport {
    pub file_count: usize,
    pub hunk_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffIntegrityError {
    pub reason: String,
}

pub fn validate_diff_targets(
    diff: &str,
    requested_files: &[String],
    policy: &crate::policy::PolicySet,
) -> Result<DiffValidationReport, DiffValidationError> {
    validate_diff_targets_with_mode(diff, requested_files, policy, false)
}

pub fn validate_diff_targets_allowing_new_files(
    diff: &str,
    requested_files: &[String],
    policy: &crate::policy::PolicySet,
) -> Result<DiffValidationReport, DiffValidationError> {
    validate_diff_targets_with_mode(diff, requested_files, policy, true)
}

fn validate_diff_targets_with_mode(
    diff: &str,
    requested_files: &[String],
    policy: &crate::policy::PolicySet,
    allow_new_files: bool,
) -> Result<DiffValidationReport, DiffValidationError> {
    let requested_files = normalize_requested_files(requested_files);
    let requested_set = requested_files.iter().cloned().collect::<BTreeSet<_>>();

    // write policy(P1)。requested_files は全 write 経路の明示ターゲット集合なので、
    // ここで deny を強制すれば手動 /propose fallback を含む全経路が check/apply 前に落ちる。
    // built-in の安全規則より後・実処理より前(P-2/P-3)。deny は user 明示より強い(P-4)。
    for requested in &requested_files {
        if let Some(reason) = policy.write_denial_reason(requested) {
            return Err(validation_error(
                requested_files.clone(),
                BTreeSet::new(),
                reason,
            ));
        }
    }

    let mut proposed = BTreeSet::new();
    let mut pending_new_file: Option<String> = None;

    for line in diff.lines() {
        if line.starts_with("rename from ") || line.starts_with("rename to ") {
            return Err(validation_error(
                requested_files,
                proposed,
                "rename diffs are not supported in this phase",
            ));
        }

        let Some(raw_path) = diff_header_path(line) else {
            continue;
        };

        if raw_path == "/dev/null" {
            if !allow_new_files {
                return Err(validation_error(
                    requested_files,
                    proposed,
                    "new file and delete file diffs are not supported in this phase",
                ));
            }
            if line.starts_with("+++ ") {
                return Err(validation_error(
                    requested_files,
                    proposed,
                    "delete file diffs are not supported in this phase",
                ));
            }
            pending_new_file = Some(raw_path.to_string());
            continue;
        }

        let normalized = match normalize_diff_path(raw_path) {
            Ok(path) => path,
            Err(reason) => {
                proposed.insert(raw_path.to_string());
                return Err(validation_error(requested_files, proposed, reason));
            }
        };

        if pending_new_file.take().is_some() {
            proposed.insert(normalized.clone());
            if !requested_set.contains(&normalized) {
                return Err(validation_error(
                    requested_files,
                    proposed,
                    "model proposed changes outside the requested file scope",
                ));
            }
            continue;
        }

        if allow_new_files {
            proposed.insert(normalized);
            return Err(validation_error(
                requested_files,
                proposed,
                "new-file proposals must use `--- /dev/null` and create only the requested target",
            ));
        }

        proposed.insert(normalized.clone());
        if !requested_set.contains(&normalized) {
            return Err(validation_error(
                requested_files,
                proposed,
                "model proposed changes outside the requested file scope",
            ));
        }
    }

    Ok(DiffValidationReport {
        requested_files,
        proposed_files: proposed.into_iter().collect(),
    })
}

pub fn validate_diff_integrity(diff: &str) -> Result<DiffIntegrityReport, DiffIntegrityError> {
    if diff.trim().is_empty() {
        return Ok(DiffIntegrityReport {
            file_count: 0,
            hunk_count: 0,
        });
    }

    let lines = diff.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut file_count = 0;
    let mut hunk_count = 0;
    let mut state = IntegrityState::Start;
    let mut hunk: Option<HunkIntegrity> = None;
    let mut file_has_hunk = false;

    while index < lines.len() {
        let line = lines[index];

        if let Some(current) = hunk.as_ref() {
            if current.counts_match()
                && (line.starts_with("--- ")
                    || line.starts_with("@@ ")
                    || is_supported_diff_metadata(line))
            {
                finish_hunk(hunk.take().unwrap())?;
                if line.starts_with("--- ") {
                    state = IntegrityState::NeedNewHeader;
                    file_count += 1;
                    file_has_hunk = false;
                    index += 1;
                    continue;
                }
                if line.starts_with("@@ ") {
                    let (old_expected, new_expected) = parse_hunk_header(line)?;
                    hunk = Some(HunkIntegrity {
                        old_expected,
                        new_expected,
                        old_seen: 0,
                        new_seen: 0,
                        body_lines: 0,
                    });
                    hunk_count += 1;
                    file_has_hunk = true;
                    index += 1;
                    continue;
                }
                state = IntegrityState::Start;
            }
        }

        if let Some(current) = hunk.as_mut() {
            count_hunk_line(line, current)?;
            index += 1;
            continue;
        }

        match state {
            IntegrityState::Start => {
                if line.trim().is_empty() || is_supported_diff_metadata(line) {
                    index += 1;
                    continue;
                }
                if line.starts_with("--- ") {
                    state = IntegrityState::NeedNewHeader;
                    file_count += 1;
                    file_has_hunk = false;
                } else if line.starts_with("+++ ") {
                    return Err(integrity_error(
                        "found `+++` file header before `---` header",
                    ));
                } else if line.starts_with("@@ ") {
                    return Err(integrity_error("found hunk header before file headers"));
                } else {
                    return Err(integrity_error("diff does not start with a file header"));
                }
            }
            IntegrityState::NeedNewHeader => {
                if line.starts_with("+++ ") {
                    state = IntegrityState::NeedHunk;
                } else if line.starts_with("--- ") {
                    return Err(integrity_error(
                        "found `---` file header without matching `+++` header",
                    ));
                } else {
                    return Err(integrity_error(
                        "missing `+++` file header after `---` header",
                    ));
                }
            }
            IntegrityState::NeedHunk => {
                if line.starts_with("@@ ") {
                    let (old_expected, new_expected) = parse_hunk_header(line)?;
                    hunk = Some(HunkIntegrity {
                        old_expected,
                        new_expected,
                        old_seen: 0,
                        new_seen: 0,
                        body_lines: 0,
                    });
                    hunk_count += 1;
                    file_has_hunk = true;
                } else if line.starts_with("--- ") {
                    return Err(integrity_error("file header did not include a hunk"));
                } else if line.starts_with("+++ ") {
                    return Err(integrity_error("found duplicate `+++` file header"));
                } else if is_supported_diff_metadata(line) {
                    // Allow common git diff metadata between file headers and hunks.
                } else {
                    return Err(integrity_error("expected a hunk header after file headers"));
                }
            }
        }

        index += 1;
    }

    if let Some(current) = hunk.take() {
        finish_hunk(current)?;
    } else if matches!(state, IntegrityState::NeedNewHeader) {
        return Err(integrity_error(
            "missing `+++` file header after `---` header",
        ));
    } else if matches!(state, IntegrityState::NeedHunk) && !file_has_hunk {
        return Err(integrity_error("file header did not include a hunk"));
    }

    if file_count == 0 {
        return Err(integrity_error("diff did not include file headers"));
    }

    Ok(DiffIntegrityReport {
        file_count,
        hunk_count,
    })
}

fn normalize_requested_files(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter_map(|file| normalize_diff_path(file).ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Clone, Copy)]
enum IntegrityState {
    Start,
    NeedNewHeader,
    NeedHunk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HunkIntegrity {
    old_expected: usize,
    new_expected: usize,
    old_seen: usize,
    new_seen: usize,
    body_lines: usize,
}

impl HunkIntegrity {
    fn counts_match(&self) -> bool {
        self.old_seen == self.old_expected && self.new_seen == self.new_expected
    }
}

fn parse_hunk_header(line: &str) -> Result<(usize, usize), DiffIntegrityError> {
    if !line.starts_with("@@ ") {
        return Err(integrity_error("invalid hunk header"));
    }
    let rest = &line[3..];
    let Some(end) = rest.find(" @@") else {
        return Err(integrity_error("invalid hunk header"));
    };
    let header = &rest[..end];
    let mut parts = header.split_whitespace();
    let old_range = parts
        .next()
        .ok_or_else(|| integrity_error("hunk header is missing old range"))?;
    let new_range = parts
        .next()
        .ok_or_else(|| integrity_error("hunk header is missing new range"))?;

    if parts.next().is_some() {
        return Err(integrity_error(
            "hunk header included unexpected range data",
        ));
    }

    Ok((
        parse_hunk_range(old_range, '-')?,
        parse_hunk_range(new_range, '+')?,
    ))
}

fn parse_hunk_range(range: &str, prefix: char) -> Result<usize, DiffIntegrityError> {
    let Some(range) = range.strip_prefix(prefix) else {
        return Err(integrity_error("hunk header range had the wrong sign"));
    };
    let mut parts = range.split(',');
    let start = parts
        .next()
        .ok_or_else(|| integrity_error("hunk header range is missing a start line"))?;
    if start.is_empty() || !start.chars().all(|character| character.is_ascii_digit()) {
        return Err(integrity_error("hunk header range start is invalid"));
    }
    let count = match parts.next() {
        Some(count) => {
            if count.is_empty() || !count.chars().all(|character| character.is_ascii_digit()) {
                return Err(integrity_error("hunk header range count is invalid"));
            }
            count
                .parse::<usize>()
                .map_err(|_| integrity_error("hunk header range count is invalid"))?
        }
        None => 1,
    };
    if parts.next().is_some() {
        return Err(integrity_error("hunk header range has too many parts"));
    }
    Ok(count)
}

fn count_hunk_line(line: &str, hunk: &mut HunkIntegrity) -> Result<(), DiffIntegrityError> {
    if line.starts_with('\\') {
        return Ok(());
    }
    let Some(marker) = line.chars().next() else {
        return Err(integrity_error(
            "hunk body included an empty line without a diff marker",
        ));
    };

    match marker {
        ' ' => {
            hunk.old_seen += 1;
            hunk.new_seen += 1;
            hunk.body_lines += 1;
        }
        '-' => {
            hunk.old_seen += 1;
            hunk.body_lines += 1;
        }
        '+' => {
            hunk.new_seen += 1;
            hunk.body_lines += 1;
        }
        _ => {
            return Err(integrity_error(
                "hunk body line did not start with ` `, `-`, `+`, or `\\`",
            ))
        }
    }

    if hunk.old_seen > hunk.old_expected || hunk.new_seen > hunk.new_expected {
        return Err(integrity_error(
            "hunk header line count does not match hunk body",
        ));
    }

    Ok(())
}

fn finish_hunk(hunk: HunkIntegrity) -> Result<(), DiffIntegrityError> {
    if hunk.body_lines == 0 {
        return Err(integrity_error("hunk body is empty"));
    }
    if !hunk.counts_match() {
        return Err(integrity_error(
            "hunk header line count does not match hunk body",
        ));
    }
    Ok(())
}

fn is_supported_diff_metadata(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("old mode ")
        || line.starts_with("new mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("new file mode ")
}

fn integrity_error(reason: impl Into<String>) -> DiffIntegrityError {
    DiffIntegrityError {
        reason: reason.into(),
    }
}

fn diff_header_path(line: &str) -> Option<&str> {
    line.strip_prefix("--- ")
        .or_else(|| line.strip_prefix("+++ "))
        .and_then(|path| path.split_whitespace().next())
}

fn normalize_diff_path(path: &str) -> Result<String, &'static str> {
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);

    if path.trim().is_empty() {
        return Err("diff header included an empty path");
    }
    let path_value = Path::new(path);
    if path_value.is_absolute() {
        return Err("absolute diff paths are not allowed");
    }
    if path_value
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("diff paths must not contain `..`");
    }

    Ok(path.to_string())
}

fn validation_error(
    requested_files: Vec<String>,
    proposed_files: BTreeSet<String>,
    reason: impl Into<String>,
) -> DiffValidationError {
    DiffValidationError {
        requested_files,
        proposed_files: proposed_files.into_iter().collect(),
        reason: reason.into(),
    }
}

impl fmt::Display for DiffValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Model proposed changes outside the requested file scope."
        )?;
        writeln!(f, "Reason: {}", self.reason)?;
        writeln!(f, "Requested files:")?;
        for file in &self.requested_files {
            writeln!(f, "- {file}")?;
        }
        writeln!(f, "Proposed files:")?;
        if self.proposed_files.is_empty() {
            writeln!(f, "- unknown")?;
        } else {
            for file in &self.proposed_files {
                writeln!(f, "- {file}")?;
            }
        }
        write!(f, "Nothing was applied.")
    }
}

impl Error for DiffValidationError {}

impl fmt::Display for DiffIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Model returned an invalid unified diff.")?;
        writeln!(f, "Reason: {}", self.reason)?;
        write!(f, "Nothing was applied.")
    }
}

impl Error for DiffIntegrityError {}

#[cfg(test)]
mod tests {
    use super::{
        changed_files_from_diff, validate_diff_integrity, validate_diff_targets,
        validate_diff_targets_allowing_new_files,
    };

    #[test]
    fn extracts_changed_files_from_diff() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new
--- a/src/main.rs
+++ b/src/main.rs
@@ -1 +1 @@
-old
+new"#;

        let files = changed_files_from_diff(diff);

        assert_eq!(files, ["README.md", "src/main.rs"]);
    }

    #[test]
    fn ignores_dev_null() {
        let diff = r#"--- a/README.md
+++ /dev/null
@@ -1 +0,0 @@
-old"#;

        let files = changed_files_from_diff(diff);

        assert!(files.is_empty());
    }

    #[test]
    fn validates_requested_file_without_prefixes() {
        let diff = r#"--- README.md
+++ README.md
@@ -1 +1 @@
-old
+new"#;

        let report = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap();

        assert_eq!(report.proposed_files, ["README.md"]);
    }

    #[test]
    fn validates_requested_file_with_a_b_prefixes() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new"#;

        let report = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap();

        assert_eq!(report.requested_files, ["README.md"]);
        assert_eq!(report.proposed_files, ["README.md"]);
    }

    #[test]
    fn rejects_unrequested_file() {
        let diff = r#"--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("outside the requested file scope"));
        assert_eq!(err.requested_files, ["README.md"]);
        assert_eq!(err.proposed_files, ["Cargo.toml"]);
    }

    #[test]
    fn validates_multiple_requested_files() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1 +1 @@
-old
+new"#;

        let report = validate_diff_targets(
            diff,
            &["README.md".to_string(), "Cargo.toml".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap();

        assert_eq!(report.proposed_files, ["Cargo.toml", "README.md"]);
    }

    #[test]
    fn rejects_parent_dir_path() {
        let diff = r#"--- a/../Cargo.toml
+++ b/../Cargo.toml
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("`..`"));
    }

    #[test]
    fn rejects_absolute_path() {
        let diff = r#"--- /tmp/README.md
+++ /tmp/README.md
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("absolute"));
    }

    #[test]
    fn rejects_new_file_diff() {
        let diff = r#"--- /dev/null
+++ b/README.md
@@ -0,0 +1 @@
+new"#;

        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("new file and delete file"));
    }

    #[test]
    fn validates_requested_new_file_when_allowed() {
        let diff = r#"--- /dev/null
+++ b/docs/new.md
@@ -0,0 +1 @@
+new"#;

        let report = validate_diff_targets_allowing_new_files(
            diff,
            &["docs/new.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap();

        assert_eq!(report.requested_files, ["docs/new.md"]);
        assert_eq!(report.proposed_files, ["docs/new.md"]);
    }

    #[test]
    fn rejects_unrequested_new_file_when_allowed() {
        let diff = r#"--- /dev/null
+++ b/docs/other.md
@@ -0,0 +1 @@
+new"#;

        let err = validate_diff_targets_allowing_new_files(
            diff,
            &["docs/new.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("outside the requested file scope"));
        assert_eq!(err.proposed_files, ["docs/other.md"]);
    }

    #[test]
    fn rejects_existing_file_style_headers_when_new_files_are_allowed() {
        let diff = r#"--- a/docs/new.md
+++ b/docs/new.md
@@ -0,0 +1 @@
+new"#;

        let err = validate_diff_targets_allowing_new_files(
            diff,
            &["docs/new.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("must use `--- /dev/null`"));
    }

    #[test]
    fn rejects_delete_file_diff() {
        let diff = r#"--- a/README.md
+++ /dev/null
@@ -1 +0,0 @@
-old"#;

        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("new file and delete file"));
    }

    #[test]
    fn rejects_delete_file_diff_even_when_new_files_are_allowed() {
        let diff = r#"--- a/README.md
+++ /dev/null
@@ -1 +0,0 @@
-old"#;

        let err = validate_diff_targets_allowing_new_files(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("must use `--- /dev/null`"));
    }

    #[test]
    fn rejects_rename_diff() {
        let diff = r#"rename from README.md
rename to README2.md
--- a/README.md
+++ b/README2.md
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("rename"));
    }

    #[test]
    fn rejects_rename_diff_even_when_new_files_are_allowed() {
        let diff = r#"rename from README.md
rename to README2.md
--- a/README.md
+++ b/README2.md
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_targets_allowing_new_files(
            diff,
            &["README.md".to_string()],
            &crate::policy::PolicySet::empty(),
        )
        .unwrap_err();

        assert!(err.reason.contains("rename"));
    }

    #[test]
    fn validates_integrity_for_single_file_diff() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new"#;

        let report = validate_diff_integrity(diff).unwrap();

        assert_eq!(report.file_count, 1);
        assert_eq!(report.hunk_count, 1);
    }

    #[test]
    fn validates_integrity_for_multi_hunk_diff() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1,2 +1,2 @@
 old
-helo
+hello
@@ -5,2 +5,2 @@
 context
-wrold
+world"#;

        let report = validate_diff_integrity(diff).unwrap();

        assert_eq!(report.file_count, 1);
        assert_eq!(report.hunk_count, 2);
    }

    #[test]
    fn validates_integrity_for_multi_file_diff() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1 +1 @@
-old
+new"#;

        let report = validate_diff_integrity(diff).unwrap();

        assert_eq!(report.file_count, 2);
        assert_eq!(report.hunk_count, 2);
    }

    #[test]
    fn validates_integrity_for_empty_diff_block() {
        let report = validate_diff_integrity("").unwrap();

        assert_eq!(report.file_count, 0);
        assert_eq!(report.hunk_count, 0);
    }

    #[test]
    fn rejects_integrity_when_new_header_is_missing() {
        let diff = r#"--- a/README.md
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_integrity(diff).unwrap_err();

        assert!(err.reason.contains("missing `+++`"));
    }

    #[test]
    fn rejects_integrity_when_old_header_is_missing() {
        let diff = r#"+++ b/README.md
@@ -1 +1 @@
-old
+new"#;

        let err = validate_diff_integrity(diff).unwrap_err();

        assert!(err.reason.contains("before `---`"));
    }

    #[test]
    fn rejects_integrity_for_broken_hunk_header() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ bad header @@
-old
+new"#;

        let err = validate_diff_integrity(diff).unwrap_err();

        assert!(err.reason.contains("hunk header"));
    }

    #[test]
    fn rejects_integrity_for_empty_hunk_body() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1,0 +1,0 @@"#;

        let err = validate_diff_integrity(diff).unwrap_err();

        assert!(err.reason.contains("hunk body is empty"));
    }

    #[test]
    fn rejects_integrity_for_hunk_count_mismatch() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1,2 +1,2 @@
-old
+new"#;

        let err = validate_diff_integrity(diff).unwrap_err();

        assert!(err.reason.contains("line count"));
    }

    #[test]
    fn rejects_integrity_for_invalid_hunk_line() {
        let diff = r#"--- a/README.md
+++ b/README.md
@@ -1 +1 @@
!not a diff line"#;

        let err = validate_diff_integrity(diff).unwrap_err();

        assert!(err.reason.contains("hunk body line"));
    }

    fn write_deny_policy(pattern: &str) -> crate::policy::PolicySet {
        crate::policy::PolicySet {
            write_deny: vec![crate::policy::PolicyRule {
                pattern: pattern.to_string(),
                source: crate::policy::PolicySource::Workspace,
            }],
            workspace_rules: 1,
            ..crate::policy::PolicySet::empty()
        }
    }

    #[test]
    fn write_denied_target_fails_diff_validation() {
        let diff = "--- a/docs/archive/old.md\n+++ b/docs/archive/old.md\n@@ -1 +1 @@\n-a\n+b\n";
        let err = validate_diff_targets(
            diff,
            &["docs/archive/old.md".to_string()],
            &write_deny_policy("docs/archive/**"),
        )
        .unwrap_err();
        assert!(err.reason.contains("denied by workspace policy"));
        assert!(err.reason.contains(".origin/policy.toml"));
        assert!(err.reason.contains("docs/archive/old.md"));
        assert!(!err.reason.contains("**"));
    }

    #[test]
    fn write_deny_does_not_grant_anything() {
        // policy があっても、従来 reject される diff(範囲外)は reject のまま(P-1)。
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1 +1 @@\n-a\n+b\n";
        let err = validate_diff_targets(
            diff,
            &["README.md".to_string()],
            &write_deny_policy("docs/archive/**"),
        )
        .unwrap_err();
        assert!(err.reason.contains("outside the requested file scope"));
    }
}
