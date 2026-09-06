use std::{io, path::PathBuf};

use futures::future::BoxFuture;

/// Describes the target kind returned by the workspace filesystem boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileKind {
    Regular,
    Directory,
    Other,
}

/// Abstracts workspace path, metadata, parent-directory, and write operations for unit tests.
pub(super) trait FileSystem: Send + Sync {
    /// Resolves an existing path while preserving filesystem error details.
    fn canonicalize(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathBuf>>;
    /// Checks whether the resolved path is an ordinary regular file or another target kind.
    fn metadata(&self, path: PathBuf) -> BoxFuture<'static, io::Result<FileKind>>;
    /// Creates missing parent directories within the validated workspace.
    fn create_dir_all(&self, path: PathBuf) -> BoxFuture<'static, io::Result<()>>;
    /// Writes complete contents to the target; failures are returned without a success result.
    fn write(&self, path: PathBuf, contents: Vec<u8>) -> BoxFuture<'static, io::Result<()>>;
}

/// Production filesystem adapter using Tokio.
pub(super) struct TokioFileSystem;

impl FileSystem for TokioFileSystem {
    fn canonicalize(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathBuf>> {
        Box::pin(tokio::fs::canonicalize(path))
    }

    fn metadata(&self, path: PathBuf) -> BoxFuture<'static, io::Result<FileKind>> {
        Box::pin(async move {
            tokio::fs::metadata(path).await.map(|metadata| {
                if metadata.is_file() {
                    FileKind::Regular
                } else if metadata.is_dir() {
                    FileKind::Directory
                } else {
                    FileKind::Other
                }
            })
        })
    }

    fn create_dir_all(&self, path: PathBuf) -> BoxFuture<'static, io::Result<()>> {
        Box::pin(tokio::fs::create_dir_all(path))
    }

    fn write(&self, path: PathBuf, contents: Vec<u8>) -> BoxFuture<'static, io::Result<()>> {
        Box::pin(tokio::fs::write(path, contents))
    }
}
