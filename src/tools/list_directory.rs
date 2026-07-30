use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Deserialize)]
pub(crate) struct ListDirectoryArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DirectoryEntry {
    name: String,
    #[serde(rename = "type")]
    kind: DirectoryEntryKind,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DirectoryEntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

pub(crate) struct ListDirectory {
    root: PathBuf,
}

impl ListDirectory {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the project root: {error}"))
                .with_source(error)
        })?;

        Self::with_root(current_dir).await
    }

    async fn with_root(root: PathBuf) -> Result<Self, ToolExecutionError> {
        let root = fs::canonicalize(&root).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the project root \"{}\": {error}",
                root.display()
            ))
            .with_source(error)
        })?;

        Ok(Self { root })
    }

    fn validate_relative_path(
        &self,
        path: &Path,
        original: &str,
    ) -> Result<(), ToolExecutionError> {
        if original.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot list directory: path must not be empty. Use \".\" for the project root.",
            ));
        }

        let mut depth = 0_usize;

        for component in path.components() {
            match component {
                Component::Normal(_) => depth += 1,
                Component::ParentDir if depth == 0 => {
                    return Err(outside_project_error(original));
                }
                Component::ParentDir => depth -= 1,
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot list directory \"{original}\": path must be relative to the project root."
                    )));
                }
            }
        }

        Ok(())
    }

    async fn resolve_directory(
        &self,
        path: &Path,
        original: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        self.validate_relative_path(path, original)?;

        let resolved = fs::canonicalize(self.root.join(path))
            .await
            .map_err(|error| access_error(original, error))?;

        if !resolved.starts_with(&self.root) {
            return Err(outside_project_error(original));
        }

        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| access_error(original, error))?;

        if !metadata.is_dir() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot list directory \"{original}\": path points to a file, not a directory."
            )));
        }

        Ok(resolved)
    }
}

impl Tool for ListDirectory {
    const NAME: &'static str = "list_directory";
    type Args = ListDirectoryArgs;
    type Output = Vec<DirectoryEntry>;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "List files and directories in a project directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory path relative to the project root. Use \".\" for the project root."
                }
            },
            "required": ["path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let requested_path = Path::new(&args.path);
        let directory = self.resolve_directory(requested_path, &args.path).await?;
        let mut read_dir = fs::read_dir(&directory)
            .await
            .map_err(|error| access_error(&args.path, error))?;
        let mut entries = Vec::new();

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|error| access_error(&args.path, error))?
        {
            let file_type = entry.file_type().await.map_err(|error| {
                ToolExecutionError::other(format!(
                    "Cannot inspect entry \"{}\" in directory \"{}\": {error}",
                    entry.file_name().to_string_lossy(),
                    args.path
                ))
                .with_source(error)
            })?;
            let kind = if file_type.is_dir() {
                DirectoryEntryKind::Directory
            } else if file_type.is_file() {
                DirectoryEntryKind::File
            } else if file_type.is_symlink() {
                DirectoryEntryKind::Symlink
            } else {
                DirectoryEntryKind::Other
            };

            entries.push(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind,
            });
        }

        entries.sort_by(|left, right| left.name.cmp(&right.name));

        Ok(entries)
    }
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot list directory \"{path}\": path resolves outside the project root."
    ))
}

fn access_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot list directory \"{path}\": directory does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot list directory \"{path}\": permission denied.")
        }
        std::io::ErrorKind::NotADirectory => {
            format!("Cannot list directory \"{path}\": path is not a directory.")
        }
        _ => format!("Cannot list directory \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    fn tool() -> ListDirectory {
        ListDirectory {
            root: PathBuf::from("/project"),
        }
    }

    #[test]
    fn missing_directory_error_is_clear_and_model_visible() {
        let error = access_error("missing", std::io::ErrorKind::NotFound.into());

        assert_eq!(error.kind(), ToolErrorKind::NotFound);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot list directory \"missing\": directory does not exist.")
        );
    }

    #[test]
    fn not_a_directory_error_is_clear_and_model_visible() {
        let error = access_error("file.txt", std::io::ErrorKind::NotADirectory.into());

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot list directory \"file.txt\": path is not a directory.")
        );
    }

    #[test]
    fn parent_path_outside_project_is_refused() {
        let error = tool()
            .validate_relative_path(Path::new("../outside"), "../outside")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot list directory \"../outside\": path resolves outside the project root.")
        );
    }

    #[test]
    fn absolute_path_is_rejected() {
        let path = std::path::MAIN_SEPARATOR.to_string();
        let error = tool()
            .validate_relative_path(Path::new(&path), &path)
            .expect_err("absolute path should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("path must be relative"))
        );
    }

    #[test]
    fn normalized_path_inside_project_is_allowed() {
        tool()
            .validate_relative_path(Path::new("src/../tests"), "src/../tests")
            .expect("path should remain inside the project");
    }
}
