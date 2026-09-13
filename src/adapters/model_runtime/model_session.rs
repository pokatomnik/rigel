use std::{future::Future, pin::Pin, sync::Arc};

use rig::completion::CompletionModel;
use tokio::sync::Mutex;

use crate::{
    adapters::{
        model_runtime::{
            agent::{
                agent::Agent,
                agent_config::AgentConfig,
                dependencies::{AgentDependencies, ConfiguredAgent},
            },
            chat::{
                compaction::context_compactor::ContextCompactor,
                history::{
                    chat_history::ChatHistory,
                    history::History,
                    history_persistence::{HistoryPersistence, NonPersistentHistory},
                },
                recovery::turn_recovery::{RecoveryRequest, TurnRecoverer},
                streaming::streamed_turn::{StreamRunOutcome, StreamedTurn, StreamedTurnContext},
            },
        },
        terminal::terminal_io::TerminalIO,
    },
    contracts::model::{ModelSession, ModelSessionFactory, ModelSessionKind},
    entities::{
        conversation::{
            context_usage::ContextUsage,
            turn::{CompactionKind, CompactionResult, TurnStatus},
        },
        selected_model::selected_model::SelectedModel,
    },
};

/// A Rig-backed session that keeps provider-specific streaming behind the model contract.
pub(crate) struct RigModelSession<CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    agent: Arc<Mutex<rig::Agent<CM>>>,
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<P>>,
    max_context_tokens: Option<u64>,
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
            terminal_io,
            history,
            max_context_tokens: configured.max_context_tokens,
        }
    }

    async fn complete_turn(
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
            let base = self.history.snapshot().await;
            let streamed_turn = StreamedTurn::with_context(
                self.agent.clone(),
                self.terminal_io.as_ref(),
                StreamedTurnContext::new(self.history.as_ref(), context_usage),
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
            ContextCompactor::new(&self.agent, self.history.as_ref())
                .compact(kind)
                .await
        })
    }

    fn max_context_tokens(&self) -> Option<u64> {
        self.max_context_tokens
    }
}

/// Builds complete Rig sessions while leaving the replacement operation transactional to Chat.
pub(crate) struct RigModelSessionFactory {
    config: AgentConfig,
    dependencies: Arc<AgentDependencies>,
    history: Arc<ChatHistory<Arc<History>>>,
}

impl RigModelSessionFactory {
    /// Creates a factory for persistent chat sessions and isolated subagent sessions.
    pub(crate) fn new(
        config: AgentConfig,
        dependencies: Arc<AgentDependencies>,
        history: Arc<ChatHistory<Arc<History>>>,
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

impl ModelSessionFactory for RigModelSessionFactory {
    fn create(
        &self,
        kind: ModelSessionKind,
        selected_model: Option<SelectedModel>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Box<dyn ModelSession>>> + '_>> {
        Box::pin(async move {
            let config = self.configured(selected_model)?;
            if kind == ModelSessionKind::Subagent {
                let history = Arc::new(ChatHistory::new(Vec::new(), NonPersistentHistory));
                let configured =
                    Agent::new_subagent(config, history.clone(), self.dependencies.clone()).await?;
                let session = RigModelSession::from_configured(
                    configured,
                    self.dependencies.terminal_io(),
                    history,
                );
                return Ok(Box::new(session) as Box<dyn ModelSession>);
            }

            if kind == ModelSessionKind::ModelChange {
                self.dependencies.reselect_mcp().await;
            }
            let configured =
                Agent::new_chat_agent(config, self.history.clone(), self.dependencies.clone())
                    .await?;
            let session = RigModelSession::from_configured(
                configured,
                self.dependencies.terminal_io(),
                self.history.clone(),
            );
            Ok(Box::new(session) as Box<dyn ModelSession>)
        })
    }
}
