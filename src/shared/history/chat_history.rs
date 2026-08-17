use rig::message::Message;
use tokio::sync::Mutex;

use super::history_persistence::HistoryPersistence;

pub(crate) enum HistoryUpdate {
    Append(Message),
    Replace(Vec<Message>),
}

impl HistoryUpdate {
    fn apply(self, current: &[Message]) -> Vec<Message> {
        match self {
            Self::Append(message) => {
                let mut messages = current.to_vec();
                messages.push(message);
                messages
            }
            Self::Replace(messages) => messages,
        }
    }
}

/// Keeps persisted and in-memory history on the same committed snapshot.
pub(crate) struct ChatHistory<P>
where
    P: HistoryPersistence,
{
    messages: Mutex<Vec<Message>>,
    persistence: P,
}

impl<P> ChatHistory<P>
where
    P: HistoryPersistence,
{
    pub(crate) fn new(messages: Vec<Message>, persistence: P) -> Self {
        Self {
            messages: Mutex::new(messages),
            persistence,
        }
    }

    pub(crate) async fn snapshot(&self) -> Vec<Message> {
        self.messages.lock().await.clone()
    }

    pub(crate) async fn update(&self, update: HistoryUpdate) -> anyhow::Result<()> {
        let mut messages = self.messages.lock().await;
        let candidate = update.apply(messages.as_slice());
        if *messages == candidate {
            return Ok(());
        }

        self.persistence.save(&candidate).await?;
        *messages = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, ensure};
    use rig::message::Message;
    use tokio::sync::Mutex;

    use super::{ChatHistory, HistoryUpdate};
    use crate::shared::history::history_persistence::{HistoryPersistence, NonPersistentHistory};

    #[derive(Clone)]
    struct RecordingPersistence {
        snapshots: Arc<Mutex<Vec<Vec<Message>>>>,
    }

    impl HistoryPersistence for RecordingPersistence {
        fn save<'a>(
            &'a self,
            messages: &'a [Message],
        ) -> futures::future::BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                self.snapshots.lock().await.push(messages.to_vec());
                Ok(())
            })
        }
    }

    #[derive(Clone, Copy)]
    struct FailingPersistence;

    impl HistoryPersistence for FailingPersistence {
        fn save<'a>(
            &'a self,
            _messages: &'a [Message],
        ) -> futures::future::BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async { anyhow::bail!("write failed") })
        }
    }

    #[tokio::test]
    async fn update_persists_before_committing_in_memory() -> Result<()> {
        let snapshots = Arc::new(Mutex::new(Vec::<Vec<Message>>::new()));
        let persistence = RecordingPersistence {
            snapshots: snapshots.clone(),
        };
        let history = ChatHistory::new(vec![Message::user("earlier")], persistence);

        history
            .update(HistoryUpdate::Append(Message::user("new prompt")))
            .await?;

        let stored = history.snapshot().await;
        let observed = snapshots.lock().await;
        ensure!(observed.first() == Some(&stored));
        ensure!(observed.len() == 1, "history was persisted more than once");
        ensure!(stored == [Message::user("earlier"), Message::user("new prompt")]);
        Ok(())
    }

    #[tokio::test]
    async fn failed_persistence_keeps_previous_in_memory_history() -> Result<()> {
        let history = ChatHistory::new(vec![Message::user("earlier")], FailingPersistence);
        let result = history
            .update(HistoryUpdate::Append(Message::user("not persisted")))
            .await;

        ensure!(result.is_err(), "failed persistence was accepted");
        ensure!(history.snapshot().await == [Message::user("earlier")]);
        Ok(())
    }

    #[tokio::test]
    async fn non_persistent_history_keeps_updates_in_memory_without_writing() -> Result<()> {
        let history = ChatHistory::new(Vec::new(), NonPersistentHistory);

        history
            .update(HistoryUpdate::Append(Message::user("subagent task")))
            .await?;

        ensure!(history.snapshot().await == [Message::user("subagent task")]);
        Ok(())
    }
}
