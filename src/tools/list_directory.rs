use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::tools::contracts::{CollectionEnvelope, error_codes};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListDirectoryArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DirectoryEntry {
    name: String,
    #[serde(rename = "type")]
    kind: DirectoryEntryKind,
}

pub(crate) type ListDirectoryOutput = CollectionEnvelope<DirectoryEntry>;

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
            ToolExecutionError::other(format!("Cannot determine the current directory: {error}"))
                .with_code(error_codes::IO_ERROR)
                .with_source(error)
        })?;

        Self::with_root(current_dir).await
    }

    async fn with_root(root: PathBuf) -> Result<Self, ToolExecutionError> {
        let root = fs::canonicalize(&root).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the current directory \"{}\": {error}",
                root.display()
            ))
            .with_code(error_codes::IO_ERROR)
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
                "Cannot list directory: path must not be empty. Use \".\" for the current directory.",
            )
            .with_code(error_codes::INVALID_ARGUMENT));
        }

        for component in path.components() {
            match component {
                Component::Normal(_) => {}
                Component::ParentDir => {
                    return Err(outside_current_directory_error(original));
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot list directory \"{original}\": path must be relative to the current directory."
                    ))
                    .with_code(error_codes::INVALID_ARGUMENT));
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
            return Err(outside_current_directory_error(original));
        }

        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| access_error(original, error))?;

        if !metadata.is_dir() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot list directory \"{original}\": path points to a file, not a directory."
            ))
            .with_code(error_codes::INVALID_PATH_TYPE));
        }

        Ok(resolved)
    }
}

impl Tool for ListDirectory {
    const NAME: &'static str = "list_directory";
    type Args = ListDirectoryArgs;
    type Output = ListDirectoryOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "List direct entries in a current-directory-relative directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory path relative to the current directory. Use \".\" for the current directory."
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
                .with_code(error_codes::IO_ERROR)
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

        Ok(CollectionEnvelope::new(entries, false))
    }
}

fn outside_current_directory_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot list directory \"{path}\": path resolves outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
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
        std::io::ErrorKind::NotFound => {
            ToolExecutionError::not_found(message).with_code(error_codes::PATH_NOT_FOUND)
        }
        std::io::ErrorKind::PermissionDenied => {
            ToolExecutionError::permission_denied(message).with_code(error_codes::PERMISSION_DENIED)
        }
        std::io::ErrorKind::NotADirectory => {
            ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_PATH_TYPE)
        }
        _ => ToolExecutionError::other(message).with_code(error_codes::IO_ERROR),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::{Tool, ToolErrorKind};

    use super::*;

    fn tool() -> ListDirectory {
        ListDirectory {
            root: PathBuf::from("/project"),
        }
    }

    #[test]
    fn schema_and_output_use_collection_contract() {
        let schema = tool().parameters();
        let output = CollectionEnvelope::new(Vec::<DirectoryEntry>::new(), false);

        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(
            serde_json::to_value(output).ok(),
            Some(serde_json::json!({"items": [], "count": 0, "truncated": false}))
        );
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
    fn parent_path_outside_current_directory_is_refused() {
        let error = tool()
            .validate_relative_path(Path::new("../outside"), "../outside")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot list directory \"../outside\": path resolves outside the current directory."
            )
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
    fn parent_components_are_rejected() {
        let error = tool()
            .validate_relative_path(Path::new("src/../tests"), "src/../tests")
            .expect_err("parent components should be rejected");

        assert!(error.is_refusal());
    }
}
