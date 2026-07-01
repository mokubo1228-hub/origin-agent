use std::error::Error;
use std::fmt;

use origin_protocol::patch::DiffProposal;

#[derive(Debug)]
pub enum DiffExtractError {
    MissingDiff,
    InvalidDiff(String),
}

pub fn extract_diff_proposal(response: &str) -> Result<DiffProposal, DiffExtractError> {
    if let Some(diff) = extract_fenced_diff(response) {
        return validate_or_empty(diff);
    }

    if is_no_change_marker(response) {
        return Ok(DiffProposal::new(""));
    }

    if let Some(diff) = extract_unfenced_diff(response) {
        return validate_or_empty(diff);
    }

    Err(DiffExtractError::MissingDiff)
}

fn extract_fenced_diff(response: &str) -> Option<String> {
    let mut in_diff = false;
    let mut lines = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim_start();
        if !in_diff && trimmed.starts_with("```diff") {
            in_diff = true;
            continue;
        }
        if in_diff && trimmed.starts_with("```") {
            return Some(lines.join("\n"));
        }
        if in_diff {
            lines.push(line);
        }
    }

    None
}

fn extract_unfenced_diff(response: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut found_start = false;

    for line in response.lines() {
        if !found_start && line.starts_with("--- ") {
            found_start = true;
        }
        if found_start {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn validate_or_empty(diff: String) -> Result<DiffProposal, DiffExtractError> {
    if diff.trim().is_empty() {
        return Ok(DiffProposal::new(""));
    }

    if looks_like_unified_diff(&diff) {
        Ok(DiffProposal::new(diff))
    } else {
        Err(DiffExtractError::InvalidDiff(
            "response included a diff block, but it did not look like unified diff".to_string(),
        ))
    }
}

fn looks_like_unified_diff(diff: &str) -> bool {
    diff.contains("--- ") && diff.contains("+++ ") && diff.contains("@@")
}

fn is_no_change_marker(response: &str) -> bool {
    let normalized = response
        .trim()
        .to_ascii_uppercase()
        .replace([' ', '-'], "_");
    matches!(
        normalized.as_str(),
        "NO_CHANGE" | "NO_CHANGES" | "NO_CHANGE_NEEDED"
    )
}

impl fmt::Display for DiffExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDiff => write!(
                f,
                "Model response did not include a unified diff. Nothing was applied."
            ),
            Self::InvalidDiff(message) => write!(f, "{message}. Nothing was applied."),
        }
    }
}

impl Error for DiffExtractError {}

#[cfg(test)]
mod tests {
    use super::{extract_diff_proposal, DiffExtractError};

    #[test]
    fn extracts_fenced_diff_block() {
        let response = r#"```diff
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new
```"#;

        let proposal = extract_diff_proposal(response).unwrap();

        assert!(proposal.diff.contains("--- a/README.md"));
        assert!(proposal.diff.contains("+new"));
    }

    #[test]
    fn extracts_unfenced_diff() {
        let response = r#"Here is the patch:
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-old
+new"#;

        let proposal = extract_diff_proposal(response).unwrap();

        assert!(proposal.diff.contains("@@"));
    }

    #[test]
    fn accepts_empty_diff_block() {
        let response = "```diff\n```";
        let proposal = extract_diff_proposal(response).unwrap();

        assert!(proposal.is_empty());
    }

    #[test]
    fn accepts_no_change_marker() {
        let proposal = extract_diff_proposal("NO_CHANGE").unwrap();

        assert!(proposal.is_empty());
    }

    #[test]
    fn rejects_missing_diff() {
        let err = extract_diff_proposal("I changed the file.").unwrap_err();

        assert!(matches!(err, DiffExtractError::MissingDiff));
    }

    #[test]
    fn rejects_hunk_without_file_headers() {
        let response = r#"```diff
@@ -1 +1 @@
-old
+new
```"#;

        let err = extract_diff_proposal(response).unwrap_err();

        assert!(matches!(err, DiffExtractError::InvalidDiff(_)));
    }
}
