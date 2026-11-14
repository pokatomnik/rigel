use std::sync::Arc;

use crate::{
    controllers::{
        model_runtime::{
            agent::agent_config::AgentConfig, agent::runtime::Agent, session::chat::RigModelSession,
        },
        storage::history::JsonlHistory,
    },
    entities::{
        model::model_session_kind::ModelSessionKind, selected_model::selection::SelectedModel,
    },
    infrastructure::model::{ModelSession, ModelSessionFactory},
};

use crate::controllers::model_runtime::{
    agent::dependencies::AgentWiring, history::chat_history::ChatHistory,
};

/// Creates persistent chat and model-replacement sessions from capabilities selected by the composition root.
///
/// Delegated tasks use `RigSubagentSessionFactory` so each task receives an isolated history.
pub(crate) struct RigModelSessionFactory {
    config: AgentConfig,
    dependencies: Arc<AgentWiring>,
    history: Arc<ChatHistory<JsonlHistory>>,
}

impl RigModelSessionFactory {
    /// Creates a factory for persistent chat sessions and isolated subagent sessions.
    pub(crate) fn new(
        config: AgentConfig,
        dependencies: Arc<AgentWiring>,
        history: Arc<ChatHistory<JsonlHistory>>,
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
        .with_context_compaction(self.dependencies.context_compaction()?)
        .with_events(self.dependencies.event_sink()?)
        .with_cancellation(self.dependencies.run_cancellation()?);
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
