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
pub(crate) struct RenamePathArgs {
    source: String,
    destination: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RenameKind {
    File,
    Directory,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RenamePathOutput {
    action: Action,
    source: String,
    destination: String,
    kind: RenameKind,
}

pub(crate) struct RenamePath {
    root: PathBuf,
    permission: PermissionRequirement,
}

impl RenamePath {
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
            permission: PermissionRequirement::Automatic,
        })
    }

    fn normalize_relative_path(
        path: &str,
        argument: &str,
        allow_current_directory: bool,
    ) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(invalid_path(format!(
                "Cannot rename path: {argument} must not be empty."
            )));
        }
        let mut normalized = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::CurDir => {}
                Component::ParentDir => return Err(outside_current_directory(path, argument)),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(invalid_path(format!(
                        "Cannot rename path: {argument} must be relative to the current directory."
                    )));
                }
            }
        }
        if !allow_current_directory && normalized.as_os_str().is_empty() {
            return Err(current_directory_protected(path));
        }
        Ok(normalized)
    }

    async fn inspect_target(
        &self,
        relative: &Path,
        original: &str,
        argument: &str,
    ) -> Result<Option<(PathBuf, RenameKind)>, ToolExecutionError> {
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
                Err(error) => return Err(rename_error(original, argument, error)),
            };
            self.validate_component(
                &target,
                &metadata,
                index,
                components.len(),
                original,
                argument,
            )
            .await?;
            if index + 1 == components.len() {
                return Ok(Some((target, target_kind(&metadata, original, argument)?)));
            }
        }
        Ok(Some((self.root.clone(), RenameKind::Directory)))
    }

    async fn validate_component(
        &self,
        path: &Path,
        metadata: &std::fs::Metadata,
        index: usize,
        count: usize,
        original: &str,
        argument: &str,
    ) -> Result<(), ToolExecutionError> {
        if metadata.file_type().is_symlink() {
            return Err(symlink_error(original, argument));
        }
        let resolved = fs::canonicalize(path)
            .await
            .map_err(|error| rename_error(original, argument, error))?;
        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory(original, argument));
        }
        if index + 1 < count && !metadata.is_dir() {
            return Err(invalid_path_type(
                original,
                argument,
                "a parent path is not a directory",
            ));
        }
        Ok(())
    }

    async fn ensure_destination_parent(
        &self,
        relative: &Path,
        source: &str,
        destination: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let parent = relative.parent().ok_or_else(|| {
            invalid_path(format!(
                "Cannot rename \"{source}\" to \"{destination}\": destination parent is missing."
            ))
        })?;
        let mut current = self.root.clone();
        for component in parent.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current.push(name);
            ensure_directory_component(&self.root, &current, source, destination).await?;
        }
        Ok(self.root.join(relative))
    }

    async fn reject_existing_destination(
        &self,
        relative: &Path,
        original: &str,
    ) -> Result<(), ToolExecutionError> {
        if self
            .inspect_target(relative, original, "destination")
            .await?
            .is_some()
        {
            return Err(path_already_exists(original));
        }
        Ok(())
    }
}

impl ToolPermissionMetadata for RenamePath {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for RenamePath {
    const NAME: &'static str = "rename_path";
    type Args = RenamePathArgs;
    type Output = RenamePathOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Rename one file or directory in the current directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "source": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Existing file or directory path relative to the current directory."
                },
                "destination": {
                    "type": "string",
                    "minLength": 1,
                    "description": "New path relative to the current directory; missing parent directories are created."
                }
            },
            "required": ["source", "destination"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let source = Self::normalize_relative_path(&args.source, "source", false)?;
        let destination = Self::normalize_relative_path(&args.destination, "destination", true)?;
        let Some((source_path, kind)) =
            self.inspect_target(&source, &args.source, "source").await?
        else {
            return Err(path_not_found(&args.source));
        };
        if source == destination {
            return Ok(rename_result(source, destination, kind, Action::Unchanged));
        }
        reject_destination_inside_source(
            &source,
            &destination,
            kind,
            &args.source,
            &args.destination,
        )?;
        self.reject_existing_destination(&destination, &args.destination)
            .await?;
        let destination_path = self
            .ensure_destination_parent(&destination, &args.source, &args.destination)
            .await?;
        self.reject_existing_destination(&destination, &args.destination)
            .await?;
        fs::rename(&source_path, &destination_path)
            .await
            .map_err(|error| rename_error(&args.source, "source", error))?;
        Ok(rename_result(source, destination, kind, Action::Moved))
    }
}

async fn ensure_directory_component(
    root: &Path,
    path: &Path,
    source: &str,
    destination: &str,
) -> Result<(), ToolExecutionError> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => {
            validate_directory_component(root, path, &metadata, source, destination).await
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(path).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path)
                        .await
                        .map_err(|error| rename_error(source, destination, error))?;
                    validate_directory_component(root, path, &metadata, source, destination).await
                }
                Err(error) => Err(rename_error(source, destination, error)),
            }
        }
        Err(error) => Err(rename_error(source, destination, error)),
    }
}

async fn validate_directory_component(
    root: &Path,
    path: &Path,
    metadata: &std::fs::Metadata,
    source: &str,
    destination: &str,
) -> Result<(), ToolExecutionError> {
    if metadata.file_type().is_symlink() {
        return Err(symlink_error(destination, "destination"));
    }
    let resolved = fs::canonicalize(path)
        .await
        .map_err(|error| rename_error(destination, "destination", error))?;
    if !resolved.starts_with(root) {
        return Err(outside_current_directory(destination, "destination"));
    }
    if !metadata.is_dir() {
        return Err(invalid_path_type(
            source,
            "destination",
            "a parent path is not a directory",
        ));
    }
    Ok(())
}

fn reject_destination_inside_source(
    source: &Path,
    destination: &Path,
    kind: RenameKind,
    source_display: &str,
    destination_display: &str,
) -> Result<(), ToolExecutionError> {
    if matches!(kind, RenameKind::Directory) && destination.starts_with(source) {
        return Err(invalid_path(format!(
            "Cannot rename directory \"{source_display}\" to \"{destination_display}\": destination is inside the source."
        )));
    }
    Ok(())
}

fn target_kind(
    metadata: &std::fs::Metadata,
    path: &str,
    argument: &str,
) -> Result<RenameKind, ToolExecutionError> {
    if metadata.is_file() {
        Ok(RenameKind::File)
    } else if metadata.is_dir() {
        Ok(RenameKind::Directory)
    } else {
        Err(invalid_path_type(
            path,
            argument,
            "path is not a regular file or directory",
        ))
    }
}

fn rename_result(
    source: PathBuf,
    destination: PathBuf,
    kind: RenameKind,
    action: Action,
) -> RenamePathOutput {
    RenamePathOutput {
        action,
        source: source.to_string_lossy().into_owned(),
        destination: destination.to_string_lossy().into_owned(),
        kind,
    }
}

fn path_not_found(path: &str) -> ToolExecutionError {
    ToolExecutionError::not_found(format!(
        "Cannot rename \"{path}\": source path does not exist."
    ))
    .with_code(error_codes::PATH_NOT_FOUND)
}

fn path_already_exists(path: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot rename to \"{path}\": destination already exists. Choose a new destination."
    ))
    .with_code(error_codes::PATH_ALREADY_EXISTS)
}

fn outside_current_directory(path: &str, argument: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot rename {argument} \"{path}\": path resolves outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn current_directory_protected(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot rename \"{path}\": the current directory cannot be renamed."
    ))
    .with_code(error_codes::INVALID_ARGUMENT)
}

fn symlink_error(path: &str, argument: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot rename {argument} \"{path}\": symbolic links are not allowed in the path."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn invalid_path(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

fn invalid_path_type(path: &str, argument: &str, reason: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!("Cannot rename {argument} \"{path}\": {reason}."))
        .with_code(error_codes::INVALID_PATH_TYPE)
}

fn rename_error(path: &str, argument: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot rename {argument} \"{path}\": permission denied.")
        }
        _ => format!("Cannot rename {argument} \"{path}\": {error}"),
    };
    let tool_error = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::AlreadyExists => ToolExecutionError::invalid_args(message),
        std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    };
    tool_error
        .with_code(match error.kind() {
            std::io::ErrorKind::PermissionDenied => error_codes::PERMISSION_DENIED,
            std::io::ErrorKind::NotFound => error_codes::PATH_NOT_FOUND,
            std::io::ErrorKind::AlreadyExists => error_codes::PATH_ALREADY_EXISTS,
            std::io::ErrorKind::NotADirectory => error_codes::INVALID_PATH_TYPE,
            _ => error_codes::IO_ERROR,
        })
        .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::Tool;

    use super::*;

    #[test]
    fn output_has_unified_rename_shape() {
        let output = rename_result(
            PathBuf::from("old.txt"),
            PathBuf::from("new.txt"),
            RenameKind::File,
            Action::Moved,
        );

        assert_eq!(
            serde_json::to_value(output).ok(),
            Some(serde_json::json!({
                "action": "moved",
                "source": "old.txt",
                "destination": "new.txt",
                "kind": "file"
            }))
        );
    }

    #[test]
    fn same_normalized_path_is_unchanged() {
        let source = RenamePath::normalize_relative_path("./src/old.rs", "source", false).ok();
        let destination =
            RenamePath::normalize_relative_path("src/old.rs", "destination", true).ok();

        assert_eq!(source, destination);
    }

    #[test]
    fn path_safety_rejects_parent_components_and_current_source() {
        let parent = RenamePath::normalize_relative_path("src/../old.rs", "source", false)
            .expect_err("parent component should fail");
        let root = RenamePath::normalize_relative_path(".", "source", false)
            .expect_err("source root should fail");

        assert_eq!(
            parent.code(),
            Some(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
        );
        assert!(root.is_refusal());
    }

    #[test]
    fn existing_destination_has_one_recovery_action() {
        let error = path_already_exists("new.txt");

        assert_eq!(error.code(), Some(error_codes::PATH_ALREADY_EXISTS));
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("new destination"))
        );
    }

    #[test]
    fn schema_requires_source_and_destination() {
        let tool = RenamePath {
            root: PathBuf::from("."),
            permission: PermissionRequirement::Automatic,
        };
        assert_eq!(
            tool.parameters()["required"],
            serde_json::json!(["source", "destination"])
        );
        assert_eq!(
            tool.parameters()["additionalProperties"],
            serde_json::json!(false)
        );
    }
}
