use std::path::{Component, Path, PathBuf};

use rig::tool::ToolExecutionError;

use super::errors::{invalid_argument, path_outside_workspace};

/// Shared maximum UTF-8 byte length for workspace-relative paths.
pub(crate) const MAX_PATH_BYTES: usize = 4 * 1024;

/// Validates a path before filesystem access while rejecting lexical workspace escapes.
pub(crate) fn validate_relative_path(
    path: &str,
    max_bytes: usize,
    operation: &str,
) -> Result<PathBuf, ToolExecutionError> {
    if path.trim().is_empty() || path.contains('\0') || path.len() > max_bytes {
        return Err(invalid_argument(format!(
            "Cannot {operation} files: path must be non-empty, relative, and within the server path limit."
        )));
    }
    let path_ref = Path::new(path);
    if path_ref.is_absolute() || escapes_workspace(path_ref) {
        return Err(path_outside_workspace(path, operation));
    }
    Ok(path_ref.to_path_buf())
}

/// Validates a read path without rejecting `..` that still resolves inside the workspace.
pub(crate) fn validate_non_absolute_path(
    path: &str,
    operation: &str,
) -> Result<PathBuf, ToolExecutionError> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err(invalid_argument(format!(
            "Cannot {operation} file: path must be a non-empty relative path without NUL characters."
        )));
    }
    let path_ref = Path::new(path);
    if path_ref.is_absolute() {
        return Err(path_outside_workspace(path, operation));
    }
    Ok(path_ref.to_path_buf())
}

/// Normalizes a relative path and rejects an empty result such as `.`.
pub(crate) fn normalize_relative_path(
    path: &str,
    max_bytes: usize,
    operation: &str,
) -> Result<PathBuf, ToolExecutionError> {
    let parsed = validate_relative_path(path, max_bytes, operation)?;
    let mut normalized = PathBuf::new();
    for component in parsed.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            Component::ParentDir => return Err(path_outside_workspace(path, operation)),
            Component::RootDir | Component::Prefix(_) => {
                return Err(path_outside_workspace(path, operation));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(invalid_argument(
            "path must name a file, not the workspace directory; provide a relative file path."
                .to_string(),
        ));
    }
    Ok(normalized)
}

/// Converts a canonical path into a normalized workspace-relative display path.
pub(crate) fn display_workspace_path(
    root: &Path,
    path: &Path,
    requested_path: &str,
    operation: &str,
) -> Result<String, ToolExecutionError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| path_outside_workspace(requested_path, operation))?;
    let display = display_relative_path(relative);
    Ok(if display.is_empty() {
        ".".to_string()
    } else {
        display
    })
}

/// Formats a relative path with stable forward separators for model-facing output.
pub(crate) fn display_relative_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Reports whether a canonical path remains within the canonical workspace root.
pub(crate) fn is_inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

fn escapes_workspace(path: &Path) -> bool {
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth == 0 => return true,
            Component::ParentDir => depth -= 1,
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return true,
        }
    }
    false
}
