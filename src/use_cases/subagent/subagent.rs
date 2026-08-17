#![allow(dead_code)]

use std::sync::Arc;

use rig::{Agent, completion::CompletionModel};
use tokio::sync::Mutex;

use crate::{
    prompts::subagent::task_prompt,
    shared::{
        history::{ChatHistory, NonPersistentHistory},
        recovery::{RecoveryRequest, TurnRecoverer, TurnStatus},
        streaming::{StreamRunOutcome, StreamedTurn},
        terminal::TerminalIO,
    },
};

/// Runs one autonomous task and returns the model's structured work report.
pub(crate) struct Subagent<CM>
where
    CM: CompletionModel,
{
    agent: Arc<Mutex<Agent<CM>>>,
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<NonPersistentHistory>>,
}

impl<CM> Subagent<CM>
where
    CM: CompletionModel + 'static,
{
    pub(crate) fn new(agent: Agent<CM>, terminal_io: Arc<TerminalIO>) -> Self {
        let history = Arc::new(ChatHistory::new(Vec::new(), NonPersistentHistory));
        Self::with_history(agent, terminal_io, history)
    }

    pub(crate) fn with_history(
        agent: Agent<CM>,
        terminal_io: Arc<TerminalIO>,
        history: Arc<ChatHistory<NonPersistentHistory>>,
    ) -> Self {
        Self {
            agent: Arc::new(Mutex::new(agent)),
            terminal_io,
            history,
        }
    }

    fn streamed_turn(&self) -> StreamedTurn<'_, CM, NonPersistentHistory> {
        StreamedTurn::new(
            self.agent.clone(),
            self.terminal_io.as_ref(),
            self.history.as_ref(),
        )
    }

    /// Executes the supplied task without reading from the terminal.
    pub(crate) async fn run(&self, task: String) -> anyhow::Result<String> {
        let prompt = task_prompt(task);
        let base = self.history.snapshot().await;
        let streamed_turn = self.streamed_turn();
        let outcome = streamed_turn.run(prompt, base).await?;
        self.complete_turn(streamed_turn, outcome).await
    }

    async fn complete_turn(
        &self,
        streamed_turn: StreamedTurn<'_, CM, NonPersistentHistory>,
        outcome: StreamRunOutcome,
    ) -> anyhow::Result<String> {
        let request = self.recovery_request(&outcome);
        let Some(request) = request else {
            return self.completed_report(outcome);
        };

        match TurnRecoverer::new(streamed_turn).recover(request).await? {
            TurnStatus::Complete(report) => Ok(report),
            TurnStatus::RecoveryStopped(error) => {
                anyhow::bail!("subagent recovery stopped: {error}")
            }
        }
    }

    fn recovery_request(&self, outcome: &StreamRunOutcome) -> Option<RecoveryRequest> {
        match outcome {
            StreamRunOutcome::Failed(error) => Some(RecoveryRequest::StreamFailure(error.clone())),
            StreamRunOutcome::Completed(completion) if completion.requires_tool_recovery() => {
                Some(RecoveryRequest::UnresolvedTool)
            }
            StreamRunOutcome::Completed(completion) if completion.requires_answer_recovery() => {
                Some(RecoveryRequest::MissingAnswer)
            }
            StreamRunOutcome::Completed(completion) if !completion.has_output() => {
                Some(RecoveryRequest::MissingAnswer)
            }
            StreamRunOutcome::Completed(_) => None,
        }
    }

    fn completed_report(&self, outcome: StreamRunOutcome) -> anyhow::Result<String> {
        match outcome {
            StreamRunOutcome::Completed(completion) => Ok(completion.output().to_string()),
            StreamRunOutcome::Failed(error) => anyhow::bail!("subagent failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, ensure};
    use rig::{
        AgentBuilder,
        message::Message,
        test_utils::{MockCompletionModel, MockStreamEvent},
    };

    use super::Subagent;
    use crate::{prompts::subagent::subagent_prompt, shared::terminal::TerminalIO};

    #[tokio::test]
    async fn run_returns_the_model_report_and_keeps_task_in_memory() -> Result<()> {
        let report = "SUBAGENT REPORT\nSTATUS: COMPLETED\n\nSUMMARY:\nDone.";
        let model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::text(report),
            MockStreamEvent::final_response_with_total_tokens(1),
        ]]);
        let agent = AgentBuilder::new(model).build();
        let subagent = Subagent::new(agent, Arc::new(TerminalIO));

        ensure!(subagent.run("inspect the repository".to_string()).await? == report);
        let messages = subagent.history.snapshot().await;
        ensure!(messages.iter().any(|message| {
            matches!(message, Message::User { content } if content.iter().any(|item| {
                matches!(item, rig::message::UserContent::Text(text)
                    if text.text.contains("inspect the repository")
                        && text.text.contains(subagent_prompt()))
            }))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn run_recovers_a_missing_answer_without_reading_input() -> Result<()> {
        let report = "SUBAGENT REPORT\nSTATUS: COMPLETED\n\nSUMMARY:\nRecovered.";
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::reasoning("working"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::text(report),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
        ]);
        let agent = AgentBuilder::new(model).build();
        let subagent = Subagent::new(agent, Arc::new(TerminalIO));

        ensure!(subagent.run("finish autonomously".to_string()).await? == report);
        Ok(())
    }
}
