use anyhow::Context;
use tokio::fs;

use crate::{
    controllers::storage::{
        history::JsonlHistory, lock::FileHistoryLock, paths::HistoryPaths, reader::HistoryReader,
    },
    entities::{
        conversation::conversation_message::ConversationMessage, transcript::entry::TranscriptEntry,
    },
    infrastructure::storage::lock::{HistoryLock, HistoryLockGuard},
};

/// Loaded history, its durable writer, and the guard that owns the directory.
pub(crate) struct BootstrappedHistory {
    /// Append-only persistence used by the domain history owner.
    pub(crate) history: JsonlHistory,
    /// Model context from the active segment.
    pub(crate) messages: Vec<ConversationMessage>,
    /// Effective system prompt stored for this history, if any.
    pub(crate) system_prompt: Option<String>,
    /// Unreadable lines skipped while loading.
    pub(crate) skipped_lines: usize,
    /// Transcript entries projected for the first UI frame.
    pub(crate) entries: Vec<TranscriptEntry>,
    /// Exclusive directory ownership held for the lifetime of the chat.
    pub(crate) lock_guard: Box<dyn HistoryLockGuard>,
}

/// Resolves the current directory history, acquires its lock, and loads it.
pub(crate) async fn bootstrap() -> anyhow::Result<BootstrappedHistory> {
    bootstrap_at(HistoryPaths::from_current_dir()?).await
}

/// Loads one explicit history family; used by bootstrap and integration tests.
pub(crate) async fn bootstrap_at(paths: HistoryPaths) -> anyhow::Result<BootstrappedHistory> {
    fs::create_dir_all(paths.directory())
        .await
        .with_context(|| {
            format!(
                "failed to create history directory '{}'",
                paths.directory().display()
            )
        })?;
    let lock_guard = FileHistoryLock.acquire(paths.lock().as_path())?;
    let stale_temp = paths.directory().join("chat.tmp");
    match fs::remove_file(stale_temp.as_path()).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to remove stale '{}'", stale_temp.display()));
        }
    }
    let loaded = HistoryReader::new(paths.clone()).load().await?;
    let history = JsonlHistory::new(&paths, &loaded);
    Ok(BootstrappedHistory {
        history,
        messages: loaded.context,
        system_prompt: loaded.system_prompt,
        skipped_lines: loaded.skipped_lines,
        entries: loaded.entries,
        lock_guard,
    })
}
