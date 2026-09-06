use std::{
    env, io,
    path::{Component, Path, PathBuf},
};

use rig::tool::ToolExecutionError;
use tokio::io::AsyncReadExt;

use crate::tools::error_codes;

/// Filesystem entry classes used to prevent symlink traversal and special-file reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathKind {
    File,
    Directory,
    Symlink,
    Other,
}

/// Bounded file bytes plus whether the source had more unread bytes.
pub(crate) struct ReadPrefix {
    /// Bytes read within the caller's scan budget.
    pub(crate) bytes: Vec<u8>,
    /// Whether the source exceeded the requested prefix limit.
    pub(crate) truncated: bool,
}

/// Bounded directory listing with an indicator for omitted entries.
pub(crate) struct DirectoryEntries {
    /// Child paths collected in the listing budget.
    pub(crate) paths: Vec<std::path::PathBuf>,
    /// Whether at least one additional child was detected.
    pub(crate) truncated: bool,
}

/// Resolves the process startup directory once for workspace confinement.
pub(crate) async fn startup_root() -> Result<PathBuf, ToolExecutionError> {
    let current_dir = env::current_dir().map_err(|error| io_error("workspace", error))?;
    tokio::fs::canonicalize(&current_dir)
        .await
        .map_err(|error| io_error(current_dir.display(), error))
}

/// Validates relative paths and rejects lexical traversal before filesystem access.
pub(crate) fn validate_path(path: &str, max_bytes: usize) -> Result<PathBuf, ToolExecutionError> {
    if path.trim().is_empty() || path.contains('\0') || path.len() > max_bytes {
        return Err(invalid_path_argument(
            "Cannot search files: path must be non-empty, relative, and within the server path limit.",
        ));
    }
    let path_ref = Path::new(path);
    if path_ref.is_absolute() || escapes_workspace(path_ref) {
        return Err(path_outside_workspace(path));
    }
    Ok(path_ref.to_path_buf())
}

/// Canonicalizes a path and rejects symlink or traversal escapes from `root`.
pub(crate) async fn canonical_path(
    root: &Path,
    candidate: &Path,
    requested_path: &str,
) -> Result<PathBuf, ToolExecutionError> {
    let canonical = tokio::fs::canonicalize(candidate)
        .await
        .map_err(|error| file_access_error(requested_path, error))?;
    if !canonical.starts_with(root) {
        return Err(path_outside_workspace(requested_path));
    }
    Ok(canonical)
}

/// Reads entry type without following symlinks so callers can reject them safely.
pub(crate) async fn path_kind(path: &Path) -> io::Result<PathKind> {
    let file_type = tokio::fs::symlink_metadata(path).await?.file_type();
    Ok(if file_type.is_symlink() {
        PathKind::Symlink
    } else if file_type.is_file() {
        PathKind::File
    } else if file_type.is_dir() {
        PathKind::Directory
    } else {
        PathKind::Other
    })
}

/// Reads at most one bounded prefix and reports whether the file was longer.
pub(crate) async fn read_prefix(path: &Path, max_bytes: usize) -> io::Result<ReadPrefix> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    let truncated = bytes.len() > max_bytes;
    if truncated {
        bytes.truncate(max_bytes);
    }
    Ok(ReadPrefix { bytes, truncated })
}

/// Reads a bounded child listing and reports omitted entries deterministically.
pub(crate) async fn read_directory(
    path: &Path,
    max_entries: usize,
) -> io::Result<DirectoryEntries> {
    let mut directory = tokio::fs::read_dir(path).await?;
    let mut paths = Vec::new();
    while paths.len() < max_entries {
        let Some(entry) = directory.next_entry().await? else {
            return Ok(DirectoryEntries {
                paths,
                truncated: false,
            });
        };
        paths.push(entry.path());
    }
    let truncated = directory.next_entry().await?.is_some();
    Ok(DirectoryEntries { paths, truncated })
}

/// Converts a canonical path into a normalized workspace-relative display path.
pub(crate) fn display_path(
    root: &Path,
    path: &Path,
    requested_path: &str,
) -> Result<String, ToolExecutionError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| path_outside_workspace(requested_path))?;
    let display = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(if display.is_empty() {
        ".".to_string()
    } else {
        display
    })
}

/// Identifies known metadata and generated directories that searches must skip.
pub(crate) fn is_excluded_directory(path: &Path) -> bool {
    [
        ".git",
        "target",
        "node_modules",
        ".venv",
        "vendor",
        "dist",
        "build",
        ".next",
    ]
    .iter()
    .any(|excluded| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == *excluded)
    })
}

/// Converts filesystem failures into model-facing not-found or I/O errors.
pub(crate) fn file_access_error(
    path: impl std::fmt::Display,
    error: io::Error,
) -> ToolExecutionError {
    if error.kind() == io::ErrorKind::NotFound {
        return ToolExecutionError::not_found(format!(
            "Path \"{path}\" was not found. Check the relative path and retry."
        ))
        .with_code(error_codes::NOT_FOUND)
        .with_source(error);
    }
    io_error(path, error)
}

/// Builds a structured workspace I/O error with one corrective message.
pub(crate) fn io_error(path: impl std::fmt::Display, error: io::Error) -> ToolExecutionError {
    ToolExecutionError::other(format!(
        "Cannot search \"{path}\": filesystem access failed. Check the path and retry."
    ))
    .with_code(error_codes::IO_ERROR)
    .with_source(error)
}

/// Builds the structured error used for absolute, traversing, or escaped paths.
pub(crate) fn path_outside_workspace(path: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot search path \"{path}\": it resolves outside the workspace. Use a relative path inside the workspace."
    ))
    .with_code(error_codes::PATH_OUTSIDE_WORKSPACE)
}

/// Builds an invalid-argument error for rejected path syntax or size.
pub(crate) fn invalid_path_argument(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

/// Builds an invalid-argument error for unsupported filesystem entry types.
pub(crate) fn invalid_search_path(path: impl std::fmt::Display) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot search \"{path}\": choose a regular file or directory inside the workspace."
    ))
    .with_code(error_codes::INVALID_ARGUMENT)
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
