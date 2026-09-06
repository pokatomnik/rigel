use std::path::{Component, Path, PathBuf};

use rig::tool::ToolExecutionError;

use super::errors::{invalid_argument, path_outside_workspace};

/// Maximum UTF-8 byte length accepted for one complete file write.
pub(super) const MAX_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_PATH_BYTES: usize = 4 * 1024;

/// Validates the path and complete content before any filesystem access.
pub(super) fn validate_arguments(path: &str, content: &str) -> Result<(), ToolExecutionError> {
    if path.trim().is_empty() || path.contains('\0') || path.len() > MAX_PATH_BYTES {
        return Err(invalid_argument(
            "path must be a non-empty relative path without NUL and within the server path limit; correct the path and retry.",
        ));
    }
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
    let parsed = PathBuf::from(path);
    if parsed.is_absolute() {
        return Err(path_outside_workspace(path));
    }
    let mut normalized = PathBuf::new();
    for component in parsed.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            Component::ParentDir => return Err(path_outside_workspace(path)),
            Component::RootDir | Component::Prefix(_) => {
                return Err(path_outside_workspace(path));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(invalid_argument(
            "path must name a file, not the workspace directory; provide a relative file path.",
        ));
    }
    Ok(normalized)
}

/// Formats a normalized workspace-relative path consistently for model-visible output.
pub(super) fn display_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_supported_text(text: &str) -> bool {
    !text.chars().any(|character| {
        character.is_control() && !matches!(character, '\t' | '\n' | '\r' | '\u{c}')
    })
}
