use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use super::contracts::{Action, error_codes};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateDirectoryArgs {
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CreateDirectoryOutput {
    action: Action,
    path: String,
}

pub(crate) struct CreateDirectory {
    root: PathBuf,
}

impl CreateDirectory {
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
        Ok(Self { root })
    }

    fn normalize_relative_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
        if path.is_empty() {
            return Err(invalid_path(
                "Cannot create directory: path must not be empty. Use \".\" for the current directory.",
            ));
        }
        let mut normalized = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::CurDir => {}
                Component::ParentDir => return Err(outside_current_directory(path)),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(invalid_path(format!(
                        "Cannot create directory \"{path}\": path must be relative to the current directory."
                    )));
                }
            }
        }
        Ok(normalized)
    }

    async fn ensure_chain(
        &self,
        relative: &Path,
        original: &str,
    ) -> Result<bool, ToolExecutionError> {
        let mut current = self.root.clone();
        let mut final_created = false;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current.push(name);
            final_created = ensure_directory_component(&self.root, &current, original).await?;
        }
        Ok(final_created)
    }
}

impl Tool for CreateDirectory {
    const NAME: &'static str = "create_directory";
    type Args = CreateDirectoryArgs;
    type Output = CreateDirectoryOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Create a directory chain in the current directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory path relative to the current directory."
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
        let action = if relative.as_os_str().is_empty() {
            Action::Unchanged
        } else if self.ensure_chain(&relative, &args.path).await? {
            Action::Created
        } else {
            Action::Unchanged
        };
        Ok(CreateDirectoryOutput {
            action,
            path: if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                relative.to_string_lossy().into_owned()
            },
        })
    }
}

async fn ensure_directory_component(
    root: &Path,
    path: &Path,
    original: &str,
) -> Result<bool, ToolExecutionError> {
    match fs::symlink_metadata(path).await {
        Ok(_) => {
            validate_directory_component(root, path, original).await?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(path).await {
                Ok(()) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    validate_directory_component(root, path, original).await?;
                    Ok(false)
                }
                Err(error) => Err(create_error(original, error)),
            }
        }
        Err(error) => Err(create_error(original, error)),
    }
}

async fn validate_directory_component(
    root: &Path,
    path: &Path,
    original: &str,
) -> Result<(), ToolExecutionError> {
    let resolved = fs::canonicalize(path)
        .await
        .map_err(|error| create_error(original, error))?;
    if !resolved.starts_with(root) {
        return Err(outside_current_directory(original));
    }
    let metadata = fs::metadata(&resolved)
        .await
        .map_err(|error| create_error(original, error))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(invalid_path_type(original))
    }
}

fn invalid_path(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

fn invalid_path_type(path: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot create directory \"{path}\": path is occupied by a file."
    ))
    .with_code(error_codes::INVALID_PATH_TYPE)
}

fn outside_current_directory(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot create directory \"{path}\": path resolves outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn create_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot create directory \"{path}\": a parent path does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot create directory \"{path}\": permission denied.")
        }
        _ => format!("Cannot create directory \"{path}\": {error}"),
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
    use rig::tool::{Tool, ToolErrorKind};

    use super::*;

    #[test]
    fn output_has_one_action_and_path() {
        let output = CreateDirectoryOutput {
            action: Action::Unchanged,
            path: "src".to_string(),
        };

        assert_eq!(
            serde_json::to_value(output).ok(),
            Some(serde_json::json!({"action": "unchanged", "path": "src"}))
        );
    }

    #[test]
    fn schema_rejects_extra_fields() {
        let tool = CreateDirectory {
            root: PathBuf::from("."),
        };
        assert_eq!(
            tool.parameters()["additionalProperties"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn parent_path_outside_current_directory_is_refused() {
        let error = CreateDirectory::normalize_relative_path("../outside")
            .expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.code(),
            Some(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
        );
    }

    #[test]
    fn file_occupying_directory_path_has_invalid_type() {
        let error = invalid_path_type("notes.txt");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(error.code(), Some(error_codes::INVALID_PATH_TYPE));
    }
}
