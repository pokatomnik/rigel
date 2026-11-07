#[cfg(test)]
mod legacy {
    use std::{future::Future, pin::Pin, sync::Arc};

    use futures::future::BoxFuture;

    use rig::{Agent, completion::CompletionModel as CompletionModelTrait};
    use tokio::sync::Mutex;

    use crate::{
        controllers::model_runtime::{
            compaction::context_compactor::RigContextSummarizer,
            history::{
                chat_history::{ChatHistory, InMemoryHistory},
                history_persistence::NonPersistentHistory,
            },
            recovery::turn_recovery::TurnRecoverer,
            streaming::streamed_turn::{StreamRunOutcome, StreamedTurn, StreamedTurnContext},
        },
        controllers::terminal::terminal_io::TerminalIO,
        entities::{
            conversation::{
                context_usage::ContextUsage,
                turn::{CompactionKind, TurnStatus},
            },
            recovery::RecoveryRequest,
            tool::tool_entities::{SubagentReport, SubagentTask},
        },
        infrastructure::{
            conversation::ContextCompaction, recovery::TurnRecovery,
            tools::subagent::SubagentRunner,
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
        prompt_provider: Arc<dyn crate::infrastructure::prompts::PromptProvider>,
        turn_recovery: Option<Arc<dyn TurnRecovery>>,
    }

    pub(crate) struct SubagentContext {
        history: Arc<ChatHistory<NonPersistentHistory>>,
        max_context_tokens: Option<u64>,
        prompt_provider: Arc<dyn crate::infrastructure::prompts::PromptProvider>,
        turn_recovery: Option<Arc<dyn TurnRecovery>>,
    }

    impl SubagentContext {
        /// Creates subagent session context; recovery is supplied separately via its builder.
        pub(crate) fn new(
            history: Arc<ChatHistory<NonPersistentHistory>>,
            max_context_tokens: Option<u64>,
            prompt_provider: Arc<dyn crate::infrastructure::prompts::PromptProvider>,
        ) -> Self {
            Self {
                history,
                max_context_tokens,
                prompt_provider,
                turn_recovery: None,
            }
        }

        fn with_turn_recovery(mut self, recovery: Arc<dyn TurnRecovery>) -> Self {
            self.turn_recovery = Some(recovery);
            self
        }
    }

    impl<CM> Subagent<CM>
    where
        CM: CompletionModelTrait + 'static,
    {
        #[cfg(test)]
        fn new(agent: Agent<CM>, terminal_io: Arc<TerminalIO>) -> Self {
            let history = Arc::new(ChatHistory::<NonPersistentHistory>::new(Arc::new(
                InMemoryHistory::new(Vec::new()),
            )));
            Self::with_history_context(
            agent,
            terminal_io,
            SubagentContext::new(
                history,
                None,
                Arc::new(crate::prompts::provider::StaticPromptProvider),
            )
            .with_turn_recovery(Arc::new(
                crate::use_cases::chat::recovery::turn_recovery_workflow::TurnRecoveryWorkflow::new(
                    Arc::new(crate::prompts::provider::StaticPromptProvider),
                ),
            )),
        )
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
                prompt_provider: context.prompt_provider,
                turn_recovery: context.turn_recovery,
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
            let prompt = self.prompt_provider.task(task.as_str());
            let base = self.history.snapshot_result().await?;
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

            let recovery = self
                .turn_recovery
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("turn recovery workflow was not configured"))?;
            let status = TurnRecoverer::new(streamed_turn, recovery.clone())
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
                StreamRunOutcome::Completed(completion)
                    if completion.requires_answer_recovery() =>
                {
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
            let policy = crate::use_cases::chat::compaction::compact_context::CompactContext;
            let summarizer = RigContextSummarizer::new(&self.agent, self.prompt_provider.as_ref());
            let result = policy
                .compact(
                    CompactionKind::Automatic,
                    &summarizer,
                    self.history.as_ref(),
                )
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

    type SubagentFactory<CM> =
        Arc<dyn Fn() -> BoxFuture<'static, anyhow::Result<Subagent<CM>>> + Send + Sync>;

    /// Builds a fresh provider-specific runner for every delegated task.
    pub(crate) struct RuntimeSubagentRunner<CM>
    where
        CM: CompletionModelTrait,
    {
        factory: SubagentFactory<CM>,
    }

    #[cfg(test)]
    impl<CM> RuntimeSubagentRunner<CM>
    where
        CM: CompletionModelTrait + 'static,
    {
        fn from_test_factory(factory: SubagentFactory<CM>) -> Self {
            Self { factory }
        }
    }

    impl<CM> SubagentRunner for RuntimeSubagentRunner<CM>
    where
        CM: CompletionModelTrait + 'static,
    {
        fn run(
            &self,
            task: SubagentTask,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<SubagentReport>> + Send + '_>> {
            let factory = self.factory.clone();
            Box::pin(async move {
                let subagent = factory().await?;
                subagent
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
            controllers::terminal::terminal_io::TerminalIO,
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
        async fn one_runner_call_gets_a_fresh_session_and_history() -> Result<()> {
            use std::sync::atomic::{AtomicUsize, Ordering};

            use super::RuntimeSubagentRunner;
            use crate::entities::tool::tool_entities::SubagentTask;
            use crate::infrastructure::tools::subagent::SubagentRunner;

            let creations = Arc::new(AtomicUsize::new(0));
            let creations_for_factory = creations.clone();
            let factory = Arc::new(move || {
                let index = creations_for_factory.fetch_add(1, Ordering::SeqCst) + 1;
                Box::pin(async move {
                    let model = MockCompletionModel::from_stream_turns([vec![
                        MockStreamEvent::text(format!("report-{index}")),
                        MockStreamEvent::final_response_with_total_tokens(1),
                    ]]);
                    Ok(Subagent::new(
                        AgentBuilder::new(model).build(),
                        Arc::new(TerminalIO),
                    ))
                })
                    as futures::future::BoxFuture<
                        'static,
                        anyhow::Result<Subagent<MockCompletionModel>>,
                    >
            });
            let runner = RuntimeSubagentRunner::from_test_factory(factory);
            let first = runner.run(SubagentTask::new("first".to_string())).await?;
            let second = runner.run(SubagentTask::new("second".to_string())).await?;

            assert_eq!(creations.load(Ordering::SeqCst), 2);
            assert_eq!(first.report, "report-1");
            assert_eq!(second.report, "report-2");
            Ok(())
        }

        #[tokio::test]
        async fn automatic_compaction_uses_the_subagent_history_and_limit() -> Result<()> {
            let model =
                MockCompletionModel::new([rig::test_utils::MockTurn::text("subagent summary")]);
            let agent = AgentBuilder::new(model).build();
            let history = Arc::new(
            crate::controllers::model_runtime::history::chat_history::ChatHistory::from_provider(
                vec![Message::user("old subagent context")],
                crate::controllers::model_runtime::history::history_persistence::NonPersistentHistory,
            )?,
        );
            let subagent = Subagent::with_history_context(
                agent,
                Arc::new(TerminalIO),
                super::SubagentContext::new(
                    history,
                    Some(1_000),
                    Arc::new(crate::prompts::provider::StaticPromptProvider),
                ),
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
}
