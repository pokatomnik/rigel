#![allow(dead_code)]

use std::{future::Future, pin::Pin, sync::Arc};

use futures::future::BoxFuture;

use rig::{
    Agent, completion::CompletionModel as CompletionModelTrait, providers::openai::CompletionModel,
};
use tokio::sync::Mutex;

use crate::{
    adapters::model_runtime::chat::{
        compaction::context_compactor::ContextCompactor,
        history::{chat_history::ChatHistory, history_persistence::NonPersistentHistory},
        recovery::turn_recovery::{RecoveryRequest, TurnRecoverer},
        streaming::streamed_turn::{StreamRunOutcome, StreamedTurn, StreamedTurnContext},
    },
    adapters::model_runtime::prompt_provider::StaticPromptProvider,
    adapters::{
        model_runtime::agent::{
            agent::Agent as AgentFactory, agent_config::AgentConfig,
            dependencies::AgentDependencies,
        },
        terminal::terminal_io::TerminalIO,
    },
    contracts::prompts::PromptProvider,
    contracts::tools::subagent::SubagentRunner,
    entities::{
        conversation::{
            context_usage::ContextUsage,
            turn::{CompactionKind, TurnStatus},
        },
        tool::tool_entities::{SubagentReport, SubagentTask},
    },
};

/// Runs one autonomous task and returns the model's structured work report.
pub(crate) struct Subagent<CM>
where
    CM: CompletionModelTrait,
{
    agent: Arc<Mutex<Agent<CM>>>,
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<NonPersistentHistory>>,
    context_usage: Mutex<ContextUsage>,
}

pub(crate) struct SubagentContext {
    history: Arc<ChatHistory<NonPersistentHistory>>,
    max_context_tokens: Option<u64>,
}

impl SubagentContext {
    pub(crate) fn new(
        history: Arc<ChatHistory<NonPersistentHistory>>,
        max_context_tokens: Option<u64>,
    ) -> Self {
        Self {
            history,
            max_context_tokens,
        }
    }
}

impl<CM> Subagent<CM>
where
    CM: CompletionModelTrait + 'static,
{
    pub(crate) fn new(agent: Agent<CM>, terminal_io: Arc<TerminalIO>) -> Self {
        let history = Arc::new(ChatHistory::new(Vec::new(), NonPersistentHistory));
        Self::with_history_context(agent, terminal_io, SubagentContext::new(history, None))
    }

    pub(crate) fn with_history(
        agent: Agent<CM>,
        terminal_io: Arc<TerminalIO>,
        history: Arc<ChatHistory<NonPersistentHistory>>,
    ) -> Self {
        Self::with_history_context(agent, terminal_io, SubagentContext::new(history, None))
    }

    pub(crate) fn with_history_context(
        agent: Agent<CM>,
        terminal_io: Arc<TerminalIO>,
        context: SubagentContext,
    ) -> Self {
        Self {
            agent: Arc::new(Mutex::new(agent)),
            terminal_io,
            history: context.history,
            context_usage: Mutex::new(ContextUsage::new(None, context.max_context_tokens)),
        }
    }

    async fn streamed_turn(&self) -> StreamedTurn<'_, CM, NonPersistentHistory> {
        let context_usage = *self.context_usage.lock().await;
        StreamedTurn::with_context(
            self.agent.clone(),
            self.terminal_io.as_ref(),
            StreamedTurnContext::new(self.history.as_ref(), context_usage),
        )
    }

    /// Executes the supplied task without reading from the terminal.
    pub(crate) async fn run(&self, task: String) -> anyhow::Result<String> {
        let prompt = StaticPromptProvider.task(task.as_str());
        let base = self.history.snapshot().await;
        let streamed_turn = self.streamed_turn().await;
        let outcome = streamed_turn.run(prompt, base).await?;
        let status = self.complete_turn(streamed_turn, outcome).await?;
        self.update_context_usage(status.context_usage()).await;
        self.compact_after_turn(&status).await?;
        match status.output() {
            Some(output) => Ok(output.to_string()),
            None => anyhow::bail!(
                "subagent recovery stopped: {}",
                status.error().unwrap_or("unknown error")
            ),
        }
    }

    async fn complete_turn(
        &self,
        streamed_turn: StreamedTurn<'_, CM, NonPersistentHistory>,
        outcome: StreamRunOutcome,
    ) -> anyhow::Result<TurnStatus> {
        let context_usage = match &outcome {
            StreamRunOutcome::Completed(completion) => completion.context_usage(),
            StreamRunOutcome::Failed(failure) => failure.context_usage(),
        };
        let request = self.recovery_request(&outcome);
        let Some(request) = request else {
            return self.completed_status(outcome);
        };

        let status = TurnRecoverer::new(streamed_turn)
            .recover(request, context_usage)
            .await?;
        Ok(status)
    }

    fn recovery_request(&self, outcome: &StreamRunOutcome) -> Option<RecoveryRequest> {
        match outcome {
            StreamRunOutcome::Failed(failure) => {
                Some(RecoveryRequest::StreamFailure(failure.error().to_string()))
            }
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

    fn completed_status(&self, outcome: StreamRunOutcome) -> anyhow::Result<TurnStatus> {
        match outcome {
            StreamRunOutcome::Completed(completion) => Ok(TurnStatus::Complete {
                output: completion.output().to_string(),
                context_usage: completion.context_usage(),
            }),
            StreamRunOutcome::Failed(failure) => {
                anyhow::bail!("subagent failed: {}", failure.error())
            }
        }
    }

    async fn compact_after_turn(&self, status: &TurnStatus) -> anyhow::Result<()> {
        if !matches!(status, TurnStatus::Complete { .. })
            || !status.context_usage().should_compact()
        {
            return Ok(());
        }

        self.terminal_io.eprintln_red("Compacting context...");
        let result = ContextCompactor::new(&self.agent, self.history.as_ref())
            .compact(CompactionKind::Automatic)
            .await?;
        if result.kind() == CompactionKind::Automatic && result.compacted() {
            self.reset_used_tokens().await;
            self.terminal_io.eprintln_red("Context compacted.");
        } else {
            self.terminal_io.eprintln_red("Compacting failed.");
        }
        Ok(())
    }

    async fn update_context_usage(&self, context_usage: ContextUsage) {
        *self.context_usage.lock().await = context_usage;
    }

    async fn reset_used_tokens(&self) {
        let mut context_usage = self.context_usage.lock().await;
        *context_usage = ContextUsage::new(None, context_usage.max_context_tokens());
    }
}

/// Builds the provider-specific runner used by the provider-neutral spawn tool.
pub(crate) struct RuntimeSubagentRunner {
    subagent: Subagent<CompletionModel>,
}

impl RuntimeSubagentRunner {
    /// Creates an isolated subagent session from the current runtime dependencies.
    pub(crate) fn build(
        terminal_io: Arc<TerminalIO>,
        dependencies: Arc<AgentDependencies>,
        config: AgentConfig,
    ) -> BoxFuture<'static, anyhow::Result<Arc<dyn SubagentRunner>>> {
        Box::pin(async move {
            let history = Arc::new(ChatHistory::new(Vec::new(), NonPersistentHistory));
            let configured =
                AgentFactory::new_subagent(config, history.clone(), dependencies).await?;
            let subagent = Subagent::with_history_context(
                configured.agent,
                terminal_io,
                SubagentContext::new(history, configured.max_context_tokens),
            );
            Ok(Arc::new(Self { subagent }) as Arc<dyn SubagentRunner>)
        })
    }
}

impl SubagentRunner for RuntimeSubagentRunner {
    fn run(
        &self,
        task: SubagentTask,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SubagentReport>> + Send + '_>> {
        Box::pin(async move {
            self.subagent
                .run(task.task)
                .await
                .map(|report| SubagentReport { report })
        })
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
    use crate::{
        adapters::terminal::terminal_io::TerminalIO,
        entities::conversation::context_usage::ContextUsage,
        entities::conversation::turn::TurnStatus, prompts::subagent::subagent_prompt,
    };

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

    #[tokio::test]
    async fn automatic_compaction_uses_the_subagent_history_and_limit() -> Result<()> {
        let model = MockCompletionModel::new([rig::test_utils::MockTurn::text("subagent summary")]);
        let agent = AgentBuilder::new(model).build();
        let history = Arc::new(
            crate::adapters::model_runtime::chat::history::chat_history::ChatHistory::new(
                vec![Message::user("old subagent context")],
                crate::adapters::model_runtime::chat::history::history_persistence::NonPersistentHistory,
            ),
        );
        let subagent = Subagent::with_history_context(
            agent,
            Arc::new(TerminalIO),
            super::SubagentContext::new(history, Some(1_000)),
        );
        let status = TurnStatus::Complete {
            output: "report".to_string(),
            context_usage: ContextUsage::new(Some(800), Some(1_000)),
        };

        subagent.compact_after_turn(&status).await?;

        assert_eq!(
            subagent.history.snapshot().await,
            vec![Message::assistant("subagent summary")]
        );
        assert_eq!(
            *subagent.context_usage.lock().await,
            ContextUsage::new(None, Some(1_000))
        );
        Ok(())
    }
}
