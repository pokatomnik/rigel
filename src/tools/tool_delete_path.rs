use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::shared::tool_permissions::{PermissionRequirement, ToolPermissionMetadata};

use super::contracts::{Action, error_codes};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeletePathArgs {
    path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeleteKind {
    File,
    Directory,
    Unknown,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DeletePathOutput {
    action: Action,
    path: String,
    kind: DeleteKind,
}

pub(crate) struct DeletePath {
    root: PathBuf,
    permission: PermissionRequirement,
}

impl DeletePath {
    pub(crate) async fn new() -> Result<Self, ToolExecutionError> {
        let current_dir = env::current_dir().map_err(|error| {
            ToolExecutionError::other(format!("Cannot determine the current directory: {error}"))
                .with_code(error_codes::IO_ERROR)
                .with_source(error)
        })?;
        let root = fs::canonicalize(&current_dir).await.map_err(|error| {
            ToolExecutionError::other(format!(
                "Cannot access the current directory \"{}\": {error}",
                current_dir.display()
            ))
            .with_code(error_codes::IO_ERROR)
            .with_source(error)
        })?;
        Ok(Self {
            root,
            permission: PermissionRequirement::ConfirmationRequired,
        })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(invalid_path("Cannot delete path: path must not be empty."));
        }
        let mut normalized = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::CurDir => {}
                Component::ParentDir => return Err(outside_current_directory(path)),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(invalid_path(format!(
                        "Cannot delete path \"{path}\": path must be relative to the current directory."
                    )));
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(current_directory_protected(path));
        }
        Ok(normalized)
    }

    async fn inspect_target(
        &self,
        relative: &Path,
        original: &str,
    ) -> Result<Option<(PathBuf, DeleteKind)>, ToolExecutionError> {
        let mut target = self.root.clone();
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                continue;
            };
            target.push(name);
            let metadata = match fs::symlink_metadata(&target).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(None);
                }
                Err(error) => return Err(delete_error(original, error)),
            };
            self.validate_component(&target, &metadata, index, components.len(), original)
                .await?;
            if index + 1 == components.len() {
                return Ok(Some((target, target_kind(&metadata, original)?)));
            }
        }
        Err(current_directory_protected(original))
    }

    async fn validate_component(
        &self,
        path: &Path,
        metadata: &std::fs::Metadata,
        index: usize,
        count: usize,
        original: &str,
    ) -> Result<(), ToolExecutionError> {
        if metadata.file_type().is_symlink() {
            return Err(symlink_error(original));
        }
        let resolved = fs::canonicalize(path)
            .await
            .map_err(|error| delete_error(original, error))?;
        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory(original));
        }
        if index + 1 < count && !metadata.is_dir() {
            return Err(invalid_path_type(
                original,
                "a parent path is not a directory",
            ));
        }
        Ok(())
    }
}

impl ToolPermissionMetadata for DeletePath {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for DeletePath {
    const NAME: &'static str = "delete_path";
    type Args = DeletePathArgs;
    type Output = DeletePathOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Delete a file or directory recursively in the current directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File or directory path relative to the current directory."
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
        let relative = Self::normalize_relative_path(&args.path)?;
        let Some((_, kind)) = self.inspect_target(&relative, &args.path).await? else {
            return Ok(delete_result(
                relative,
                DeleteKind::Unknown,
                Action::NotFound,
            ));
        };
        let previous_kind = kind;
        let Some((target, kind)) = self.inspect_target(&relative, &args.path).await? else {
            return Ok(delete_result(relative, previous_kind, Action::NotFound));
        };
        remove_target(&target, kind, &args.path).await?;
        Ok(delete_result(relative, kind, Action::Deleted))
    }
}

async fn remove_target(
    target: &Path,
    kind: DeleteKind,
    original: &str,
) -> Result<(), ToolExecutionError> {
    let result = match kind {
        DeleteKind::File => fs::remove_file(target).await,
        DeleteKind::Directory => fs::remove_dir_all(target).await,
        DeleteKind::Unknown => return Err(invalid_path("Cannot delete an unknown path type.")),
    };
    result.map_err(|error| delete_error(original, error))
}

fn target_kind(metadata: &std::fs::Metadata, path: &str) -> Result<DeleteKind, ToolExecutionError> {
    if metadata.is_file() {
        Ok(DeleteKind::File)
    } else if metadata.is_dir() {
        Ok(DeleteKind::Directory)
    } else {
        Err(invalid_path_type(
            path,
            "path is not a regular file or directory",
        ))
    }
}

fn delete_result(path: PathBuf, kind: DeleteKind, action: Action) -> DeletePathOutput {
    DeletePathOutput {
        action,
        path: path.to_string_lossy().into_owned(),
        kind,
    }
}

fn outside_current_directory(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete path \"{path}\": path resolves outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn current_directory_protected(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete path \"{path}\": deleting the current directory is not allowed."
    ))
    .with_code(error_codes::INVALID_ARGUMENT)
}

fn symlink_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot delete path \"{path}\": symbolic links are not allowed in the path."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn invalid_path(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

fn invalid_path_type(path: &str, reason: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!("Cannot delete path \"{path}\": {reason}."))
        .with_code(error_codes::INVALID_PATH_TYPE)
}

fn delete_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot delete path \"{path}\": path does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot delete path \"{path}\": permission denied.")
        }
        _ => format!("Cannot delete path \"{path}\": {error}"),
    };
    let tool_error = match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        _ => ToolExecutionError::other(message),
    };
    tool_error
        .with_code(match error.kind() {
            std::io::ErrorKind::NotFound => error_codes::PATH_NOT_FOUND,
            std::io::ErrorKind::PermissionDenied => error_codes::PERMISSION_DENIED,
            _ => error_codes::IO_ERROR,
        })
        .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::Tool;

    use super::*;

    #[test]
    fn output_has_action_path_and_kind() {
        let output = delete_result(
            PathBuf::from("notes.txt"),
            DeleteKind::File,
            Action::Deleted,
        );

        assert_eq!(
            serde_json::to_value(output).ok(),
            Some(serde_json::json!({
                "action": "deleted",
                "path": "notes.txt",
                "kind": "file"
            }))
        );
    }

    #[test]
    fn missing_path_is_a_successful_not_found_result() {
        let output = delete_result(PathBuf::from("missing"), DeleteKind::File, Action::NotFound);

        assert_eq!(output.action, Action::NotFound);
    }

    #[test]
    fn path_safety_rejects_current_directory_and_parent_components() {
        let root = DeletePath::normalize_relative_path(".").expect_err("root should be protected");
        let parent = DeletePath::normalize_relative_path("src/../file")
            .expect_err("parent component should fail");

        assert!(root.is_refusal());
        assert_eq!(root.code(), Some(error_codes::INVALID_ARGUMENT));
        assert_eq!(
            parent.code(),
            Some(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
        );
    }

    #[test]
    fn schema_is_flat_and_rejects_unknown_arguments() {
        let tool = DeletePath {
            root: PathBuf::from("."),
            permission: PermissionRequirement::ConfirmationRequired,
        };
        assert_eq!(
            tool.parameters()["additionalProperties"],
            serde_json::json!(false)
        );
        assert_eq!(tool.parameters()["required"], serde_json::json!(["path"]));
    }
}
