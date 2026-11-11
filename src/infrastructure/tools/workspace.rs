use std::path::{Path, PathBuf};
use std::{future::Future, pin::Pin};

use crate::entities::workspace::workspace_entities::{
    WorkspaceDirectoryEntries, WorkspacePathKind, WorkspaceRead,
};

/// Resolves safe paths against the one workspace root selected at composition time.
pub(crate) trait WorkspacePathResolver: Send + Sync {
    /// Returns the canonical or otherwise trusted root used for model workspace access.
    fn root(&self) -> PathBuf;

    /// Resolves a validated model-relative path without consulting process-global state.
    fn resolve(&self, path: &str) -> anyhow::Result<PathBuf>;

    /// Canonicalizes an existing path so callers can reject symlink and traversal escapes.
    fn canonicalize(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PathBuf>> + Send + '_>>;
}

/// Reads and classifies existing workspace entries through an injected capability.
pub(crate) trait WorkspaceReader: Send + Sync {
    /// Classifies a path without following symbolic links where the adapter supports it.
    fn metadata(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkspacePathKind>> + Send + '_>>;

    /// Reads at most `max_bytes` plus enough information to report truncation.
    fn read(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkspaceRead>> + Send + '_>>;
}

/// Performs bounded workspace writes and parent-directory creation.
pub(crate) trait WorkspaceWriter: Send + Sync {
    /// Classifies an existing target before a write is attempted.
    fn metadata(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkspacePathKind>> + Send + '_>>;

    /// Creates a missing parent directory tree selected by the tool.
    fn create_dir_all(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Replaces complete file contents and returns only after the write succeeds.
    fn write(
        &self,
        path: &Path,
        contents: &[u8],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
}

/// Lists workspace entries for bounded filename and content traversal.
pub(crate) trait WorkspaceScanner: Send + Sync {
    /// Classifies an entry without following symbolic links.
    fn path_kind(
        &self,
        path: &Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkspacePathKind>> + Send + '_>>;

    /// Reads a bounded child listing and reports whether entries were omitted.
    fn read_directory(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<WorkspaceDirectoryEntries>> + Send + '_>>;
}
