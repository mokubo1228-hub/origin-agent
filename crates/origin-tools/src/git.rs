use std::error::Error;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRepo {
    pub root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatus {
    pub is_dirty: bool,
    pub porcelain: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitApplyMode {
    Check,
    Apply,
}

#[derive(Debug)]
pub enum GitError {
    NotRepository {
        cwd: PathBuf,
        stderr: String,
    },
    Io {
        command: &'static str,
        source: std::io::Error,
    },
    CommandFailed {
        command: &'static str,
        status: Option<i32>,
        stderr: String,
    },
}

pub fn discover_git_repo() -> Result<GitRepo, GitError> {
    let cwd = std::env::current_dir().map_err(|source| GitError::Io {
        command: "current_dir",
        source,
    })?;
    discover_git_repo_from(&cwd)
}

pub fn discover_git_repo_from(cwd: &Path) -> Result<GitRepo, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|source| GitError::Io {
            command: "git rev-parse --show-toplevel",
            source,
        })?;

    if !output.status.success() {
        return Err(GitError::NotRepository {
            cwd: cwd.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(GitRepo {
        root: PathBuf::from(root),
    })
}

pub fn git_dirty_status(repo: &GitRepo) -> Result<GitStatus, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo.root)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|source| GitError::Io {
            command: "git status --porcelain",
            source,
        })?;

    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: "git status --porcelain",
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let porcelain = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(GitStatus {
        is_dirty: !porcelain.trim().is_empty(),
        porcelain,
    })
}

pub fn git_apply_check(repo: &GitRepo, diff: &str) -> Result<(), GitError> {
    run_git_apply(repo, diff, GitApplyMode::Check)
}

pub fn git_apply(repo: &GitRepo, diff: &str) -> Result<(), GitError> {
    run_git_apply(repo, diff, GitApplyMode::Apply)
}

fn run_git_apply(repo: &GitRepo, diff: &str, mode: GitApplyMode) -> Result<(), GitError> {
    let diff = normalize_patch_stdin(diff);
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&repo.root)
        .args(git_apply_args(mode))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| GitError::Io {
            command: "git apply",
            source,
        })?;

    let mut stdin = child.stdin.take().ok_or_else(|| GitError::CommandFailed {
        command: "git apply",
        status: None,
        stderr: "could not open git apply stdin".to_string(),
    })?;
    stdin
        .write_all(diff.as_bytes())
        .map_err(|source| GitError::Io {
            command: "git apply",
            source,
        })?;
    drop(stdin);

    let output = child.wait_with_output().map_err(|source| GitError::Io {
        command: "git apply",
        source,
    })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: match mode {
                GitApplyMode::Check => "git apply --check",
                GitApplyMode::Apply => "git apply",
            },
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn normalize_patch_stdin(diff: &str) -> String {
    if diff.ends_with('\n') {
        diff.to_string()
    } else {
        format!("{diff}\n")
    }
}

pub fn git_apply_args(mode: GitApplyMode) -> Vec<&'static str> {
    match mode {
        GitApplyMode::Check => vec!["apply", "--check", "-"],
        GitApplyMode::Apply => vec!["apply", "-"],
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRepository { cwd, stderr } => {
                if stderr.is_empty() {
                    write!(
                        f,
                        "This command must be run inside a Git repository. `{}` is not a Git repository.",
                        cwd.display()
                    )
                } else {
                    write!(
                        f,
                        "This command must be run inside a Git repository. `{}` is not a Git repository: {stderr}",
                        cwd.display()
                    )
                }
            }
            Self::Io { command, source } => write!(f, "Failed to run {command}: {source}"),
            Self::CommandFailed {
                command,
                status,
                stderr,
            } => write!(
                f,
                "{command} failed{}{}",
                status
                    .map(|status| format!(" with status {status}"))
                    .unwrap_or_default(),
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            ),
        }
    }
}

impl Error for GitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::NotRepository { .. } | Self::CommandFailed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{git_apply_args, GitApplyMode};

    #[test]
    fn builds_git_apply_check_args() {
        assert_eq!(
            git_apply_args(GitApplyMode::Check),
            ["apply", "--check", "-"]
        );
    }

    #[test]
    fn builds_git_apply_args() {
        assert_eq!(git_apply_args(GitApplyMode::Apply), ["apply", "-"]);
    }

    #[test]
    fn normalizes_patch_stdin_with_trailing_newline() {
        assert_eq!(super::normalize_patch_stdin("diff"), "diff\n");
        assert_eq!(super::normalize_patch_stdin("diff\n"), "diff\n");
    }
}
