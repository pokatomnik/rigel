use std::{
    env,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

use super::contracts::{Action, error_codes};

const TEMP_FILE_ATTEMPTS: usize = 8;
static NEXT_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFileArgs {
    path: String,
    content: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CreateFileOutput {
    action: Action,
    path: String,
}

pub(crate) struct CreateFile {
    root: PathBuf,
}

impl CreateFile {
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
            return Err(invalid_path("Cannot create file: path must not be empty."));
        }

        let mut normalized = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::CurDir => {}
                Component::ParentDir => return Err(outside_current_directory(path)),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(invalid_path(format!(
                        "Cannot create file \"{path}\": path must be relative to the current directory."
                    )));
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(invalid_path(format!(
                "Cannot create file \"{path}\": path must name a file."
            )));
        }
        Ok(normalized)
    }

    async fn ensure_parent_chain(
        &self,
        relative_parent: &Path,
        original: &str,
    ) -> Result<PathBuf, ToolExecutionError> {
        let mut current = self.root.clone();
        for component in relative_parent.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            current.push(name);
            ensure_directory_component(&self.root, &current, original).await?;
        }
        Ok(current)
    }
}

impl Tool for CreateFile {
    const NAME: &'static str = "create_file";
    type Args = CreateFileArgs;
    type Output = CreateFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Create a new file and missing parent directories in the current directory.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File path relative to the current directory."
                },
                "content": {
                    "type": "string",
                    "description": "Complete file content; an empty string creates an empty file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let relative = Self::normalize_relative_path(&args.path)?;
        let parent = relative.parent().ok_or_else(|| {
            invalid_path(format!(
                "Cannot create file \"{}\": parent is missing.",
                args.path
            ))
        })?;
        let resolved_parent = self.ensure_parent_chain(parent, &args.path).await?;
        let name = relative.file_name().ok_or_else(|| {
            invalid_path(format!(
                "Cannot create file \"{}\": path must name a file.",
                args.path
            ))
        })?;
        let target = resolved_parent.join(name);
        create_file_atomically(&target, &args.content, &args.path).await?;
        Ok(CreateFileOutput {
            action: Action::Created,
            path: relative.to_string_lossy().into_owned(),
        })
    }
}

async fn ensure_directory_component(
    root: &Path,
    path: &Path,
    original: &str,
) -> Result<(), ToolExecutionError> {
    match fs::symlink_metadata(path).await {
        Ok(_) => validate_directory_component(root, path, original).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(path).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    validate_directory_component(root, path, original).await
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
        Err(invalid_path_type(
            original,
            "a parent path is not a directory",
        ))
    }
}

async fn create_file_atomically(
    target: &Path,
    content: &str,
    original: &str,
) -> Result<(), ToolExecutionError> {
    let (temporary_path, mut temporary) = create_temporary_file(target, original).await?;
    if let Err(error) = write_temporary_content(&mut temporary, content).await {
        drop(temporary);
        return Err(cleanup_temporary_file(
            temporary_path,
            create_error(original, error),
            original,
        )
        .await);
    }
    drop(temporary);
    match fs::hard_link(&temporary_path, target).await {
        Ok(()) => remove_temporary_link(temporary_path, original).await,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let cleanup = cleanup_temporary_file(
                temporary_path,
                create_exists_error(target, original).await,
                original,
            )
            .await;
            Err(cleanup)
        }
        Err(error) => {
            Err(
                cleanup_temporary_file(temporary_path, create_error(original, error), original)
                    .await,
            )
        }
    }
}

async fn create_temporary_file(
    target: &Path,
    original: &str,
) -> Result<(PathBuf, File), ToolExecutionError> {
    for _ in 0..TEMP_FILE_ATTEMPTS {
        let path = temporary_path(target)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(create_error(original, error)),
        }
    }
    Err(ToolExecutionError::other(format!(
        "Cannot create file \"{original}\": could not create a unique temporary file."
    ))
    .with_code(error_codes::IO_ERROR))
}

fn temporary_path(target: &Path) -> Result<PathBuf, ToolExecutionError> {
    let parent = target
        .parent()
        .ok_or_else(|| invalid_path("Cannot create file: parent is missing."))?;
    let name = target
        .file_name()
        .ok_or_else(|| invalid_path("Cannot create file: name is missing."))?;
    let id = NEXT_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{}.rigel-{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        id
    )))
}

async fn write_temporary_content(file: &mut File, content: &str) -> std::io::Result<()> {
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;
    file.sync_all().await
}

async fn remove_temporary_link(path: PathBuf, original: &str) -> Result<(), ToolExecutionError> {
    fs::remove_file(path)
        .await
        .map_err(|error| create_error(original, error))
}

async fn cleanup_temporary_file(
    path: PathBuf,
    primary: ToolExecutionError,
    original: &str,
) -> ToolExecutionError {
    match fs::remove_file(path).await {
        Ok(()) => primary,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => primary,
        Err(error) => create_error(original, error),
    }
}

async fn create_exists_error(target: &Path, original: &str) -> ToolExecutionError {
    match fs::symlink_metadata(target).await {
        Ok(metadata) if metadata.is_file() => ToolExecutionError::invalid_args(format!(
            "Cannot create file \"{original}\": file already exists. Use apply_patch to update it."
        ))
        .with_code(error_codes::PATH_ALREADY_EXISTS),
        Ok(metadata) if metadata.is_dir() => invalid_path_type(original, "path is a directory"),
        Ok(_) => invalid_path_type(original, "path is not a regular file"),
        Err(error) => create_error(original, error),
    }
}

fn invalid_path(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

fn invalid_path_type(path: &str, reason: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!("Cannot create file \"{path}\": {reason}."))
        .with_code(error_codes::INVALID_PATH_TYPE)
}

fn outside_current_directory(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot create file \"{path}\": path resolves outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn create_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot create file \"{path}\": a parent path does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot create file \"{path}\": permission denied.")
        }
        _ => format!("Cannot create file \"{path}\": {error}"),
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
    use rig::tool::ToolErrorKind;

    use super::*;

    #[test]
    fn schema_requires_complete_content_and_rejects_extra_fields() {
        let tool = CreateFile {
            root: PathBuf::from("."),
        };
        let schema = tool.parameters();

        assert_eq!(schema["required"], serde_json::json!(["path", "content"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn empty_content_is_valid_input() {
        let args = serde_json::from_value::<CreateFileArgs>(serde_json::json!({
            "path": "empty.txt",
            "content": ""
        }));

        assert!(args.is_ok());
    }

    #[test]
    fn existing_file_error_has_one_recovery_action() {
        let error = ToolExecutionError::invalid_args(
            "Cannot create file \"notes.txt\": file already exists. Use apply_patch to update it.",
        )
        .with_code(error_codes::PATH_ALREADY_EXISTS);

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(error.code(), Some(error_codes::PATH_ALREADY_EXISTS));
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("apply_patch"))
        );
    }

    #[test]
    fn parent_paths_are_created_only_inside_current_directory() {
        let error = CreateFile::normalize_relative_path("../file").expect_err("path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.code(),
            Some(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
        );
    }
}
