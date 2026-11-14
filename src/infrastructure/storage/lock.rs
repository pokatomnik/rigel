use std::path::Path;

/// A guard that releases exclusive history ownership when dropped.
pub(crate) trait HistoryLockGuard: Send + Sync {}

/// Acquires exclusive interprocess ownership of one history directory.
pub(crate) trait HistoryLock: Send + Sync {
    /// Acquires the lock or returns an actionable error when another process owns it.
    fn acquire(&self, path: &Path) -> anyhow::Result<Box<dyn HistoryLockGuard>>;
}
