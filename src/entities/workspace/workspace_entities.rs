use std::path::PathBuf;

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

/// A bounded byte read returned by a workspace reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceRead {
    /// Bytes returned from the requested path, within the adapter's bound.
    pub(crate) bytes: Vec<u8>,
    /// Whether the adapter found more bytes than it returned.
    pub(crate) truncated: bool,
}

/// A bounded directory listing returned by a workspace scanner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceDirectoryEntries {
    /// Child paths returned from the requested directory.
    pub(crate) paths: Vec<PathBuf>,
    /// Whether at least one additional child was omitted by the adapter.
    pub(crate) truncated: bool,
}
