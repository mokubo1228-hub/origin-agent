use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

#[derive(Debug)]
pub enum WorkspaceError {
    CurrentDir(std::io::Error),
    CanonicalizeRoot {
        path: PathBuf,
        source: std::io::Error,
    },
    CanonicalizePath {
        path: PathBuf,
        source: std::io::Error,
    },
    OutsideWorkspace {
        path: PathBuf,
        root: PathBuf,
    },
}

impl Workspace {
    pub fn from_current_dir() -> Result<Self, WorkspaceError> {
        let cwd = std::env::current_dir().map_err(WorkspaceError::CurrentDir)?;
        Self::new(cwd)
    }

    pub fn new(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let path = root.as_ref().to_path_buf();
        let root = path
            .canonicalize()
            .map_err(|source| WorkspaceError::CanonicalizeRoot { path, source })?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve_existing_file(&self, input: &str) -> Result<PathBuf, WorkspaceError> {
        let raw = PathBuf::from(input);
        let candidate = if raw.is_absolute() {
            raw
        } else {
            self.root.join(raw)
        };
        let path = candidate
            .canonicalize()
            .map_err(|source| WorkspaceError::CanonicalizePath {
                path: candidate,
                source,
            })?;

        if !path.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideWorkspace {
                path,
                root: self.root.clone(),
            });
        }

        Ok(path)
    }

    pub fn relative_path<'a>(&'a self, path: &'a Path) -> &'a Path {
        path.strip_prefix(&self.root).unwrap_or(path)
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDir(source) => {
                write!(f, "Could not resolve current directory: {source}")
            }
            Self::CanonicalizeRoot { path, source } => {
                write!(
                    f,
                    "Could not resolve workspace root `{}`: {source}",
                    path.display()
                )
            }
            Self::CanonicalizePath { path, source } => {
                write!(f, "Could not resolve path `{}`: {source}", path.display())
            }
            Self::OutsideWorkspace { path, root } => {
                write!(
                    f,
                    "Refusing to read `{}` because it is outside workspace `{}`.",
                    path.display(),
                    root.display()
                )
            }
        }
    }
}

impl Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CurrentDir(source)
            | Self::CanonicalizeRoot { source, .. }
            | Self::CanonicalizePath { source, .. } => Some(source),
            Self::OutsideWorkspace { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{Workspace, WorkspaceError};

    #[test]
    fn resolves_file_inside_workspace() {
        let root = temp_dir("inside");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let workspace = Workspace::new(&root).unwrap();
        let path = workspace.resolve_existing_file("src/main.rs").unwrap();

        assert!(path.ends_with("src/main.rs"));
    }

    #[test]
    fn rejects_file_outside_workspace() {
        let root = temp_dir("outside-root");
        let outside = temp_dir("outside-file");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();

        let workspace = Workspace::new(&root).unwrap();
        let err = workspace
            .resolve_existing_file(outside.join("secret.txt").to_str().unwrap())
            .unwrap_err();

        assert!(matches!(err, WorkspaceError::OutsideWorkspace { .. }));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("origin-tools-{name}-{nanos}"))
    }
}
