use std::io;

use rig::tool::ToolExecutionError;

use crate::tools::error_codes;

/// Classifies path reads while preserving a single corrective next step.
pub(super) fn file_access_error(path: &str, error: io::Error) -> ToolExecutionError {
    if error.kind() == io::ErrorKind::NotFound {
        return ToolExecutionError::not_found(format!(
            "File \"{path}\" was not found. Check the relative path and reread the file."
        ))
        .with_code(error_codes::NOT_FOUND)
        .with_source(error);
    }
    io_error(path, error)
}

/// Builds the error used when workspace initialization or reading fails.
pub(super) fn io_error(path: impl std::fmt::Display, error: io::Error) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot edit \"{path}\": filesystem access failed. Check the path and retry the edit."
    ))
    .with_code(error_codes::IO_ERROR)
    .with_source(error)
}

/// Reports direct-write failure without exposing a success-shaped response.
pub(super) fn write_error(path: &str, error: io::Error) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot edit \"{path}\": write failed; no successful edit was reported. Check the path and retry."
    ))
    .with_code(error_codes::IO_ERROR)
    .with_source(error)
}

/// Tells the model to reread after a second read differs from the first.
pub(super) fn stale_content_error(path: &str) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "STALE_CONTENT: file \"{path}\" changed after it was read. Reread the file and retry with current old_text."
    ))
    .with_code(error_codes::STALE_CONTENT)
}
