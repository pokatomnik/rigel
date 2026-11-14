use crate::{
    controllers::{
        model_runtime::{
            agent::dependencies::ConfiguredAgent,
            compaction::context_compactor::RigContextSummarizer,
            history::{chat_history::ChatHistory, history_persistence::HistoryPersistence},
            recovery::turn_recovery::TurnRecoverer,
            streaming::{
                streamed_turn::{StreamRunOutcome, StreamedTurn},
                turn_context::{StreamedTurnContext, overflow_error},
            },
        },
        terminal::terminal_io::TerminalIO,
    },
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
        model::ModelSession,
        recovery::TurnRecovery,
        ui::{cancellation::RunCancellation, gateway::EventSink},
    },
};
use rig::completion::CompletionModel;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;
/// A Rig-backed session that keeps provider-specific streaming behind the model contract.
pub(crate) struct RigModelSession<CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    agent: Arc<Mutex<rig::Agent<CM>>>,
    summarizer: Option<Arc<Mutex<rig::Agent<CM>>>>,
    prompt_provider: Option<Arc<dyn crate::infrastructure::prompts::PromptProvider>>,
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<P>>,
    max_context_tokens: Option<u64>,
    selected_model: Option<SelectedModel>,
    turn_recovery: Option<Arc<dyn TurnRecovery>>,
    context_compaction: Option<Arc<dyn ContextCompaction>>,
    events: Option<Arc<dyn EventSink>>,
    cancellation: Option<Arc<dyn RunCancellation>>,
    turn_sequence: std::sync::atomic::AtomicU64,
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
            summarizer: configured
                .summarizer
                .map(|agent| Arc::new(Mutex::new(agent))),
            prompt_provider: None,
            terminal_io,
            history,
            max_context_tokens: configured.max_context_tokens,
            selected_model: configured.selected_model,
            turn_recovery: None,
            context_compaction: None,
            events: None,
            cancellation: None,
            turn_sequence: std::sync::atomic::AtomicU64::new(0),
            require_answer: false,
        }
    }
    /// Installs recovery behavior selected by the composition root.
    pub(crate) fn with_turn_recovery(mut self, recovery: Arc<dyn TurnRecovery>) -> Self {
        self.turn_recovery = Some(recovery);
        self
    }
    /// Installs prompt composition for this provider session.
    pub(crate) fn with_prompt_provider(
        mut self,
        provider: Arc<dyn crate::infrastructure::prompts::PromptProvider>,
    ) -> Self {
        self.prompt_provider = Some(provider);
        self
    }
    /// Installs the application compaction policy for this session's history.
    pub(crate) fn with_context_compaction(mut self, policy: Arc<dyn ContextCompaction>) -> Self {
        self.context_compaction = Some(policy);
        self
    }
    /// Installs the live event sink used by streaming translation.
    pub(crate) fn with_events(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = Some(events);
        self
    }
    /// Installs the soft-stop flag checked by streamed turns.
    pub(crate) fn with_cancellation(mut self, cancellation: Arc<dyn RunCancellation>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
    /// Requires non-empty model output and routes omissions through recovery.
    pub(crate) fn requiring_answer(mut self) -> Self {
        self.require_answer = true;
        self
    }
    async fn complete_turn(
        &self,
        streamed_turn: StreamedTurn<'_, CM, P>,
        outcome: StreamRunOutcome,
    ) -> anyhow::Result<TurnStatus> {
        if let StreamRunOutcome::Cancelled { context_usage } = &outcome {
            return Ok(TurnStatus::Stopped {
                context_usage: *context_usage,
            });
        }
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
        let mut context = StreamedTurnContext::new(self.history.as_ref(), context_usage);
        if let Some(model) = self.selected_model.clone() {
            context = context.with_selected_model(model);
        }
        if let Some(events) = self.events.clone() {
            context = context.with_events(events);
        }
        if let Some(cancellation) = self.cancellation.clone() {
            context = context.with_cancellation(cancellation);
        }
        context.with_turn_id(
            self.turn_sequence
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                .saturating_add(1),
        )
    }
    fn recovery_request(
        &self,
        outcome: &StreamRunOutcome,
    ) -> Option<(RecoveryRequest, ContextUsage)> {
        let usage = match outcome {
            StreamRunOutcome::Completed(completion) => completion.context_usage(),
            StreamRunOutcome::Failed(failure) => failure.context_usage(),
            StreamRunOutcome::Cancelled { context_usage } => *context_usage,
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
            StreamRunOutcome::Completed(_) | StreamRunOutcome::Cancelled { .. } => return None,
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
            StreamRunOutcome::Cancelled { context_usage } => {
                Ok(TurnStatus::Stopped { context_usage })
            }
        }
    }

    fn new_streamed_turn(&self, context_usage: ContextUsage) -> StreamedTurn<'_, CM, P> {
        StreamedTurn::with_context(
            self.agent.clone(),
            self.terminal_io.as_ref(),
            self.streamed_turn_context(context_usage),
        )
    }

    async fn run_with_overflow_retry(
        &self,
        prompt: String,
        context_usage: ContextUsage,
    ) -> anyhow::Result<TurnStatus> {
        let streamed_turn = self.new_streamed_turn(context_usage);
        let base = self.history.snapshot_result().await?;
        let outcome = streamed_turn.run(prompt.clone(), base).await?;
        if let Some(cancellation) = self.cancellation.as_ref() {
            cancellation.reset();
        }
        let Some(error) = overflow_error(&outcome).map(str::to_string) else {
            return self.complete_turn(streamed_turn, outcome).await;
        };
        self.retry_after_overflow(prompt, error.as_str()).await
    }

    async fn retry_after_overflow(
        &self,
        prompt: String,
        error: &str,
    ) -> anyhow::Result<TurnStatus> {
        let result = self.compact(CompactionKind::Automatic).await?;
        if !result.compacted() {
            anyhow::bail!("context overflow was not resolved by compaction: {error}");
        }
        let retry_turn = self.new_streamed_turn(ContextUsage::new(None, self.max_context_tokens));
        let retry_base = self.history.snapshot_result().await?;
        let retry_outcome = retry_turn.run(prompt, retry_base).await?;
        if let Some(retry_error) = overflow_error(&retry_outcome) {
            anyhow::bail!("context is still too large after compaction: {retry_error}");
        }
        self.complete_turn(retry_turn, retry_outcome).await
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
        Box::pin(async move { self.run_with_overflow_retry(prompt, context_usage).await })
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
            let summarizer_agent = self
                .summarizer
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("tool-free summarizer agent was not configured"))?;
            let summarizer = RigContextSummarizer::new(summarizer_agent, provider.as_ref());
            policy
                .compact(kind, &summarizer, self.history.as_ref())
                .await
        })
    }

    fn max_context_tokens(&self) -> Option<u64> {
        self.max_context_tokens
    }

    fn selected_model(&self) -> Option<SelectedModel> {
        self.selected_model.clone()
    }

    fn attach_events(&mut self, events: Arc<dyn EventSink>) {
        self.events = Some(events);
    }
}

#[cfg(test)]
impl<CM, P> RigModelSession<CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    /// Supplies explicit test-only recovery and compaction capabilities for runtime tests.
    pub(crate) fn with_test_capabilities(mut self) -> Self {
        if self.summarizer.is_none() {
            self.summarizer = Some(self.agent.clone());
        }
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
}
