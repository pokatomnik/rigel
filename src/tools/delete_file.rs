use std::{
    env,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{entities::tool_confirm_result::ToolConfirmResult, shared::terminal_io::TerminalIO};

#[derive(Deserialize)]
pub(crate) struct DeleteFileArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DeleteFileOutput {
    path: String,
    status: DeleteFileStatus,
    message: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeleteFileStatus {
    Deleted,
}

pub(crate) struct DeleteFile {
    root: PathBuf,
    terminal_io: Arc<TerminalIO>,
}

impl DeleteFile {
    pub(crate) async fn new(terminal_io: Arc<TerminalIO>) -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the project root: {error}"))
                .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the project root \"{}\": {error}",
                current_dir.display()
            ))
            .with_source(error)
        })?;

        Ok(Self { root, terminal_io })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot delete file: path must not be empty.",
            ));
        }

        let mut normalized = PathBuf::new();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::ParentDir => {
                    return Err(outside_project_error(path));
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot delete file \"{path}\": path must be relative to the project root."
                    )));
                }
            }
        }

        if normalized.as_os_str().is_empty() {
            return Err(project_root_error(path));
        }

        Ok(normalized)
    }
}

impl Tool for DeleteFile {
    const NAME: &'static str = "delete_file";
    type Args = DeleteFileArgs;
    type Output = DeleteFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Delete one regular file inside the project. A missing file is an error.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File path relative to the project root. The file must exist."
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
        let relative_path = Self::normalize_relative_path(&args.path)?;
        let target = self.root.join(&relative_path);
        let metadata = fs::symlink_metadata(&target)
            .await
            .map_err(|error| delete_error(&args.path, error))?;

        if metadata.file_type().is_symlink() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot delete file \"{}\": path is a symbolic link, not a regular file.",
                args.path
            ))
            .with_code("INVALID_PATH_TYPE"));
        }

        if metadata.is_dir() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot delete file \"{}\": path is a directory. Use delete_directory instead.",
                args.path
            ))
            .with_code("INVALID_PATH_TYPE"));
        }

        if !metadata.is_file() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot delete file \"{}\": path is not a regular file.",
                args.path
            ))
            .with_code("INVALID_PATH_TYPE"));
        }

        let resolved = fs::canonicalize(&target)
            .await
            .map_err(|error| delete_error(&args.path, error))?;

        if !resolved.starts_with(&self.root) {
            return Err(outside_project_error(&args.path));
        }

        let confirmed = self.terminal_io.confirm_toll_call(
            format!("Are you sure about removing a file \"{}\"", args.path).as_str(),
        );

        if let ToolConfirmResult::No = confirmed {
            return Err(user_forbid(&args.path));
        }

        fs::remove_file(&resolved)
            .await
            .map_err(|error| delete_error(&args.path, error))?;

        Ok(file_result(relative_path))
    }
}

fn file_result(path: PathBuf) -> DeleteFileOutput {
    let path = path.to_string_lossy().into_owned();

    DeleteFileOutput {
        message: format!("File \"{path}\" was deleted."),
        path,
        status: DeleteFileStatus::Deleted,
    }
}

fn user_forbid(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete file \"{path}\": the user did not approve the deletion. Ask the user what to do next."
    ))
    .with_code("USER_REFUSED")
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete file \"{path}\": path resolves outside the project root. Use a project-relative path without '..'."
    ))
    .with_code("PATH_OUTSIDE_CURRENT_DIRECTORY")
}

fn project_root_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete file \"{path}\": deleting the project root is not allowed."
    ))
    .with_code("PROJECT_ROOT_PROTECTED")
}

fn delete_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot delete file \"{path}\": file does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!(
                "Cannot delete file \"{path}\": permission denied. Check the file and parent directory permissions."
            )
        }
        std::io::ErrorKind::IsADirectory => {
            format!(
                "Cannot delete file \"{path}\": path is a directory. Use delete_directory instead."
            )
        }
        std::io::ErrorKind::NotADirectory => {
            format!(
                "Cannot delete file \"{path}\": a parent path is not a directory. Check the path and retry."
            )
        }
        _ => format!("Cannot delete file \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => {
            ToolExecutionError::not_found(message).with_code("PATH_NOT_FOUND")
        }
        std::io::ErrorKind::PermissionDenied => {
            ToolExecutionError::permission_denied(message).with_code("PERMISSION_DENIED")
        }
        std::io::ErrorKind::IsADirectory | std::io::ErrorKind::NotADirectory => {
            ToolExecutionError::invalid_args(message).with_code("INVALID_PATH_TYPE")
        }
        _ => ToolExecutionError::other(message).with_code("IO_ERROR"),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    #[test]
    fn deleted_result_is_explicit() {
        let result = file_result(PathBuf::from("notes.txt"));

        assert_eq!(result.status, DeleteFileStatus::Deleted);
        assert_eq!(result.path, "notes.txt");
        assert_eq!(result.message, "File \"notes.txt\" was deleted.");
    }

    #[test]
    fn relative_path_is_normalized() {
        let path = DeleteFile::normalize_relative_path("./notes.txt")
            .expect("relative path should be valid");

        assert_eq!(path, PathBuf::from("notes.txt"));
    }

    #[test]
    fn project_root_deletion_is_refused() {
        let error =
            DeleteFile::normalize_relative_path(".").expect_err("project root should be protected");

        assert!(error.is_refusal());
        assert_eq!(error.code(), Some("PROJECT_ROOT_PROTECTED"));
        assert_eq!(
            error.model_feedback(),
            Some("Cannot delete file \".\": deleting the project root is not allowed.")
        );
    }

    #[test]
    fn parent_path_outside_project_is_refused() {
        let error = DeleteFile::normalize_relative_path("../outside.txt")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(error.code(), Some("PATH_OUTSIDE_CURRENT_DIRECTORY"));
    }

    #[test]
    fn parent_component_inside_project_is_also_refused() {
        let error = DeleteFile::normalize_relative_path("src/../notes.txt")
            .expect_err("parent components should fail");

        assert!(error.is_refusal());
        assert_eq!(error.code(), Some("PATH_OUTSIDE_CURRENT_DIRECTORY"));
    }

    #[test]
    fn absolute_path_is_rejected() {
        let path = format!("{}outside.txt", std::path::MAIN_SEPARATOR);
        let error =
            DeleteFile::normalize_relative_path(&path).expect_err("absolute path should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("path must be relative"))
        );
    }

    #[test]
    fn missing_file_is_a_model_visible_error() {
        let error = delete_error("missing.txt", std::io::ErrorKind::NotFound.into());

        assert_eq!(error.kind(), ToolErrorKind::NotFound);
        assert_eq!(error.code(), Some("PATH_NOT_FOUND"));
        assert_eq!(
            error.model_feedback(),
            Some("Cannot delete file \"missing.txt\": file does not exist.")
        );
    }

    #[test]
    fn permission_error_explains_the_next_step() {
        let error = delete_error("protected.txt", std::io::ErrorKind::PermissionDenied.into());

        assert_eq!(error.kind(), ToolErrorKind::PermissionDenied);
        assert_eq!(error.code(), Some("PERMISSION_DENIED"));
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot delete file \"protected.txt\": permission denied. Check the file and parent directory permissions."
            )
        );
    }
}
