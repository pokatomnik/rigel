use std::{future::Future, pin::Pin, sync::Arc};

use rig::completion::CompletionModel;
use tokio::sync::Mutex;

use crate::{
    controllers::model_runtime::{
        compaction::context_compactor::RigContextSummarizer,
        history::{
            chat_history::ChatHistory, history_persistence::HistoryPersistence,
            persistence::History,
        },
        recovery::turn_recovery::TurnRecoverer,
        streaming::streamed_turn::{StreamRunOutcome, StreamedTurn, StreamedTurnContext},
    },
    controllers::{
        model_runtime::agent::{
            agent_config::AgentConfig,
            dependencies::{AgentWiring, ConfiguredAgent},
            runtime::Agent,
        },
        terminal::terminal_io::TerminalIO,
    },
    entities::model::model_session_kind::ModelSessionKind,
    entities::{
        conversation::{
            context_usage::ContextUsage,
            turn::{CompactionKind, CompactionResult, TurnStatus},
        },
        recovery::RecoveryRequest,
        selected_model::selection::SelectedModel,
    },
    infrastructure::{
        conversation::ContextCompaction,
        model::{ModelSession, ModelSessionFactory},
        recovery::TurnRecovery,
    },
};

/// A Rig-backed session that keeps provider-specific streaming behind the model contract.
pub(crate) struct RigModelSession<CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    agent: Arc<Mutex<rig::Agent<CM>>>,
    prompt_provider: Option<Arc<dyn crate::infrastructure::prompts::PromptProvider>>,
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<P>>,
    max_context_tokens: Option<u64>,
    selected_model: Option<SelectedModel>,
    turn_recovery: Option<Arc<dyn TurnRecovery>>,
    context_compaction: Option<Arc<dyn ContextCompaction>>,
    require_answer: bool,
}

impl<CM, P> RigModelSession<CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    /// Creates a session around a configured agent and the history shared with its hooks.
    pub(crate) fn from_configured(
        configured: ConfiguredAgent<CM>,
        terminal_io: Arc<TerminalIO>,
        history: Arc<ChatHistory<P>>,
    ) -> Self {
        Self {
            agent: Arc::new(Mutex::new(configured.agent)),
            prompt_provider: None,
            terminal_io,
            history,
            max_context_tokens: configured.max_context_tokens,
            selected_model: configured.selected_model,
            turn_recovery: None,
            context_compaction: None,
            require_answer: false,
        }
    }

    /// Supplies explicit test-only recovery and compaction capabilities for runtime tests.
    #[cfg(test)]
    pub(crate) fn with_test_capabilities(self) -> Self {
        let prompts = Arc::new(crate::prompts::provider::StaticPromptProvider);
        self.with_prompt_provider(prompts.clone())
            .with_turn_recovery(Arc::new(
                crate::use_cases::chat::recovery::turn_recovery_workflow::TurnRecoveryWorkflow::new(
                    prompts,
                ),
            ))
            .with_context_compaction(Arc::new(
                crate::use_cases::chat::compaction::compact_context::CompactContext,
            ))
    }

    /// Installs recovery behavior selected by the composition root.
    pub(super) fn with_turn_recovery(mut self, recovery: Arc<dyn TurnRecovery>) -> Self {
        self.turn_recovery = Some(recovery);
        self
    }

    /// Installs prompt composition for this provider session.
    pub(super) fn with_prompt_provider(
        mut self,
        provider: Arc<dyn crate::infrastructure::prompts::PromptProvider>,
    ) -> Self {
        self.prompt_provider = Some(provider);
        self
    }

    /// Installs the application compaction policy for this session's history.
    pub(super) fn with_context_compaction(mut self, policy: Arc<dyn ContextCompaction>) -> Self {
        self.context_compaction = Some(policy);
        self
    }

    /// Requires non-empty model output and routes omissions through recovery.
    pub(super) fn requiring_answer(mut self) -> Self {
        self.require_answer = true;
        self
    }

    async fn complete_turn(
        &self,
        streamed_turn: StreamedTurn<'_, CM, P>,
        outcome: StreamRunOutcome,
    ) -> anyhow::Result<TurnStatus> {
        let Some((request, usage)) = self.recovery_request(&outcome) else {
            return Self::completed_turn(outcome);
        };
        let recovery = self
            .turn_recovery
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("turn recovery workflow was not configured"))?;
        TurnRecoverer::new(streamed_turn, recovery.clone())
            .recover(request, usage)
            .await
    }

    fn streamed_turn_context(&self, context_usage: ContextUsage) -> StreamedTurnContext<'_, P> {
        let context = StreamedTurnContext::new(self.history.as_ref(), context_usage);
        match self.selected_model.clone() {
            Some(model) => context.with_selected_model(model),
            None => context,
        }
    }

    fn recovery_request(
        &self,
        outcome: &StreamRunOutcome,
    ) -> Option<(RecoveryRequest, ContextUsage)> {
        let usage = match outcome {
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
            StreamRunOutcome::Completed(completion)
                if completion.requires_answer_recovery()
                    || (self.require_answer && !completion.has_output()) =>
            {
                RecoveryRequest::MissingAnswer
            }
            StreamRunOutcome::Completed(_) => return None,
        };
        Some((request, usage))
    }

    fn completed_turn(outcome: StreamRunOutcome) -> anyhow::Result<TurnStatus> {
        match outcome {
            StreamRunOutcome::Completed(completion) => Ok(TurnStatus::Complete {
                output: completion.output().to_string(),
                context_usage: completion.context_usage(),
            }),
            StreamRunOutcome::Failed(failure) => {
                anyhow::bail!(
                    "unexpected failure without a recovery request: {}",
                    failure.error()
                )
            }
        }
    }
}

impl<CM, P> ModelSession for RigModelSession<CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    fn run_turn(
        &self,
        prompt: String,
        context_usage: ContextUsage,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TurnStatus>> + Send + '_>> {
        Box::pin(async move {
            let base = self.history.snapshot_result().await?;
            let streamed_turn = StreamedTurn::with_context(
                self.agent.clone(),
                self.terminal_io.as_ref(),
                self.streamed_turn_context(context_usage),
            );
            let outcome = streamed_turn.run(prompt, base).await?;
            self.complete_turn(streamed_turn, outcome).await
        })
    }

    fn compact(
        &self,
        kind: CompactionKind,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<CompactionResult>> + Send + '_>> {
        Box::pin(async move {
            let provider = self
                .prompt_provider
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("prompt provider was not configured"))?;
            let policy = self
                .context_compaction
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("context compaction policy was not configured"))?;
            let summarizer = RigContextSummarizer::new(&self.agent, provider.as_ref());
            policy
                .compact(kind, &summarizer, self.history.as_ref())
                .await
        })
    }

    fn max_context_tokens(&self) -> Option<u64> {
        self.max_context_tokens
    }
}

/// Creates persistent chat and model-replacement sessions from capabilities selected by the composition root.
///
/// Delegated tasks use `RigSubagentSessionFactory` so each task receives an isolated history.
pub(crate) struct RigModelSessionFactory {
    config: AgentConfig,
    dependencies: Arc<AgentWiring>,
    history: Arc<ChatHistory<History>>,
}

impl RigModelSessionFactory {
    /// Creates a factory for persistent chat sessions and isolated subagent sessions.
    pub(crate) fn new(
        config: AgentConfig,
        dependencies: Arc<AgentWiring>,
        history: Arc<ChatHistory<History>>,
    ) -> Self {
        Self {
            config,
            dependencies,
            history,
        }
    }

    fn configured(&self, selected_model: Option<SelectedModel>) -> anyhow::Result<AgentConfig> {
        match selected_model {
            Some(model) => self.config.clone().with_selected_model(model),
            None => Ok(self.config.clone()),
        }
    }
}

impl RigModelSessionFactory {
    async fn build_chat_session(
        &self,
        config: AgentConfig,
        kind: ModelSessionKind,
    ) -> anyhow::Result<Box<dyn ModelSession>> {
        if kind == ModelSessionKind::ModelChange {
            self.dependencies.reselect_mcp().await;
        }
        let configured =
            Agent::new_chat_agent(config, self.history.clone(), self.dependencies.clone()).await?;
        let session = RigModelSession::from_configured(
            configured,
            self.dependencies.terminal_io(),
            self.history.clone(),
        )
        .with_turn_recovery(self.dependencies.turn_recovery()?)
        .with_prompt_provider(self.dependencies.prompt_provider()?)
        .with_context_compaction(self.dependencies.context_compaction()?);
        Ok(Box::new(session))
    }
}

impl ModelSessionFactory for RigModelSessionFactory {
    fn create(
        &self,
        kind: ModelSessionKind,
        selected_model: Option<SelectedModel>,
    ) -> futures::future::BoxFuture<'_, anyhow::Result<Box<dyn ModelSession>>> {
        Box::pin(async move {
            if kind == ModelSessionKind::Subagent {
                anyhow::bail!("chat session factory cannot create subagent sessions");
            }
            let config = self.configured(selected_model)?;
            self.build_chat_session(config, kind).await
        })
    }
}
