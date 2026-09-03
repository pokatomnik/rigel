use std::sync::Arc;

use rig::{
    agent::{
        AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, InvalidToolCallAction,
        InvalidToolCallContext, ModelTurnAction, ModelTurnFinished, ObservationAction,
        StreamResponseFinish,
    },
    message::Message,
};

use super::{
    chat_history::{ChatHistory, HistoryUpdate},
    history_persistence::HistoryPersistence,
};

pub(crate) const HISTORY_SYNC_ERROR_PREFIX: &str = "chat history persistence failed";

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
        if self.history.is_compaction_in_progress() {
            return CompletionCallAction::continue_run();
        }
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
        if self.history.is_compaction_in_progress() {
            return ModelTurnAction::continue_run();
        }
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
        if self.history.is_compaction_in_progress() {
            return None;
        }
        let update = HistoryUpdate::Replace(event.chat_history.clone());
        match self.history.update(update).await {
            Ok(()) => None,
            Err(error) => Some(InvalidToolCallAction::stop(format!(
                "{HISTORY_SYNC_ERROR_PREFIX}: {error:#}"
            ))),
        }
    }
}
