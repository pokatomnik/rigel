use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::tools::revision::sha256;

#[derive(Deserialize)]
pub(crate) struct ReadFileArgs {
    path: String,
    line_start: Option<usize>,
    line_end: Option<usize>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ReadFileOutput {
    path: String,
    line_start: Option<usize>,
    line_end: Option<usize>,
    revision: String,
    content: String,
}

#[derive(Debug)]
struct SelectedContent {
    line_start: Option<usize>,
    line_end: Option<usize>,
    content: String,
}

pub(crate) struct ReadFile {
    root: PathBuf,
}

impl ReadFile {
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
                "Cannot read file: path must not be empty.",
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
                        "Cannot read file \"{path}\": path must be relative to the project root."
                    )));
                }
            }
        }

        if normalized.as_os_str().is_empty() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot read file \"{path}\": path must name a file."
            )));
        }

        Ok(normalized)
    }

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, String), ToolExecutionError> {
        let relative_path = Self::normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative_path))
            .await
            .map_err(|error| read_error(path, error))?;

        if !resolved.starts_with(&self.root) {
            return Err(outside_project_error(path));
        }

        Ok((resolved, relative_path.to_string_lossy().into_owned()))
    }
}

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";
    type Args = ReadFileArgs;
    type Output = ReadFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Read all or a 1-based inclusive line range and return the file's SHA-256 revision."
            .to_string()
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
                "line_start": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional first line to return, starting from 1."
                },
                "line_end": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional last line to return, inclusive."
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
        validate_range(args.line_start, args.line_end, &args.path)?;

        let (resolved, path) = self.resolve_path(&args.path).await?;
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| read_error(&args.path, error))?;

        if !metadata.is_file() {
            return Err(ToolExecutionError::invalid_args(format!(
                "Cannot read file \"{}\": path is not a regular file.",
                args.path
            )));
        }

        let content = fs::read_to_string(&resolved)
            .await
            .map_err(|error| read_error(&args.path, error))?;
        let revision = sha256(content.as_bytes());
        let selected = select_content(&content, args.line_start, args.line_end, &args.path)?;

        Ok(ReadFileOutput {
            path,
            line_start: selected.line_start,
            line_end: selected.line_end,
            revision,
            content: selected.content,
        })
    }
}

fn validate_range(
    line_start: Option<usize>,
    line_end: Option<usize>,
    path: &str,
) -> Result<(), ToolExecutionError> {
    if line_start == Some(0) {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot read file \"{path}\": line_start must be at least 1."
        )));
    }

    if line_end == Some(0) {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot read file \"{path}\": line_end must be at least 1."
        )));
    }

    if let (Some(line_start), Some(line_end)) = (line_start, line_end)
        && line_start > line_end
    {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot read file \"{path}\": line_start ({line_start}) must be less than or equal to line_end ({line_end})."
        )));
    }

    Ok(())
}

fn select_content(
    content: &str,
    line_start: Option<usize>,
    line_end: Option<usize>,
    path: &str,
) -> Result<SelectedContent, ToolExecutionError> {
    if line_start.is_none() && line_end.is_none() {
        return Ok(SelectedContent {
            line_start: None,
            line_end: None,
            content: content.to_string(),
        });
    }

    validate_range(line_start, line_end, path)?;

    let lines = content.lines().collect::<Vec<_>>();
    let line_count = lines.len();
    let selected_start = line_start.unwrap_or(1);

    if let Some(line_end) = line_end
        && line_end > line_count
    {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot read file \"{path}\" through line {line_end}: file has only {line_count} lines."
        )));
    }

    if selected_start > line_count {
        return Err(ToolExecutionError::invalid_args(format!(
            "Cannot read file \"{path}\" from line {selected_start}: file has {line_count} lines."
        )));
    }

    let selected_end = line_end.unwrap_or(line_count);

    Ok(SelectedContent {
        line_start: Some(selected_start),
        line_end: Some(selected_end),
        content: lines[selected_start - 1..selected_end].join("\n"),
    })
}

fn outside_project_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot read file \"{path}\": path resolves outside the project root."
    ))
}

fn read_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let message = match error.kind() {
        std::io::ErrorKind::NotFound => {
            format!("Cannot read file \"{path}\": file does not exist.")
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("Cannot read file \"{path}\": permission denied.")
        }
        std::io::ErrorKind::InvalidData => {
            format!("Cannot read file \"{path}\": file is not valid UTF-8 text.")
        }
        std::io::ErrorKind::IsADirectory => {
            format!("Cannot read file \"{path}\": path is a directory.")
        }
        std::io::ErrorKind::NotADirectory => {
            format!("Cannot read file \"{path}\": a path component is not a directory.")
        }
        _ => format!("Cannot read file \"{path}\": {error}"),
    };

    match error.kind() {
        std::io::ErrorKind::NotFound => ToolExecutionError::not_found(message),
        std::io::ErrorKind::PermissionDenied => ToolExecutionError::permission_denied(message),
        std::io::ErrorKind::InvalidData
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
    fn full_file_read_preserves_original_content() {
        let selected = select_content("first\r\nsecond\n", None, None, "file.txt")
            .expect("full content should be selected");

        assert_eq!(selected.line_start, None);
        assert_eq!(selected.line_end, None);
        assert_eq!(selected.content, "first\r\nsecond\n");
    }

    #[test]
    fn inclusive_line_range_is_selected() {
        let selected = select_content("one\ntwo\nthree\nfour", Some(2), Some(3), "file.txt")
            .expect("range should be selected");

        assert_eq!(selected.line_start, Some(2));
        assert_eq!(selected.line_end, Some(3));
        assert_eq!(selected.content, "two\nthree");
    }

    #[test]
    fn omitted_range_bound_uses_file_boundary() {
        let from_second = select_content("one\ntwo\nthree", Some(2), None, "file.txt")
            .expect("range should be selected");
        let through_second = select_content("one\ntwo\nthree", None, Some(2), "file.txt")
            .expect("range should be selected");

        assert_eq!(from_second.content, "two\nthree");
        assert_eq!(from_second.line_end, Some(3));
        assert_eq!(through_second.content, "one\ntwo");
        assert_eq!(through_second.line_start, Some(1));
    }

    #[test]
    fn line_end_beyond_file_is_a_clear_error() {
        let error = select_content("one\ntwo", Some(1), Some(3), "file.txt")
            .expect_err("out-of-range line_end should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some("Cannot read file \"file.txt\" through line 3: file has only 2 lines.")
        );
    }

    #[test]
    fn reversed_range_is_a_clear_error() {
        let error =
            validate_range(Some(3), Some(2), "file.txt").expect_err("reversed range should fail");

        assert_eq!(error.kind(), ToolErrorKind::InvalidArgs);
        assert_eq!(
            error.model_feedback(),
            Some(
                "Cannot read file \"file.txt\": line_start (3) must be less than or equal to line_end (2)."
            )
        );
    }

    #[test]
    fn outside_path_is_refused() {
        let error =
            ReadFile::normalize_relative_path("../outside").expect_err("outside path should fail");

        assert!(error.is_refusal());
        assert_eq!(
            error.model_feedback(),
            Some("Cannot read file \"../outside\": path resolves outside the project root.")
        );
    }
}
