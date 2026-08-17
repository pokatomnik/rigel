use std::sync::Arc;

use futures::future::BoxFuture;
use rig::{
    agent::{
        AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, InvalidToolCallAction,
        InvalidToolCallContext, ModelTurnAction, ModelTurnFinished, ObservationAction,
        StreamResponseFinish,
    },
    message::Message,
};
use tokio::sync::Mutex;

use super::history::History;

pub(crate) const HISTORY_SYNC_ERROR_PREFIX: &str = "chat history persistence failed";

pub(crate) trait HistoryPersistence: Send + Sync + 'static {
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>>;
}

impl HistoryPersistence for History {
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let references = messages.iter().collect::<Vec<_>>();
            History::save(self, &references).await
        })
    }
}

impl<P> HistoryPersistence for Arc<P>
where
    P: HistoryPersistence,
{
    fn save<'a>(&'a self, messages: &'a [Message]) -> BoxFuture<'a, anyhow::Result<()>> {
        self.as_ref().save(messages)
    }
}

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

/// Persists canonical Rig boundaries hidden from ordinary stream items.
#[derive(Clone)]
pub(crate) struct HistorySyncHook<P>
where
    P: HistoryPersistence,
{
    history: Arc<ChatHistory<P>>,
}

impl<P> HistorySyncHook<P>
where
    P: HistoryPersistence,
{
    pub(crate) fn new(history: Arc<ChatHistory<P>>) -> Self {
        Self { history }
    }
}

#[derive(Clone, Default)]
struct PendingAssistantMessageId(Option<String>);

impl<P> AgentHook for HistorySyncHook<P>
where
    P: HistoryPersistence,
{
    async fn on_completion_call(
        &self,
        ctx: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        ctx.scratchpad()
            .update::<PendingAssistantMessageId, _>(|pending| pending.0 = None);
        let mut messages = event.history.to_vec();
        messages.push(event.prompt.clone());

        match self.history.update(HistoryUpdate::Replace(messages)).await {
            Ok(()) => CompletionCallAction::continue_run(),
            Err(error) => {
                CompletionCallAction::stop(format!("{HISTORY_SYNC_ERROR_PREFIX}: {error:#}"))
            }
        }
    }

    async fn on_stream_response_finish(
        &self,
        ctx: &HookContext,
        event: StreamResponseFinish<'_>,
    ) -> ObservationAction {
        let message_id = event.message_id.map(str::to_string);
        ctx.scratchpad()
            .update::<PendingAssistantMessageId, _>(|pending| pending.0 = message_id);
        ObservationAction::continue_run()
    }

    async fn on_model_turn_finished(
        &self,
        ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let message_id = ctx
            .scratchpad()
            .update::<PendingAssistantMessageId, _>(|pending| pending.0.take());
        let mut messages = self.history.snapshot().await;
        messages.push(Message::Assistant {
            id: message_id,
            content: event.content.clone(),
        });

        match self.history.update(HistoryUpdate::Replace(messages)).await {
            Ok(()) => ModelTurnAction::continue_run(),
            Err(error) => ModelTurnAction::stop(format!("{HISTORY_SYNC_ERROR_PREFIX}: {error:#}")),
        }
    }

    async fn on_invalid_tool_call(
        &self,
        _ctx: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        let update = HistoryUpdate::Replace(event.chat_history.clone());
        match self.history.update(update).await {
            Ok(()) => None,
            Err(error) => Some(InvalidToolCallAction::stop(format!(
                "{HISTORY_SYNC_ERROR_PREFIX}: {error:#}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, ensure};
    use rig::message::Message;
    use tokio::sync::Mutex;

    use super::{ChatHistory, HistoryPersistence, HistoryUpdate};

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
}
