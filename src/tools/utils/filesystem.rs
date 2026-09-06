use std::{
    env,
    future::Future,
    io,
    path::{Path, PathBuf},
};

use rig::tool::ToolExecutionError;
use tokio::io::AsyncReadExt;

use super::{
    errors::{file_access_error, invalid_argument, io_error, path_outside_workspace},
    path::is_inside,
};

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

/// Resolves a startup directory through an injected canonicalization boundary.
pub(crate) async fn startup_root_with<F, Fut>(canonicalize: F) -> io::Result<PathBuf>
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = io::Result<PathBuf>>,
{
    let current_dir = env::current_dir()?;
    canonicalize(current_dir).await
}

/// Resolves the process startup directory for a direct Tokio filesystem boundary.
pub(crate) async fn startup_root(operation: &str) -> Result<PathBuf, ToolExecutionError> {
    startup_root_with(tokio::fs::canonicalize)
        .await
        .map_err(|error| io_error("workspace", error, operation))
}

/// Canonicalizes a search path and rejects symlink or traversal escapes from `root`.
pub(crate) async fn canonical_path(
    root: &Path,
    candidate: &Path,
    requested_path: &str,
) -> Result<PathBuf, ToolExecutionError> {
    let canonical = tokio::fs::canonicalize(candidate)
        .await
        .map_err(|error| file_access_error(requested_path, error, "search"))?;
    if !is_inside(root, &canonical) {
        return Err(path_outside_workspace(requested_path, "search"));
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

/// Builds an invalid-argument error for unsupported filesystem entry types.
pub(crate) fn invalid_search_path(path: impl std::fmt::Display) -> ToolExecutionError {
    invalid_argument(format!(
        "Cannot search \"{path}\": choose a regular file or directory inside the workspace."
    ))
}
