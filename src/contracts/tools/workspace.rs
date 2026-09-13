use std::path::{Path, PathBuf};
use std::{future::Future, pin::Pin};

/// Future returned by an injected workspace capability.
pub(crate) type WorkspaceFuture<'a, T> =
    Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// The filesystem entry kinds relevant to safe workspace tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspacePathKind {
    /// A regular file that may be read or written.
    File,
    /// A directory that may be traversed or used as a parent.
    Directory,
    /// A symbolic link, which tools reject or skip to prevent escapes.
    Symlink,
    /// A special or otherwise unsupported filesystem entry.
    Other,
}

/// A bounded byte read returned by the workspace reader.
pub(crate) struct WorkspaceRead {
    /// Bytes returned from the requested path, within the adapter's bound.
    pub(crate) bytes: Vec<u8>,
    /// Whether the adapter found more bytes than it returned.
    pub(crate) truncated: bool,
}

/// A bounded directory listing returned by the workspace scanner.
pub(crate) struct WorkspaceDirectoryEntries {
    /// Child paths returned from the requested directory.
    pub(crate) paths: Vec<PathBuf>,
    /// Whether at least one additional child was omitted by the adapter.
    pub(crate) truncated: bool,
}

/// Resolves safe paths against the one workspace root selected at composition time.
pub(crate) trait WorkspacePathResolver: Send + Sync {
    /// Returns the canonical or otherwise trusted root used for model workspace access.
    fn root(&self) -> PathBuf;

    /// Resolves a validated model-relative path without consulting process-global state.
    fn resolve(&self, path: &str) -> anyhow::Result<PathBuf>;

    /// Canonicalizes an existing path so callers can reject symlink and traversal escapes.
    fn canonicalize(&self, path: &Path) -> WorkspaceFuture<'_, PathBuf>;
}

/// Reads and classifies existing workspace entries through an injected capability.
pub(crate) trait WorkspaceReader: Send + Sync {
    /// Classifies a path without following symbolic links where the adapter supports it.
    fn metadata(&self, path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind>;

    /// Reads at most `max_bytes` plus enough information to report truncation.
    fn read(&self, path: &Path, max_bytes: usize) -> WorkspaceFuture<'_, WorkspaceRead>;
}

/// Performs bounded workspace writes and parent-directory creation.
pub(crate) trait WorkspaceWriter: Send + Sync {
    /// Classifies an existing target before a write is attempted.
    fn metadata(&self, path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind>;

    /// Creates a missing parent directory tree selected by the tool.
    fn create_dir_all(&self, path: &Path) -> WorkspaceFuture<'_, ()>;

    /// Replaces complete file contents and returns only after the write succeeds.
    fn write(&self, path: &Path, contents: &[u8]) -> WorkspaceFuture<'_, ()>;
}

/// Lists workspace entries for bounded filename and content traversal.
pub(crate) trait WorkspaceScanner: Send + Sync {
    /// Classifies an entry without following symbolic links.
    fn path_kind(&self, path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind>;

    /// Reads a bounded child listing and reports whether entries were omitted.
    fn read_directory(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> WorkspaceFuture<'_, WorkspaceDirectoryEntries>;
}
