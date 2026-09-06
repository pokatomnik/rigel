use std::path::{Path, PathBuf};

use rig::tool::ToolExecutionError;

use crate::tools::utils::{
    errors::invalid_argument,
    path::{MAX_PATH_BYTES, display_relative_path, normalize_relative_path},
    text::is_supported_text,
};

/// Maximum UTF-8 byte length accepted for one complete file write.
pub(super) const MAX_CONTENT_BYTES: usize = 1024 * 1024;

/// Validates the path and complete content before any filesystem access.
pub(super) fn validate_arguments(path: &str, content: &str) -> Result<(), ToolExecutionError> {
    validate_path(path)?;
    if content.len() > MAX_CONTENT_BYTES {
        return Err(invalid_argument(
            "content exceeds the 1048576-byte server limit; provide a smaller complete file.",
        ));
    }
    if !is_supported_text(content) {
        return Err(invalid_argument(
            "content must be UTF-8 text without unsupported control characters; provide plain text and retry.",
        ));
    }
    Ok(())
}

/// Validates and normalizes a relative workspace path without following symlinks.
pub(super) fn validate_path(path: &str) -> Result<PathBuf, ToolExecutionError> {
    normalize_relative_path(path, MAX_PATH_BYTES, "write")
}

/// Formats a normalized workspace-relative path consistently for model-visible output.
pub(super) fn display_path(path: &Path) -> String {
    display_relative_path(path)
}
