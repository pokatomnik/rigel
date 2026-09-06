use std::{io, path::PathBuf};

use futures::future::BoxFuture;

use super::validation::MAX_FILE_BYTES;
use crate::tools::utils::filesystem::read_prefix;

/// Abstracts workspace reads and direct writes for unit tests.
pub(super) trait FileSystem: Send + Sync {
    /// Resolves an existing path while preserving filesystem error details.
    fn canonicalize(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathBuf>>;
    /// Checks whether the resolved path is an ordinary regular file.
    fn metadata(&self, path: PathBuf) -> BoxFuture<'static, io::Result<bool>>;
    /// Reads the current file bytes for both matching and stale-content checks.
    fn read(&self, path: PathBuf) -> BoxFuture<'static, io::Result<Vec<u8>>>;
    /// Replaces the existing file contents directly; a failed write may leave partial contents.
    fn write(&self, path: PathBuf, contents: Vec<u8>) -> BoxFuture<'static, io::Result<()>>;
}

/// Production filesystem adapter using Tokio.
pub(super) struct TokioFileSystem;

impl FileSystem for TokioFileSystem {
    fn canonicalize(&self, path: PathBuf) -> BoxFuture<'static, io::Result<PathBuf>> {
        Box::pin(tokio::fs::canonicalize(path))
    }

    fn metadata(&self, path: PathBuf) -> BoxFuture<'static, io::Result<bool>> {
        Box::pin(async move {
            tokio::fs::metadata(path)
                .await
                .map(|metadata| metadata.is_file())
        })
    }

    fn read(&self, path: PathBuf) -> BoxFuture<'static, io::Result<Vec<u8>>> {
        Box::pin(async move { Ok(read_prefix(&path, MAX_FILE_BYTES + 1).await?.bytes) })
    }

    fn write(&self, path: PathBuf, contents: Vec<u8>) -> BoxFuture<'static, io::Result<()>> {
        Box::pin(tokio::fs::write(path, contents))
    }
}
