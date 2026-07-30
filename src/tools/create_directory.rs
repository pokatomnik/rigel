use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Deserialize)]
pub(crate) struct CreateDirectoryArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CreateDirectoryOutput {
    path: String,
    status: CreateDirectoryStatus,
    message: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CreateDirectoryStatus {
    Created,
    AlreadyExists,
}

pub(crate) struct CreateDirectory {
    root: PathBuf,
}

impl CreateDirectory {
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
                "Cannot create directory: path must not be empty. Use \".\" for the project root.",
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
                        "Cannot create directory \"{path}\": path must be relative to the project root."
                    )));
                }
            }
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
                            "Cannot create directory \"{original}\": no accessible parent directory was found."
                        ))
                        .with_source(error));
                    }
                }
                Err(error) => return Err(create_error(original, error)),
            }
        }
    }

    async fn existing_target_result(
        &self,
        target: &Path,
        original: &str,
    ) -> Result<CreateDirectoryOutput, ToolExecutionError> {
        let resolved = match fs::canonicalize(target).await {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return match fs::symlink_metadata(target).await {
                    Ok(_) => Err(path_is_not_directory_error(original)),
                    Err(metadata_error)
                        if metadata_error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        Err(ToolExecutionError::other(format!(
                            "Cannot create directory \"{original}\": path changed during creation; retry the operation."
                        ))
                        .with_retryable(true)
                        .with_source(metadata_error))
                    }
                    Err(metadata_error) => Err(create_error(original, metadata_error)),
                };
            }
            Err(error) => return Err(create_error(original, error)),
        };

        if !resolved.starts_with(&self.root) {
            return Err(outside_project_error(original));
        }

        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| create_error(original, error))?;

        if metadata.is_dir() {
            Ok(directory_result(
                original,
                CreateDirectoryStatus::AlreadyExists,
            ))
        } else {
            Err(path_is_not_directory_error(original))
        }
    }
}

impl Tool for CreateDirectory {
    const NAME: &'static str = "create_directory";
    type Args = CreateDirectoryArgs;
    type Output = CreateDirectoryOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Create a directory inside the project.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory path relative to the project root. Missing parent directories are also created."
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
        let (existing_ancestor, resolved_ancestor) =
            self.closest_existing_ancestor(&target, &args.path).await?;
        let ancestor_metadata = fs::metadata(&resolved_ancestor)
            .await
            .map_err(|error| create_error(&args.path, error))?;

        if !ancestor_metadata.is_dir() {
            return Err(path_is_not_directory_error(&args.path));
        }

        if existing_ancestor == target {
            return Ok(directory_result(
                &args.path,
                CreateDirectoryStatus::AlreadyExists,
            ));
        }

        let parent = target.parent().ok_or_else(|| {
            ToolExecutionError::other(format!(
                "Cannot create directory \"{}\": parent path could not be determined.",
                args.path
            ))
        })?;

        fs::create_dir_all(parent)
            .await
            .map_err(|error| create_error(&args.path, error))?;

        let resolved_parent = fs::canonicalize(parent)
            .await
            .map_err(|error| create_error(&args.path, error))?;

        if !resolved_parent.starts_with(&self.root) {
            return Err(outside_project_error(&args.path));
        }

        let directory_name = target.file_name().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot create directory \"{}\": path does not name a directory.",
                args.path
            ))
        })?;
        let final_target = resolved_parent.join(directory_name);

        match fs::create_dir(&final_target).await {
            Ok(()) => Ok(directory_result(&args.path, CreateDirectoryStatus::Created)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.existing_target_result(&final_target, &args.path).await
            }
            Err(error) => Err(create_error(&args.path, error)),
        }
    }
}

fn directory_result(path: &str, status: CreateDirectoryStatus) -> CreateDirectoryOutput {
    let message = match status {
        CreateDirectoryStatus::Created => format!("Directory \"{path}\" was created."),
        CreateDirectoryStatus::AlreadyExists => {
            format!("Directory \"{path}\" already exists; no changes were made.")
        }
    };

    CreateDirectoryOutput {
        path: path.to_string(),
        status,
        message,
    }
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot create directory \"{path}\": path resolves outside the project root."
    ))
}

fn path_is_not_directory_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot create directory \"{path}\": path already exists and is not a directory."
    ))
}

fn create_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot create directory \"{path}\": a parent path does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot create directory \"{path}\": permission denied.")
        }
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory => {
            format!(
                "Cannot create directory \"{path}\": a path component exists and is not a directory."
            )
        }
        _ => format!("Cannot create directory \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotADirectory => {
            ToolExecutionError::invalid_args(message)
        }
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    #[test]
    fn created_result_is_explicit() {
        let result = directory_result("src/generated", CreateDirectoryStatus::Created);

        assert_eq!(result.status, CreateDirectoryStatus::Created);
        assert_eq!(result.message, "Directory \"src/generated\" was created.");
    }

    #[test]
    fn already_existing_result_is_explicit() {
        let result = directory_result("src", CreateDirectoryStatus::AlreadyExists);

        assert_eq!(result.status, CreateDirectoryStatus::AlreadyExists);
        assert_eq!(
            result.message,
            "Directory \"src\" already exists; no changes were made."
        );
    }

    #[test]
    fn parent_path_outside_project_is_refused() {
        let error = CreateDirectory::normalize_relative_path("../outside")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot create directory \"../outside\": path resolves outside the project root.")
        );
    }

    #[test]
    fn absolute_path_is_rejected() {
        let path = std::path::MAIN_SEPARATOR.to_string();
        let error =
            CreateDirectory::normalize_relative_path(&path).expect_err("absolute path should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("path must be relative"))
        );
    }

    #[test]
    fn permission_error_is_clear_and_model_visible() {
        let error = create_error("protected", std::io::ErrorKind::PermissionDenied.into());

        assert_eq!(error.kind(), ToolErrorKind::PermissionDenied);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot create directory \"protected\": permission denied.")
        );
    }
}
