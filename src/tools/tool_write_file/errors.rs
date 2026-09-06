use std::io;

use rig::tool::ToolExecutionError;

use crate::tools::error_codes;

/// Builds an invalid-argument error with the shared tool error code.
pub(super) fn invalid_argument(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

/// Formats a path that canonicalizes outside the startup workspace.
pub(super) fn path_outside_workspace(path: &str) -> ToolExecutionError {
    invalid_argument(format!(
        "Cannot write file \"{path}\": the path resolves outside the workspace. Use a relative path inside the workspace."
    ))
    .with_code(error_codes::PATH_OUTSIDE_WORKSPACE)
}

/// Classifies target metadata failures while preserving actionable recovery guidance.
pub(super) fn file_access_error(path: &str, error: io::Error) -> ToolExecutionError {
    if error.kind() == io::ErrorKind::NotFound {
        return ToolExecutionError::not_found(format!(
            "File \"{path}\" was not found. Check the relative path and retry."
        ))
        .with_code(error_codes::NOT_FOUND)
        .with_source(error);
    }
    if error.kind() == io::ErrorKind::PermissionDenied {
        return permission_error(path, error);
    }
    io_error(path, error)
}

/// Formats a parent-directory failure with a distinct recoverable error code.
pub(super) fn parent_error(path: &str, error: io::Error) -> ToolExecutionError {
    let code = if error.kind() == io::ErrorKind::PermissionDenied {
        error_codes::PERMISSION_DENIED
    } else {
        error_codes::PARENT_ERROR
    };
    ToolExecutionError::other(format!(
        "Cannot prepare parent directories for \"{path}\": check that the path is valid and writable, then retry."
    ))
    .with_code(code)
    .with_source(error)
}

fn permission_error(path: &str, error: io::Error) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot write \"{path}\": permission was denied. Check workspace permissions and retry."
    ))
    .with_code(error_codes::PERMISSION_DENIED)
    .with_source(error)
}

/// Formats a target write failure without pretending that the file was successfully written.
pub(super) fn write_error(path: &str, error: io::Error) -> ToolExecutionError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        return permission_error(path, error);
    }
    if error.kind() == io::ErrorKind::NotFound {
        return ToolExecutionError::not_found(format!(
            "Cannot write \"{path}\": the target or parent was not found. Check the relative path and retry."
        ))
        .with_code(error_codes::NOT_FOUND)
        .with_source(error);
    }
    ToolExecutionError::other(format!(
        "Cannot write \"{path}\": the write failed and no successful result was returned. Check the path and retry."
    ))
    .with_code(error_codes::IO_ERROR)
    .with_source(error)
}

/// Formats workspace initialization and other generic filesystem failures.
pub(super) fn io_error(path: impl std::fmt::Display, error: io::Error) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot access \"{path}\": filesystem access failed. Check the workspace and retry."
    ))
    .with_code(error_codes::IO_ERROR)
    .with_source(error)
}
