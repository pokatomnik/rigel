use std::io;

use rig::tool::ToolExecutionError;

use crate::tools::error_codes;

/// Builds an invalid-argument error with the shared tool error code.
pub(crate) fn invalid_argument(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

/// Classifies a filesystem failure as not-found or generic I/O for a tool operation.
pub(crate) fn file_access_error(
    path: impl std::fmt::Display,
    error: io::Error,
    operation: &str,
) -> ToolExecutionError {
    if error.kind() == io::ErrorKind::NotFound {
        return ToolExecutionError::not_found(format!(
            "Path \"{path}\" was not found. Check the relative path and retry."
        ))
        .with_code(error_codes::NOT_FOUND)
        .with_source(error);
    }
    io_error(path, error, operation)
}

/// Builds a structured generic filesystem error for a named tool operation.
pub(crate) fn io_error(
    path: impl std::fmt::Display,
    error: io::Error,
    operation: &str,
) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot {operation} \"{path}\": filesystem access failed. Check the path and retry."
    ))
    .with_code(error_codes::IO_ERROR)
    .with_source(error)
}

/// Builds the shared error used for absolute, traversing, or escaped paths.
pub(crate) fn path_outside_workspace(path: &str, operation: &str) -> ToolExecutionError {
    invalid_argument(format!(
        "Cannot {operation} file \"{path}\": the path resolves outside the workspace. Use a relative path inside the workspace."
    ))
    .with_code(error_codes::PATH_OUTSIDE_WORKSPACE)
}
