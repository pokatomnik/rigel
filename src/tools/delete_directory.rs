use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Deserialize)]
pub(crate) struct DeleteDirectoryArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DeleteDirectoryOutput {
    path: String,
    status: DeleteDirectoryStatus,
    message: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeleteDirectoryStatus {
    DeletedEmpty,
    DeletedWithContents,
    NotFound,
}

pub(crate) struct DeleteDirectory {
    root: PathBuf,
}

impl DeleteDirectory {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
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

        Ok(Self { root })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot delete directory: path must not be empty.",
            ));
        }

        let mut normalized = PathBuf::new();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::ParentDir if !normalized.pop() => {
                    return Err(outside_project_error(path));
                }
                Component::ParentDir | Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolExecutionError::invalid_args(format!(
                        "Cannot delete directory \"{path}\": path must be relative to the project root."
                    )));
                }
            }
        }

        if normalized.as_os_str().is_empty() {
            return Err(project_root_error(path));
        }

        Ok(normalized)
    }

    async fn closest_existing_ancestor(
        &self,
        target: &Path,
        original: &str,
    ) -> Result<(PathBuf, PathBuf), ToolExecutionError> {
        let mut ancestor = target.to_path_buf();

        loop {
            match fs::canonicalize(&ancestor).await {
                Ok(resolved) => {
                    if !resolved.starts_with(&self.root) {
                        return Err(outside_project_error(original));
                    }

                    return Ok((ancestor, resolved));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !ancestor.pop() {
                        return Err(ToolExecutionError::other(format!(
                            "Cannot delete directory \"{original}\": no accessible parent directory was found."
                        ))
                        .with_source(error));
                    }
                }
                Err(error) => return Err(delete_error(original, error)),
            }
        }
    }
}

impl Tool for DeleteDirectory {
    const NAME: &'static str = "delete_directory";
    type Args = DeleteDirectoryArgs;
    type Output = DeleteDirectoryOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Delete a project directory and all of its contents.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory path relative to the project root. The project root itself cannot be deleted."
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
        let target = self.root.join(relative_path);
        let (existing_ancestor, resolved_target) =
            self.closest_existing_ancestor(&target, &args.path).await?;

        if existing_ancestor != target {
            return Ok(directory_result(
                &args.path,
                DeleteDirectoryStatus::NotFound,
            ));
        }

        if resolved_target == self.root {
            return Err(project_root_error(&args.path));
        }

        let metadata = match fs::symlink_metadata(&target).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(directory_result(
                    &args.path,
                    DeleteDirectoryStatus::NotFound,
                ));
            }
            Err(error) => return Err(delete_error(&args.path, error)),
        };

        if metadata.file_type().is_symlink() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot delete directory \"{}\": path is a symbolic link, not a directory.",
                args.path
            )));
        }

        if !metadata.is_dir() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot delete directory \"{}\": path is a file, not a directory.",
                args.path
            )));
        }

        let mut entries = match fs::read_dir(&resolved_target).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(directory_result(
                    &args.path,
                    DeleteDirectoryStatus::NotFound,
                ));
            }
            Err(error) => return Err(delete_error(&args.path, error)),
        };
        let has_contents = match entries.next_entry().await {
            Ok(entry) => entry.is_some(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(directory_result(
                    &args.path,
                    DeleteDirectoryStatus::NotFound,
                ));
            }
            Err(error) => return Err(delete_error(&args.path, error)),
        };

        let deletion_result = if has_contents {
            fs::remove_dir_all(&resolved_target).await
        } else {
            fs::remove_dir(&resolved_target).await
        };

        match deletion_result {
            Ok(()) if has_contents => Ok(directory_result(
                &args.path,
                DeleteDirectoryStatus::DeletedWithContents,
            )),
            Ok(()) => Ok(directory_result(
                &args.path,
                DeleteDirectoryStatus::DeletedEmpty,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(directory_result(
                &args.path,
                DeleteDirectoryStatus::NotFound,
            )),
            Err(error) => Err(delete_error(&args.path, error)),
        }
    }
}

fn directory_result(path: &str, status: DeleteDirectoryStatus) -> DeleteDirectoryOutput {
    let message = match status {
        DeleteDirectoryStatus::DeletedEmpty => {
            format!("Empty directory \"{path}\" was deleted.")
        }
        DeleteDirectoryStatus::DeletedWithContents => {
            format!("Directory \"{path}\" and all of its contents were deleted.")
        }
        DeleteDirectoryStatus::NotFound => {
            format!("Directory \"{path}\" does not exist; nothing remains to delete.")
        }
    };

    DeleteDirectoryOutput {
        path: path.to_string(),
        status,
        message,
    }
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete directory \"{path}\": path resolves outside the project root."
    ))
}

fn project_root_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete directory \"{path}\": deleting the project root is not allowed."
    ))
}

fn delete_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot delete directory \"{path}\": permission denied.")
        }
        std::io::ErrorKind::DirectoryNotEmpty => {
            format!(
                "Cannot delete directory \"{path}\": directory contents changed during deletion; retry the operation."
            )
        }
        std::io::ErrorKind::NotADirectory => {
            format!("Cannot delete directory \"{path}\": path is not a directory.")
        }
        _ => format!("Cannot delete directory \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::DirectoryNotEmpty => {
            ToolExecutionError::other(message).with_retryable(true)
        }
        std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    #[test]
    fn deleted_empty_result_is_explicit() {
        let result = directory_result("empty", DeleteDirectoryStatus::DeletedEmpty);

        assert_eq!(result.status, DeleteDirectoryStatus::DeletedEmpty);
        assert_eq!(result.message, "Empty directory \"empty\" was deleted.");
    }

    #[test]
    fn deleted_with_contents_result_is_explicit() {
        let result = directory_result("generated", DeleteDirectoryStatus::DeletedWithContents);

        assert_eq!(result.status, DeleteDirectoryStatus::DeletedWithContents);
        assert_eq!(
            result.message,
            "Directory \"generated\" and all of its contents were deleted."
        );
    }

    #[test]
    fn missing_directory_result_is_not_an_error() {
        let result = directory_result("missing", DeleteDirectoryStatus::NotFound);

        assert_eq!(result.status, DeleteDirectoryStatus::NotFound);
        assert_eq!(
            result.message,
            "Directory \"missing\" does not exist; nothing remains to delete."
        );
    }

    #[test]
    fn project_root_deletion_is_refused() {
        let error =
            DeleteDirectory::normalize_relative_path(".").expect_err("project root should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot delete directory \".\": deleting the project root is not allowed.")
        );
    }

    #[test]
    fn parent_path_outside_project_is_refused() {
        let error = DeleteDirectory::normalize_relative_path("../outside")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot delete directory \"../outside\": path resolves outside the project root.")
        );
    }

    #[test]
    fn absolute_path_is_rejected() {
        let path = std::path::MAIN_SEPARATOR.to_string();
        let error =
            DeleteDirectory::normalize_relative_path(&path).expect_err("absolute path should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("path must be relative"))
        );
    }

    #[test]
    fn permission_error_is_clear_and_model_visible() {
        let error = delete_error("protected", std::io::ErrorKind::PermissionDenied.into());

        assert_eq!(error.kind(), ToolErrorKind::PermissionDenied);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot delete directory \"protected\": permission denied.")
        );
    }
}
