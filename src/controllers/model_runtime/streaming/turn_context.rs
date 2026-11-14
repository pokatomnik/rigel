use std::sync::Arc;

use crate::{
    controllers::model_runtime::{
        history::{chat_history::ChatHistory, history_persistence::HistoryPersistence},
        streaming::streamed_turn::StreamRunOutcome,
    },
    entities::{
        conversation::{context_overflow::is_context_overflow, context_usage::ContextUsage},
        selected_model::selection::SelectedModel,
    },
    infrastructure::ui::{cancellation::RunCancellation, gateway::EventSink},
};

/// Inputs and live-rendering dependencies for one streamed turn.
pub(crate) struct StreamedTurnContext<'a, P>
where
    P: HistoryPersistence,
{
    /// Provider history shared with the session.
    pub(crate) history: &'a ChatHistory<P>,
    /// Usage observed before the turn.
    pub(crate) context_usage: ContextUsage,
    /// Selected model inherited by delegated tools.
    pub(crate) selected_model: Option<SelectedModel>,
    /// Live event sink for deltas and transcript rows.
    pub(crate) events: Option<Arc<dyn EventSink>>,
    /// Soft-stop flag checked between stream items.
    pub(crate) cancellation: Option<Arc<dyn RunCancellation>>,
    /// Sequence number carried by live deltas.
    pub(crate) turn_id: u64,
}

impl<'a, P> StreamedTurnContext<'a, P>
where
    P: HistoryPersistence,
{
    /// Creates a context without model metadata or live rendering.
    pub(crate) fn new(history: &'a ChatHistory<P>, context_usage: ContextUsage) -> Self {
        Self {
            history,
            context_usage,
            selected_model: None,
            events: None,
            cancellation: None,
            turn_id: 0,
        }
    }

    /// Adds the selected model metadata for tools that create delegated sessions.
    pub(crate) fn with_selected_model(mut self, model: SelectedModel) -> Self {
        self.selected_model = Some(model);
        self
    }

    /// Adds the event sink used for live deltas and transcript rows.
    pub(crate) fn with_events(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = Some(events);
        self
    }

    /// Adds the soft-stop flag checked between stream items.
    pub(crate) fn with_cancellation(mut self, cancellation: Arc<dyn RunCancellation>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Sets the sequence number carried by live deltas.
    pub(crate) fn with_turn_id(mut self, turn_id: u64) -> Self {
        self.turn_id = turn_id;
        self
    }
}

/// Returns the provider error text when a failed turn was caused by context overflow.
pub(crate) fn overflow_error(outcome: &StreamRunOutcome) -> Option<&str> {
    match outcome {
        StreamRunOutcome::Failed(failure) if is_context_overflow(failure.error()) => {
            Some(failure.error())
        }
        StreamRunOutcome::Failed(_)
        | StreamRunOutcome::Completed(_)
        | StreamRunOutcome::Cancelled { .. } => None,
    }
}
