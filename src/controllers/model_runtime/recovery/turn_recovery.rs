use std::sync::Arc;

use futures::future::BoxFuture;
use rig::completion::CompletionModel;

use crate::{
    controllers::model_runtime::{
        history::history_persistence::HistoryPersistence,
        streaming::streamed_turn::{StreamRunOutcome, StreamedTurn},
    },
    entities::{
        conversation::{context_usage::ContextUsage, turn::TurnStatus},
        recovery::{RecoveryOutcome, RecoveryRequest},
    },
    infrastructure::recovery::{RecoveryTurn, TurnRecovery},
};

/// Adapts a durable Rig stream to the provider-neutral turn recovery workflow.
pub(crate) struct TurnRecoverer<'a, CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    streamed_turn: StreamedTurn<'a, CM, P>,
    workflow: Arc<dyn TurnRecovery>,
}

impl<'a, CM, P> TurnRecoverer<'a, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    /// Creates the provider bridge with the stream runner and injected recovery workflow.
    pub(crate) fn new(
        streamed_turn: StreamedTurn<'a, CM, P>,
        workflow: Arc<dyn TurnRecovery>,
    ) -> Self {
        Self {
            streamed_turn,
            workflow,
        }
    }

    /// Delegates recovery policy while preserving this session's durable stream history.
    pub(crate) async fn recover(
        &self,
        request: RecoveryRequest,
        context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        self.workflow.recover(request, self, context_usage).await
    }
}

impl<CM, P> RecoveryTurn for TurnRecoverer<'_, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    fn run_prompt(&self, prompt: String) -> BoxFuture<'_, anyhow::Result<RecoveryOutcome>> {
        Box::pin(
            async move { self.map_stream_outcome(self.streamed_turn.run_latest(prompt).await?) },
        )
    }
}

impl<CM, P> TurnRecoverer<'_, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    fn map_stream_outcome(&self, outcome: StreamRunOutcome) -> anyhow::Result<RecoveryOutcome> {
        match outcome {
            StreamRunOutcome::Completed(completion) => Ok(RecoveryOutcome::Completed {
                output: completion.output().to_string(),
                has_output: completion.has_output(),
                context_usage: completion.context_usage(),
            }),
            StreamRunOutcome::Failed(failure) => Ok(RecoveryOutcome::Failed {
                error: failure.error().to_string(),
                context_usage: failure.context_usage(),
            }),
        }
    }
}
