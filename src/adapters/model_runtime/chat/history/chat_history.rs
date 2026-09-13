use std::{future::Future, marker::PhantomData, pin::Pin, sync::Arc};

use rig::message::Message;

use super::{
    conversation_history::ConversationHistory,
    history_persistence::HistoryPersistence,
    message_codec::{from_provider, to_provider},
};
use crate::{
    contracts::conversation::{
        ConversationHistory as ConversationHistoryContract, ConversationPersistence,
    },
    entities::conversation::turn::HistoryUpdate as DomainHistoryUpdate,
};

pub(crate) enum HistoryUpdate {
    Append(Message),
    Replace(Vec<Message>),
}

/// Bridges the provider message API to the transactional domain history owner.
pub(crate) struct ChatHistory<P>
where
    P: HistoryPersistence,
{
    history: Arc<dyn ConversationHistoryContract>,
    persistence_type: PhantomData<P>,
}

impl<P> ChatHistory<P>
where
    P: HistoryPersistence,
{
    pub(crate) fn new(messages: Vec<Message>, persistence: P) -> Self {
        let domain_messages = from_provider(&messages);
        Self {
            history: Arc::new(ConversationHistory::new(
                domain_messages,
                ProviderPersistence {
                    persistence: Arc::new(persistence),
                },
            )),
            persistence_type: PhantomData,
        }
    }

    pub(crate) fn begin_compaction(&self) {
        self.history.begin_compaction();
    }

    pub(crate) fn end_compaction(&self) {
        self.history.end_compaction();
    }

    pub(crate) fn is_compaction_in_progress(&self) -> bool {
        self.history.is_compaction_in_progress()
    }

    pub(crate) async fn snapshot(&self) -> Vec<Message> {
        to_provider(&self.history.snapshot().await)
    }

    pub(crate) async fn update(&self, update: HistoryUpdate) -> anyhow::Result<()> {
        let domain_update = match update {
            HistoryUpdate::Append(message) => self.append_update(message).await,
            HistoryUpdate::Replace(messages) => {
                DomainHistoryUpdate::Replace(from_provider(&messages))
            }
        };
        self.history.update(domain_update).await
    }

    async fn append_update(&self, message: Message) -> DomainHistoryUpdate {
        let messages = from_provider(std::slice::from_ref(&message));
        match messages.as_slice() {
            [message] => DomainHistoryUpdate::Append(message.clone()),
            _ => {
                let mut current = self.history.snapshot().await;
                current.extend(messages);
                DomainHistoryUpdate::Replace(current)
            }
        }
    }
}

struct ProviderPersistence<P> {
    persistence: Arc<P>,
}

impl<P> ConversationPersistence for ProviderPersistence<P>
where
    P: HistoryPersistence,
{
    fn save(
        &self,
        messages: &[crate::entities::conversation::conversation_message::ConversationMessage],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        let provider_messages = to_provider(messages);
        let persistence = self.persistence.clone();
        Box::pin(async move { persistence.save(provider_messages.as_slice()).await })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, ensure};
    use rig::message::Message;
    use tokio::sync::Mutex;

    use super::{ChatHistory, HistoryUpdate};
    use crate::adapters::model_runtime::chat::history::history_persistence::{
        HistoryPersistence, NonPersistentHistory,
    };

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
