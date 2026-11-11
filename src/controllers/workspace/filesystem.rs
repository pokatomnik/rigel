use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::io::AsyncReadExt;

use crate::{
    entities::workspace::workspace_entities::{
        WorkspaceDirectoryEntries, WorkspacePathKind, WorkspaceRead,
    },
    infrastructure::tools::workspace::{
        WorkspacePathResolver, WorkspaceReader, WorkspaceScanner, WorkspaceWriter,
    },
};

type WorkspaceFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// Groups the filesystem capabilities selected by the composition root.
///
/// Tools receive this bundle instead of consulting process-global directories or constructing
/// their own adapters. The four trait objects remain separate so each capability can be replaced
/// independently in tests or by a future remote workspace implementation.
pub(crate) struct WorkspaceCapabilities {
    /// Resolves paths against the selected workspace root.
    pub(crate) path_resolver: Arc<dyn WorkspacePathResolver>,
    /// Reads and classifies existing workspace files.
    pub(crate) reader: Arc<dyn WorkspaceReader>,
    /// Writes files and creates parent directories.
    pub(crate) writer: Arc<dyn WorkspaceWriter>,
    /// Traverses workspace directories for bounded searches.
    pub(crate) scanner: Arc<dyn WorkspaceScanner>,
}

impl WorkspaceCapabilities {
    /// Creates all filesystem capabilities over one already trusted workspace root.
    pub(crate) fn new(root: PathBuf) -> Self {
        let filesystem = Arc::new(LocalWorkspace::new(root));
        Self {
            path_resolver: filesystem.clone(),
            reader: filesystem.clone(),
            writer: filesystem.clone(),
            scanner: filesystem,
        }
    }

    /// Returns the root selected for the workspace capabilities.
    pub(crate) fn root(&self) -> PathBuf {
        self.path_resolver.root()
    }
}

struct LocalWorkspace {
    root: PathBuf,
}

impl LocalWorkspace {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn classify(metadata: &std::fs::Metadata) -> WorkspacePathKind {
        let file_type = metadata.file_type();
        if file_type.is_file() {
            WorkspacePathKind::File
        } else if file_type.is_dir() {
            WorkspacePathKind::Directory
        } else if file_type.is_symlink() {
            WorkspacePathKind::Symlink
        } else {
            WorkspacePathKind::Other
        }
    }

    async fn metadata(path: &Path) -> anyhow::Result<WorkspacePathKind> {
        let metadata = tokio::fs::symlink_metadata(path).await?;
        Ok(Self::classify(&metadata))
    }
}

impl WorkspacePathResolver for LocalWorkspace {
    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn resolve(&self, path: &str) -> anyhow::Result<PathBuf> {
        let relative = Path::new(path);
        if relative.is_absolute() {
            anyhow::bail!("absolute paths are not allowed")
        }
        Ok(self.root.join(relative))
    }

    fn canonicalize(&self, path: &Path) -> WorkspaceFuture<'_, PathBuf> {
        let path = path.to_path_buf();
        Box::pin(async move { Ok(tokio::fs::canonicalize(path).await?) })
    }
}

impl WorkspaceReader for LocalWorkspace {
    fn metadata(&self, path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
        let path = path.to_path_buf();
        Box::pin(async move { Self::metadata(&path).await })
    }

    fn read(&self, path: &Path, max_bytes: usize) -> WorkspaceFuture<'_, WorkspaceRead> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let file = tokio::fs::File::open(path).await?;
            let mut bytes = Vec::with_capacity(max_bytes.saturating_add(1));
            let read_limit = u64::try_from(max_bytes.saturating_add(1))
                .map_err(|_| io::Error::other("workspace read limit is too large"))?;
            let mut limited = file.take(read_limit);
            limited.read_to_end(&mut bytes).await?;
            let truncated = bytes.len() > max_bytes;
            if truncated {
                bytes.truncate(max_bytes);
            }
            Ok(WorkspaceRead { bytes, truncated })
        })
    }
}

impl WorkspaceWriter for LocalWorkspace {
    fn metadata(&self, path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
        let path = path.to_path_buf();
        Box::pin(async move { Self::metadata(&path).await })
    }

    fn create_dir_all(&self, path: &Path) -> WorkspaceFuture<'_, ()> {
        let path = path.to_path_buf();
        Box::pin(async move { Ok(tokio::fs::create_dir_all(path).await?) })
    }

    fn write(&self, path: &Path, contents: &[u8]) -> WorkspaceFuture<'_, ()> {
        let path = path.to_path_buf();
        let contents = contents.to_vec();
        Box::pin(async move { Ok(tokio::fs::write(path, contents).await?) })
    }
}

impl WorkspaceScanner for LocalWorkspace {
    fn path_kind(&self, path: &Path) -> WorkspaceFuture<'_, WorkspacePathKind> {
        let path = path.to_path_buf();
        Box::pin(async move { Self::metadata(&path).await })
    }

    fn read_directory(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> WorkspaceFuture<'_, WorkspaceDirectoryEntries> {
        let path = path.to_path_buf();
        Box::pin(async move {
            let mut directory = tokio::fs::read_dir(path).await?;
            let mut paths = Vec::new();
            let mut truncated = false;
            while let Some(entry) = directory.next_entry().await? {
                if paths.len() >= max_entries {
                    truncated = true;
                    break;
                }
                paths.push(entry.path());
            }
            Ok(WorkspaceDirectoryEntries { paths, truncated })
        })
    }
}
