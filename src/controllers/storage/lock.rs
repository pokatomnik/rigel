use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
};

use anyhow::Context;

use crate::infrastructure::storage::lock::{HistoryLock, HistoryLockGuard};

/// Advisory file lock that keeps a second process from writing the same history.
pub(crate) struct FileHistoryLock;

impl HistoryLock for FileHistoryLock {
    fn acquire(&self, path: &Path) -> anyhow::Result<Box<dyn HistoryLockGuard>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("failed to open history lock '{}'", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Box::new(FileLockGuard { file })),
            Err(TryLockError::WouldBlock) => anyhow::bail!(
                "another Rigel chat already owns the history in '{}'",
                path.parent().unwrap_or(path).display()
            ),
            Err(TryLockError::Error(error)) => {
                Err(error).with_context(|| format!("failed to lock history '{}'", path.display()))
            }
        }
    }
}

struct FileLockGuard {
    file: File,
}

impl HistoryLockGuard for FileLockGuard {}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
