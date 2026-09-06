use rig::completion::CompletionModel;

use crate::shared::{
    agent::dependencies::ConfiguredAgent,
    recovery::turn_recovery::{RecoveryRequest, TurnRecoverer, TurnStatus},
    streaming::streamed_turn::{StreamRunOutcome, StreamedTurn, StreamedTurnContext},
};

use super::chat::Chat;

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: crate::shared::history::history_persistence::HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<ConfiguredAgent<CM>> + 'static,
{
    pub(super) async fn streamed_turn(&self) -> StreamedTurn<'_, CM, P> {
        let context_usage = *self.context_usage.lock().await;
        StreamedTurn::with_context(
            self.agent.clone(),
            self.terminal_io.as_ref(),
            StreamedTurnContext::new(self.history.as_ref(), context_usage),
        )
    }

    pub(super) async fn run_prompt(
        &self,
        prompt: String,
        echo: bool,
    ) -> anyhow::Result<Option<crate::shared::goal::goal_state::Report>> {
        self.echo_prompt(prompt.as_str(), echo);
        let base = self.history.snapshot().await;
        let streamed_turn = self.streamed_turn().await;
        let outcome = streamed_turn.run(prompt, base).await?;
        let status = self.complete_user_turn(streamed_turn, outcome).await?;
        self.update_context_usage(status.context_usage()).await;
        self.compact_after_turn(&status).await?;
        self.report_turn_status(&status);
        self.terminal_io.eprintln("");
        Ok(self.take_goal_report())
    }

    async fn complete_user_turn(
        &self,
        streamed_turn: StreamedTurn<'_, CM, P>,
        outcome: StreamRunOutcome,
    ) -> anyhow::Result<TurnStatus> {
        let context_usage = match &outcome {
            StreamRunOutcome::Completed(completion) => completion.context_usage(),
            StreamRunOutcome::Failed(failure) => failure.context_usage(),
        };
        let request = match outcome {
            StreamRunOutcome::Failed(failure) => {
                RecoveryRequest::StreamFailure(failure.error().to_string())
            }
            StreamRunOutcome::Completed(completion) if completion.requires_tool_recovery() => {
                RecoveryRequest::UnresolvedTool
            }
            StreamRunOutcome::Completed(completion) if completion.requires_answer_recovery() => {
                RecoveryRequest::MissingAnswer
            }
            StreamRunOutcome::Completed(completion) => {
                return Ok(TurnStatus::Complete {
                    output: completion.output().to_string(),
                    context_usage: completion.context_usage(),
                });
            }
        };

        TurnRecoverer::new(streamed_turn)
            .recover(request, context_usage)
            .await
    }
}
