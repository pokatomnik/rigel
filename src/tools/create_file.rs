use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

#[derive(Deserialize)]
pub(crate) struct CreateFileArgs {
    path: String,
    content: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CreateFileOutput {
    path: String,
    status: CreateFileStatus,
    content: CreateFileContent,
    message: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CreateFileStatus {
    Created,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CreateFileContent {
    Empty,
    Written,
}

pub(crate) struct CreateFile {
    root: PathBuf,
}

impl CreateFile {
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
                "Cannot create file: path must not be empty.",
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
                        "Cannot create file \"{path}\": path must be relative to the project root."
                    )));
                }
            }
        }

        if normalized.as_os_str().is_empty() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot create file \"{path}\": path must name a file."
            )));
        }

        Ok(normalized)
    }

    async fn existing_path_error(&self, target: &Path, original: &str) -> ToolExecutionError {
        match fs::symlink_metadata(target).await {
            Ok(metadata) if metadata.is_file() => file_exists_error(original),
            Ok(metadata) if metadata.is_dir() => ToolExecutionError::invalid_args(format!(
                "Cannot create file \"{original}\": path already exists as a directory."
            )),
            Ok(_) => ToolExecutionError::invalid_args(format!(
                "Cannot create file \"{original}\": path already exists and is not a regular file."
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ToolExecutionError::other(format!(
                    "Cannot create file \"{original}\": path changed during creation; retry the operation."
                ))
                .with_retryable(true)
                .with_source(error)
            }
            Err(error) => create_error(original, error),
        }
    }
}

impl Tool for CreateFile {
    const NAME: &'static str = "create_file";
    type Args = CreateFileArgs;
    type Output = CreateFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Create a new file inside the project.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File path relative to the project root."
                },
                "content": {
                    "type": "string",
                    "description": "Optional content to write to the new file."
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
        let parent = target.parent().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot create file \"{}\": parent directory could not be determined.",
                args.path
            ))
        })?;
        let resolved_parent = fs::canonicalize(parent)
            .await
            .map_err(|error| create_error(&args.path, error))?;

        if !resolved_parent.starts_with(&self.root) {
            return Err(outside_project_error(&args.path));
        }

        let parent_metadata = fs::metadata(&resolved_parent)
            .await
            .map_err(|error| create_error(&args.path, error))?;

        if !parent_metadata.is_dir() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot create file \"{}\": parent path is not a directory.",
                args.path
            )));
        }

        let file_name = target.file_name().ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "Cannot create file \"{}\": path must name a file.",
                args.path
            ))
        })?;
        let final_target = resolved_parent.join(file_name);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&final_target)
            .await
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(self.existing_path_error(&final_target, &args.path).await);
            }
            Err(error) => return Err(create_error(&args.path, error)),
        };

        if let Some(content) = args.content {
            if let Err(error) = write_content(&mut file, &content).await {
                drop(file);
                return Err(rollback_after_write_error(&final_target, &args.path, error).await);
            }

            Ok(file_result(&args.path, CreateFileContent::Written))
        } else {
            Ok(file_result(&args.path, CreateFileContent::Empty))
        }
    }
}

async fn write_content(file: &mut File, content: &str) -> std::io::Result<()> {
    file.write_all(content.as_bytes()).await?;
    file.flush().await
}

async fn rollback_after_write_error(
    target: &Path,
    original: &str,
    write_error: std::io::Error,
) -> ToolExecutionError {
    match fs::remove_file(target).await {
        Ok(()) => ToolExecutionError::other(format!(
            "File \"{original}\" was created, but writing its content failed; the incomplete file was removed: {write_error}"
        ))
        .with_source(write_error),
        Err(cleanup_error) => ToolExecutionError::other(format!(
            "File \"{original}\" was created, but writing its content failed and the incomplete file could not be removed. Write error: {write_error}. Cleanup error: {cleanup_error}"
        ))
        .with_source(write_error),
    }
}

fn file_result(path: &str, content: CreateFileContent) -> CreateFileOutput {
    let message = match content {
        CreateFileContent::Empty => format!("Empty file \"{path}\" was created."),
        CreateFileContent::Written => {
            format!("File \"{path}\" was created and the provided content was written to it.")
        }
    };

    CreateFileOutput {
        path: path.to_string(),
        status: CreateFileStatus::Created,
        content,
        message,
    }
}

fn file_exists_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot create file \"{path}\": file already exists. Use apply_patch to update it."
    ))
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot create file \"{path}\": path resolves outside the project root."
    ))
}

fn create_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!(
                "Cannot create file \"{path}\": parent directory does not exist. Create it first with create_directory."
            )
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot create file \"{path}\": permission denied.")
        }
        std::io::ErrorKind::AlreadyExists => {
            format!(
                "Cannot create file \"{path}\": path already exists. Use apply_patch to update an existing file."
            )
        }
        std::io::ErrorKind::IsADirectory => {
            format!("Cannot create file \"{path}\": path is a directory.")
        }
        std::io::ErrorKind::NotADirectory => {
            format!("Cannot create file \"{path}\": a parent path is not a directory.")
        }
        _ => format!("Cannot create file \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::AlreadyExists
        | std::io::ErrorKind::IsADirectory
        | std::io::ErrorKind::NotADirectory => ToolExecutionError::invalid_args(message),
        _ => ToolExecutionError::other(message),
    }
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolErrorKind;

    use super::*;

    #[test]
    fn empty_file_result_is_explicit() {
        let result = file_result("empty.txt", CreateFileContent::Empty);

        assert_eq!(result.status, CreateFileStatus::Created);
        assert_eq!(result.content, CreateFileContent::Empty);
        assert_eq!(result.message, "Empty file \"empty.txt\" was created.");
    }

    #[test]
    fn written_file_result_does_not_expose_content() {
        let result = file_result("notes.txt", CreateFileContent::Written);

        assert_eq!(result.status, CreateFileStatus::Created);
        assert_eq!(result.content, CreateFileContent::Written);
        assert_eq!(
            result.message,
            "File \"notes.txt\" was created and the provided content was written to it."
        );
    }

    #[test]
    fn existing_file_error_suggests_apply_patch() {
        let error = file_exists_error("notes.txt");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot create file \"notes.txt\": file already exists. Use apply_patch to update it."
            )
        );
    }

    #[test]
    fn parent_path_outside_project_is_refused() {
        let error =
            CreateFile::normalize_relative_path("../file").expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot create file \"../file\": path resolves outside the project root.")
        );
    }

    #[test]
    fn project_root_does_not_name_a_file() {
        let error = CreateFile::normalize_relative_path(".").expect_err("project root should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot create file \".\": path must name a file.")
        );
    }

    #[test]
    fn permission_error_is_clear_and_model_visible() {
        let error = create_error("protected.txt", std::io::ErrorKind::PermissionDenied.into());

        assert_eq!(error.kind(), ToolErrorKind::PermissionDenied);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot create file \"protected.txt\": permission denied.")
        );
    }
}
