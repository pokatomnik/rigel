use std::sync::Arc;

use crate::{
    controllers::model_runtime::{
        agent::{agent_config::AgentConfig, dependencies::AgentWiring, runtime::Agent},
        history::{
            chat_history::{ChatHistory, InMemoryHistory},
            history_persistence::NonPersistentHistory,
        },
        model_session::RigModelSession,
    },
    entities::{
        model::model_session_kind::ModelSessionKind, selected_model::selection::SelectedModel,
    },
    infrastructure::model::{ModelSession, ModelSessionFactory},
};

/// Creates a fresh non-persistent Rig session for every delegated task.
pub(crate) struct RigSubagentSessionFactory {
    config: AgentConfig,
    dependencies: Arc<AgentWiring>,
}

impl RigSubagentSessionFactory {
    /// Creates a factory that owns runtime capabilities and builds isolated task sessions.
    pub(crate) fn new(config: AgentConfig, dependencies: Arc<AgentWiring>) -> Self {
        Self {
            config,
            dependencies,
        }
    }

    async fn build_session(
        &self,
        selected_model: Option<SelectedModel>,
    ) -> anyhow::Result<Box<dyn ModelSession>> {
        let history = Arc::new(ChatHistory::<NonPersistentHistory>::new(Arc::new(
            InMemoryHistory::new(Vec::new()),
        )));
        let config = match selected_model {
            Some(model) => self.config.clone().with_selected_model(model)?,
            None => self.config.clone(),
        };
        let configured =
            Agent::new_subagent(config, history.clone(), self.dependencies.clone()).await?;
        let session =
            RigModelSession::from_configured(configured, self.dependencies.terminal_io(), history)
                .with_turn_recovery(self.dependencies.turn_recovery()?)
                .with_prompt_provider(self.dependencies.prompt_provider()?)
                .with_context_compaction(self.dependencies.context_compaction()?)
                .requiring_answer();
        Ok(Box::new(session))
    }
}

impl ModelSessionFactory for RigSubagentSessionFactory {
    fn create(
        &self,
        kind: ModelSessionKind,
        selected_model: Option<SelectedModel>,
    ) -> futures::future::BoxFuture<'_, anyhow::Result<Box<dyn ModelSession>>> {
        Box::pin(async move {
            if kind != ModelSessionKind::Subagent {
                anyhow::bail!("subagent factory only accepts subagent sessions");
            }
            self.build_session(selected_model).await
        })
    }
}
