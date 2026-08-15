use std::{
    env,
    path::{Component, Path, PathBuf},
};

use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::tools::{contracts::error_codes, revision::sha256};

const DEFAULT_MAX_LINES: usize = 200;
const MAX_LINES: usize = 500;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileArgs {
    path: String,
    start_line: Option<usize>,
    max_lines: Option<usize>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ReadFileOutput {
    path: String,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    has_more: bool,
    revision: String,
    content: String,
}

#[derive(Debug, PartialEq, Eq)]
struct LineSelection {
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    has_more: bool,
    content: String,
}

pub(crate) struct ReadFile {
    root: PathBuf,
}

impl ReadFile {
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
            return Err(coded_error(
                error_codes::INVALID_ARGUMENT,
                "Cannot read file: path must not be empty.",
            ));
        }
        let mut normalized = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(component) => normalized.push(component),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(outside_current_directory_error(path));
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(coded_error(
                error_codes::INVALID_ARGUMENT,
                format!("Cannot read file \"{path}\": path must name a file."),
            ));
        }
        Ok(normalized)
    }

    async fn resolve_path(&self, path: &str) -> Result<(PathBuf, String), ToolExecutionError> {
        let relative = Self::normalize_relative_path(path)?;
        let resolved = fs::canonicalize(self.root.join(&relative))
            .await
            .map_err(|error| read_error(path, error))?;
        if !resolved.starts_with(&self.root) {
            return Err(outside_current_directory_error(path));
        }
        Ok((resolved, relative.to_string_lossy().into_owned()))
    }
}

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";
    type Args = ReadFileArgs;
    type Output = ReadFileOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Read a UTF-8 file in line chunks and return its full-content revision.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "File path relative to the current directory."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "First 1-based line to return. The default is 1."
                },
                "max_lines": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LINES,
                    "description": "Maximum number of lines to return. The server limits this value."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let max_lines = validate_range(args.start_line, args.max_lines, &args.path)?;
        let (resolved, path) = self.resolve_path(&args.path).await?;
        let metadata = fs::metadata(&resolved)
            .await
            .map_err(|error| read_error(&args.path, error))?;
        if !metadata.is_file() {
            return Err(coded_error(
                error_codes::INVALID_PATH_TYPE,
                format!(
                    "Cannot read file \"{}\": path is not a regular file.",
                    args.path
                ),
            ));
        }
        let bytes = fs::read(&resolved)
            .await
            .map_err(|error| read_error(&args.path, error))?;
        let revision = sha256(&bytes);
        let content = String::from_utf8(bytes).map_err(|error| {
            read_error(
                &args.path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, error),
            )
        })?;
        let selection = select_lines(&content, args.start_line, max_lines);
        Ok(ReadFileOutput {
            path,
            start_line: selection.start_line,
            end_line: selection.end_line,
            total_lines: selection.total_lines,
            has_more: selection.has_more,
            revision,
            content: selection.content,
        })
    }
}

fn validate_range(
    start_line: Option<usize>,
    max_lines: Option<usize>,
    path: &str,
) -> Result<usize, ToolExecutionError> {
    if start_line == Some(0) {
        return Err(coded_error(
            error_codes::INVALID_ARGUMENT,
            format!("Cannot read file \"{path}\": start_line must be at least 1."),
        ));
    }
    if max_lines == Some(0) {
        return Err(coded_error(
            error_codes::INVALID_ARGUMENT,
            format!("Cannot read file \"{path}\": max_lines must be at least 1."),
        ));
    }
    Ok(max_lines.unwrap_or(DEFAULT_MAX_LINES).min(MAX_LINES))
}

fn select_lines(content: &str, start_line: Option<usize>, max_lines: usize) -> LineSelection {
    let lines = content.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let requested_start = start_line.unwrap_or(1);
    let selected_start = requested_start.min(total_lines.saturating_add(1));
    let selected_end = if selected_start <= total_lines {
        (selected_start + max_lines - 1).min(total_lines)
    } else {
        0
    };
    let content = if selected_end == 0 {
        String::new()
    } else {
        lines[selected_start - 1..selected_end].join("\n")
    };
    LineSelection {
        start_line: selected_start,
        end_line: selected_end,
        total_lines,
        has_more: selected_end < total_lines,
        content,
    }
}

fn outside_current_directory_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::refused(format!(
        "Cannot read file \"{path}\": path is outside the current directory."
    ))
    .with_code(error_codes::PATH_OUTSIDE_CURRENT_DIRECTORY)
}

fn read_error(path: &str, error: std::io::Error) -> ToolExecutionError {
    let (kind, code, message) = match error.kind() {
        std::io::ErrorKind::NotFound => (
            ToolErrorKind::NotFound,
            error_codes::PATH_NOT_FOUND,
            format!("Cannot read file \"{path}\": file does not exist."),
        ),
        std::io::ErrorKind::PermissionDenied => (
            ToolErrorKind::PermissionDenied,
            error_codes::PERMISSION_DENIED,
            format!("Cannot read file \"{path}\": permission denied."),
        ),
        std::io::ErrorKind::InvalidData => (
            ToolErrorKind::InvalidArgs,
            error_codes::BINARY_FILE,
            format!("Cannot read file \"{path}\": file is not valid UTF-8 text."),
        ),
        _ => (
            ToolErrorKind::Other,
            error_codes::IO_ERROR,
            format!("Cannot read file \"{path}\": {error}"),
        ),
    };
    ToolExecutionError::new(kind, message)
        .with_code(code)
        .with_source(error)
}

fn coded_error(code: &'static str, message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_accepts_only_path_and_optional_chunk_fields() {
        assert!(
            serde_json::from_value::<ReadFileArgs>(serde_json::json!({
                "path": "src/main.rs"
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<ReadFileArgs>(serde_json::json!({
                "path": "src/main.rs",
                "line_end": 10
            }))
            .is_err()
        );
    }

    #[test]
    fn line_selection_uses_defaults_and_reports_more_lines() {
        let selection = select_lines("one\ntwo\nthree", None, 2);

        assert_eq!(selection.start_line, 1);
        assert_eq!(selection.end_line, 2);
        assert_eq!(selection.total_lines, 3);
        assert!(selection.has_more);
        assert_eq!(selection.content, "one\ntwo");
    }

    #[test]
    fn line_selection_continues_from_requested_line() {
        let selection = select_lines("one\ntwo\nthree", Some(2), 2);

        assert_eq!(selection.start_line, 2);
        assert_eq!(selection.end_line, 3);
        assert!(!selection.has_more);
        assert_eq!(selection.content, "two\nthree");
    }

    #[test]
    fn line_selection_clamps_max_lines_and_tolerates_eof() {
        let selection = select_lines("one\ntwo", Some(2), MAX_LINES + 100);
        let beyond_eof = select_lines("one\ntwo", Some(20), 10);

        assert_eq!(selection.end_line, 2);
        assert!(!selection.has_more);
        assert_eq!(beyond_eof.start_line, 3);
        assert_eq!(beyond_eof.end_line, 0);
        assert_eq!(beyond_eof.content, "");
    }

    #[test]
    fn invalid_range_is_model_visible() {
        let error = validate_range(Some(0), None, "file.txt");

        assert!(error.is_err());
        if let Err(error) = error {
            assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
            assert!(
                error
                    .model_feedback()
                    .is_some_and(|message| message.contains("start_line"))
            );
        }
    }

    #[test]
    fn full_content_revision_uses_bytes() {
        assert_eq!(sha256("one\ntwo".as_bytes()), sha256(b"one\ntwo"));
    }
}
