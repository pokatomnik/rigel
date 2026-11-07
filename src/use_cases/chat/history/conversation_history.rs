use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
};

use tokio::sync::Mutex;

use crate::entities::conversation::{
    conversation_message::ConversationMessage, turn::HistoryUpdate,
};

/// Keeps an in-memory conversation snapshot consistent with a persistence contract.
pub(crate) struct ConversationHistory<P> {
    messages: Mutex<Vec<ConversationMessage>>,
    persistence: P,
    compaction_in_progress: AtomicBool,
}

impl<P> ConversationHistory<P>
where
    P: crate::infrastructure::conversation::ConversationPersistence,
{
    /// Creates history from an already loaded snapshot and persistence adapter.
    pub(crate) fn new(messages: Vec<ConversationMessage>, persistence: P) -> Self {
        Self {
            messages: Mutex::new(messages),
            persistence,
            compaction_in_progress: AtomicBool::new(false),
        }
    }

    /// Marks model callbacks as compaction-internal so they do not duplicate history.
    pub(crate) fn begin_compaction(&self) {
        self.compaction_in_progress.store(true, Ordering::SeqCst);
    }

    /// Re-enables normal history callback synchronization.
    pub(crate) fn end_compaction(&self) {
        self.compaction_in_progress.store(false, Ordering::SeqCst);
    }

    /// Reports whether a compaction operation currently owns the history boundary.
    pub(crate) fn is_compaction_in_progress(&self) -> bool {
        self.compaction_in_progress.load(Ordering::SeqCst)
    }

    /// Takes a consistent copy for a model request.
    pub(crate) async fn snapshot_messages(&self) -> Vec<ConversationMessage> {
        self.messages.lock().await.clone()
    }

    /// Persists a candidate before committing it to memory.
    pub(crate) async fn update_messages(&self, update: HistoryUpdate) -> anyhow::Result<()> {
        let mut messages = self.messages.lock().await;
        let candidate = apply_update(update, messages.as_slice());
        if *messages == candidate {
            return Ok(());
        }
        self.persistence.save(&candidate).await?;
        *messages = candidate;
        Ok(())
    }
}

impl<P> crate::infrastructure::conversation::ConversationHistory for ConversationHistory<P>
where
    P: crate::infrastructure::conversation::ConversationPersistence,
{
    fn snapshot(&self) -> Pin<Box<dyn Future<Output = Vec<ConversationMessage>> + Send + '_>> {
        Box::pin(async move { self.snapshot_messages().await })
    }

    fn update(
        &self,
        update: HistoryUpdate,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move { self.update_messages(update).await })
    }

    fn begin_compaction(&self) {
        self.begin_compaction();
    }

    fn end_compaction(&self) {
        self.end_compaction();
    }

    fn is_compaction_in_progress(&self) -> bool {
        self.is_compaction_in_progress()
    }
}

fn apply_update(
    update: HistoryUpdate,
    current: &[ConversationMessage],
) -> Vec<ConversationMessage> {
    match update {
        HistoryUpdate::Append(message) => {
            let mut messages = current.to_vec();
            messages.push(message);
            messages
        }
        HistoryUpdate::Replace(messages) => messages,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, ensure};
    use tokio::sync::Mutex;

    use super::ConversationHistory;
    use crate::{
        entities::conversation::{conversation_message::ConversationMessage, turn::HistoryUpdate},
        infrastructure::conversation::ConversationPersistence,
    };

    #[derive(Clone)]
    struct RecordingPersistence {
        snapshots: Arc<Mutex<Vec<Vec<ConversationMessage>>>>,
    }

    impl ConversationPersistence for RecordingPersistence {
        fn save(
            &self,
            messages: &[ConversationMessage],
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
            let messages = messages.to_vec();
            Box::pin(async move {
                self.snapshots.lock().await.push(messages);
                Ok(())
            })
        }
    }

    struct FailingPersistence;

    impl ConversationPersistence for FailingPersistence {
        fn save(
            &self,
            _messages: &[ConversationMessage],
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
            Box::pin(async { anyhow::bail!("write failed") })
        }
    }

    #[tokio::test]
    async fn persistence_commits_before_memory_changes() -> Result<()> {
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let history = ConversationHistory::new(
            vec![ConversationMessage::User("before".to_string())],
            RecordingPersistence {
                snapshots: snapshots.clone(),
            },
        );

        history
            .update_messages(HistoryUpdate::Append(ConversationMessage::Assistant(
                "after".to_string(),
            )))
            .await?;

        let stored = history.snapshot_messages().await;
        ensure!(snapshots.lock().await.as_slice() == std::slice::from_ref(&stored));
        ensure!(stored.len() == 2);
        Ok(())
    }

    #[tokio::test]
    async fn failed_persistence_preserves_the_previous_snapshot() -> Result<()> {
        let history = ConversationHistory::new(
            vec![ConversationMessage::User("before".to_string())],
            FailingPersistence,
        );

        ensure!(
            history
                .update_messages(HistoryUpdate::Append(ConversationMessage::Assistant(
                    "after".to_string(),
                )))
                .await
                .is_err()
        );
        ensure!(
            history.snapshot_messages().await == [ConversationMessage::User("before".to_string())]
        );
        Ok(())
    }
}
