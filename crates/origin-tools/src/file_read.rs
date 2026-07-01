use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;

use crate::workspace::{Workspace, WorkspaceError};

pub const MAX_FILE_BYTES: u64 = 200 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadFile {
    pub relative_path: String,
    pub contents: String,
}

#[derive(Debug)]
pub enum ReadFileError {
    Workspace(WorkspaceError),
    Directory(PathBuf),
    TooLarge {
        path: PathBuf,
        size: u64,
        max_size: u64,
    },
    Binary(PathBuf),
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub fn read_workspace_files(
    workspace: &Workspace,
    paths: &[String],
) -> Result<Vec<ReadFile>, ReadFileError> {
    paths
        .iter()
        .map(|path| read_workspace_file(workspace, path))
        .collect()
}

pub fn read_workspace_file(workspace: &Workspace, input: &str) -> Result<ReadFile, ReadFileError> {
    let path = workspace
        .resolve_existing_file(input)
        .map_err(ReadFileError::Workspace)?;
    let metadata = fs::metadata(&path).map_err(|source| ReadFileError::Read {
        path: path.clone(),
        source,
    })?;

    if metadata.is_dir() {
        return Err(ReadFileError::Directory(path));
    }

    if metadata.len() > MAX_FILE_BYTES {
        return Err(ReadFileError::TooLarge {
            path,
            size: metadata.len(),
            max_size: MAX_FILE_BYTES,
        });
    }

    let bytes = fs::read(&path).map_err(|source| ReadFileError::Read {
        path: path.clone(),
        source,
    })?;
    if bytes.contains(&0) {
        return Err(ReadFileError::Binary(path));
    }

    let contents = String::from_utf8(bytes).map_err(|_| ReadFileError::Binary(path.clone()))?;
    let relative_path = workspace.relative_path(&path).to_string_lossy().to_string();

    Ok(ReadFile {
        relative_path,
        contents,
    })
}

impl fmt::Display for ReadFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => write!(f, "{error}"),
            Self::Directory(path) => {
                write!(f, "Refusing to read directory `{}`.", path.display())
            }
            Self::TooLarge {
                path,
                size,
                max_size,
            } => write!(
                f,
                "Refusing to read `{}` because it is too large ({size} bytes, max {max_size} bytes).",
                path.display()
            ),
            Self::Binary(path) => {
                write!(f, "Refusing to read binary file `{}`.", path.display())
            }
            Self::Read { path, source } => {
                write!(f, "Could not read `{}`: {source}", path.display())
            }
        }
    }
}

impl Error for ReadFileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Workspace(error) => Some(error),
            Self::Read { source, .. } => Some(source),
            Self::Directory(_) | Self::TooLarge { .. } | Self::Binary(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::workspace::Workspace;

    use super::{read_workspace_file, ReadFileError, MAX_FILE_BYTES};

    #[test]
    fn reads_text_file() {
        let root = temp_dir("read-text");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "hello\n").unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let file = read_workspace_file(&workspace, "README.md").unwrap();

        assert_eq!(file.relative_path, "README.md");
        assert_eq!(file.contents, "hello\n");
    }

    #[test]
    fn rejects_directory() {
        let root = temp_dir("read-dir");
        fs::create_dir_all(root.join("src")).unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let err = read_workspace_file(&workspace, "src").unwrap_err();

        assert!(matches!(err, ReadFileError::Directory(_)));
    }

    #[test]
    fn rejects_binary_file() {
        let root = temp_dir("read-binary");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("image.bin"), [0, 1, 2, 3]).unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let err = read_workspace_file(&workspace, "image.bin").unwrap_err();

        assert!(matches!(err, ReadFileError::Binary(_)));
    }

    #[test]
    fn rejects_large_file() {
        let root = temp_dir("read-large");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("large.txt"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let workspace = Workspace::new(&root).unwrap();

        let err = read_workspace_file(&workspace, "large.txt").unwrap_err();

        assert!(matches!(err, ReadFileError::TooLarge { .. }));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("origin-tools-{name}-{nanos}"))
    }
}
