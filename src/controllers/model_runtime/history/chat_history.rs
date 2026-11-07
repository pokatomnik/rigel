use std::{
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::Mutex;

use rig::message::Message;

use crate::{
    entities::conversation::{
        conversation_message::ConversationMessage, turn::HistoryUpdate as DomainHistoryUpdate,
    },
    infrastructure::conversation::ConversationHistory as ConversationHistoryContract,
};

use super::{
    history_persistence::HistoryPersistence,
    message_codec::{from_provider, to_provider},
};

/// Owns an isolated in-memory history for a non-persistent subagent session.
pub(crate) struct InMemoryHistory {
    messages: Mutex<Vec<ConversationMessage>>,
    compaction_in_progress: AtomicBool,
}

impl InMemoryHistory {
    /// Creates an isolated history with the supplied initial snapshot.
    pub(crate) fn new(messages: Vec<ConversationMessage>) -> Self {
        Self {
            messages: Mutex::new(messages),
            compaction_in_progress: AtomicBool::new(false),
        }
    }
}

impl ConversationHistoryContract for InMemoryHistory {
    fn snapshot(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<ConversationMessage>> + Send + '_>>
    {
        Box::pin(async { self.messages.lock().await.clone() })
    }

    fn update(
        &self,
        update: DomainHistoryUpdate,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut messages = self.messages.lock().await;
            match update {
                DomainHistoryUpdate::Append(message) => messages.push(message),
                DomainHistoryUpdate::Replace(replacement) => *messages = replacement,
            }
            Ok(())
        })
    }

    fn begin_compaction(&self) {
        self.compaction_in_progress.store(true, Ordering::SeqCst);
    }

    fn end_compaction(&self) {
        self.compaction_in_progress.store(false, Ordering::SeqCst);
    }

    fn is_compaction_in_progress(&self) -> bool {
        self.compaction_in_progress.load(Ordering::SeqCst)
    }
}

/// Provider-facing history view over one provider-neutral conversation owner.
pub(crate) struct ChatHistory<P>
where
    P: HistoryPersistence,
{
    history: Arc<dyn ConversationHistoryContract>,
    persistence_type: PhantomData<P>,
}

/// A provider-only history mutation used by Rig hooks and streaming translation.
pub(crate) enum HistoryUpdate {
    /// Adds one provider message after a tool or prompt event.
    Append(Message),
    /// Replaces the complete provider message sequence.
    Replace(Vec<Message>),
}

impl<P> ChatHistory<P>
where
    P: HistoryPersistence,
{
    /// Creates a provider view over an already composed domain history.
    pub(crate) fn new(history: Arc<dyn ConversationHistoryContract>) -> Self {
        Self {
            history,
            persistence_type: PhantomData,
        }
    }

    pub(crate) fn is_compaction_in_progress(&self) -> bool {
        self.history.is_compaction_in_progress()
    }

    pub(crate) async fn snapshot_result(&self) -> anyhow::Result<Vec<Message>> {
        to_provider(&self.history.snapshot().await)
    }

    #[cfg(test)]
    pub(crate) async fn snapshot(&self) -> Vec<Message> {
        self.snapshot_result().await.unwrap_or_default()
    }

    pub(crate) async fn update(&self, update: HistoryUpdate) -> anyhow::Result<()> {
        let domain_update = match update {
            HistoryUpdate::Append(message) => {
                let mut messages = from_provider(std::slice::from_ref(&message))?;
                match messages.pop() {
                    Some(message) => DomainHistoryUpdate::Append(message),
                    None => return Ok(()),
                }
            }
            HistoryUpdate::Replace(messages) => {
                DomainHistoryUpdate::Replace(from_provider(&messages)?)
            }
        };
        self.history.update(domain_update).await
    }
}

#[cfg(test)]
struct ProviderPersistence<P> {
    persistence: Arc<P>,
}

#[cfg(test)]
impl<P> crate::infrastructure::conversation::ConversationPersistence for ProviderPersistence<P>
where
    P: HistoryPersistence,
{
    fn save(
        &self,
        messages: &[ConversationMessage],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let provider_messages = match to_provider(messages) {
            Ok(messages) => messages,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let persistence = self.persistence.clone();
        Box::pin(async move { persistence.save(provider_messages.as_slice()).await })
    }
}

#[cfg(test)]
impl<P> ChatHistory<P>
where
    P: HistoryPersistence,
{
    pub(crate) fn from_provider(messages: Vec<Message>, persistence: P) -> anyhow::Result<Self> {
        let domain_messages = from_provider(messages.as_slice())?;
        let domain =
            crate::use_cases::chat::history::conversation_history::ConversationHistory::new(
                domain_messages,
                ProviderPersistence {
                    persistence: Arc::new(persistence),
                },
            );
        Ok(Self::new(Arc::new(domain)))
    }
}

impl<P> ConversationHistoryContract for ChatHistory<P>
where
    P: HistoryPersistence,
{
    fn snapshot(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<ConversationMessage>> + Send + '_>>
    {
        self.history.snapshot()
    }

    fn update(
        &self,
        update: DomainHistoryUpdate,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.history.update(update)
    }

    fn begin_compaction(&self) {
        self.history.begin_compaction();
    }

    fn end_compaction(&self) {
        self.history.end_compaction();
    }

    fn is_compaction_in_progress(&self) -> bool {
        self.history.is_compaction_in_progress()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, ensure};
    use rig::message::Message;
    use tokio::sync::Mutex;

    use super::{ChatHistory, HistoryUpdate};
    use crate::{
        controllers::model_runtime::history::history_persistence::NonPersistentHistory,
        entities::conversation::{
            conversation_message::ConversationMessage, turn::HistoryUpdate as DomainHistoryUpdate,
        },
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
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            let messages = messages.to_vec();
            Box::pin(async move {
                self.snapshots.lock().await.push(messages);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn provider_history_preserves_provider_boundaries() -> Result<()> {
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let domain = Arc::new(
            crate::use_cases::chat::history::conversation_history::ConversationHistory::new(
                vec![ConversationMessage::User("earlier".to_string())],
                RecordingPersistence { snapshots },
            ),
        );
        let history =
            ChatHistory::<super::super::history_persistence::NonPersistentHistory>::new(domain);
        history
            .update(HistoryUpdate::Append(Message::user("new prompt")))
            .await?;
        let stored = history.snapshot_result().await?;
        ensure!(
            serde_json::to_value(&stored)?
                == serde_json::json!([{"role": "user", "content": [{"type": "text", "text": "earlier"}]} , {"role": "user", "content": [{"type": "text", "text": "new prompt"}]}])
        );
        Ok(())
    }

    #[derive(Clone, Copy)]
    struct FailingPersistence;

    impl ConversationPersistence for FailingPersistence {
        fn save(
            &self,
            _messages: &[ConversationMessage],
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            Box::pin(async { anyhow::bail!("write failed") })
        }
    }

    #[tokio::test]
    async fn update_persists_before_committing_in_memory() -> Result<()> {
        let snapshots = Arc::new(Mutex::new(Vec::new()));
        let domain = Arc::new(
            crate::use_cases::chat::history::conversation_history::ConversationHistory::new(
                vec![ConversationMessage::User("earlier".to_string())],
                RecordingPersistence {
                    snapshots: snapshots.clone(),
                },
            ),
        );
        let history = ChatHistory::<NonPersistentHistory>::new(domain);
        history
            .update(HistoryUpdate::Append(Message::user("new prompt")))
            .await?;
        let stored = history.snapshot_result().await?;
        ensure!(
            serde_json::to_value(&stored)?
                == serde_json::json!([{"role": "user", "content": [{"type": "text", "text": "earlier"}]} , {"role": "user", "content": [{"type": "text", "text": "new prompt"}]}])
        );
        ensure!(snapshots.lock().await.len() == 1);
        Ok(())
    }

    #[tokio::test]
    async fn failed_persistence_keeps_previous_in_memory_history() -> Result<()> {
        let domain = Arc::new(
            crate::use_cases::chat::history::conversation_history::ConversationHistory::new(
                vec![ConversationMessage::User("earlier".to_string())],
                FailingPersistence,
            ),
        );
        let history = ChatHistory::<NonPersistentHistory>::new(domain);
        ensure!(
            history
                .update(HistoryUpdate::Append(Message::user("not persisted")))
                .await
                .is_err()
        );
        ensure!(
            serde_json::to_value(history.snapshot_result().await?)?
                == serde_json::json!([{"role": "user", "content": [{"type": "text", "text": "earlier"}]}])
        );
        Ok(())
    }

    #[tokio::test]
    async fn non_persistent_history_keeps_updates_in_memory_without_writing() -> Result<()> {
        let history = ChatHistory::<NonPersistentHistory>::new(Arc::new(
            super::InMemoryHistory::new(Vec::new()),
        ));
        history
            .update(HistoryUpdate::Append(Message::user("subagent task")))
            .await?;
        ensure!(
            serde_json::to_value(history.snapshot_result().await?)?
                == serde_json::json!([{"role": "user", "content": [{"type": "text", "text": "subagent task"}]}])
        );
        Ok(())
    }

    #[allow(dead_code)]
    fn domain_update(_: DomainHistoryUpdate) {}
}
