use std::sync::Arc;

use rig::{
    agent::{AgentBuilder, WithBuilderTools},
    client::AgentClientExt,
    completion::CompletionModel as CompletionModelTrait,
    providers::openai::CompletionModel,
};

use super::{
    agent_config::AgentConfig,
    agent_tool_set::AgentToolSet,
    dependencies::{AgentWiring, ConfiguredAgent},
};
use crate::{
    controllers::model_runtime::{
        history::{
            chat_history::ChatHistory,
            history_persistence::{HistoryPersistence, NonPersistentHistory},
            persistence::History,
        },
        recovery::tool_recovery::ToolRecoveryHook,
        response::invalid_response::InvalidResponseHook,
    },
    controllers::{
        mcp_registry::registry::McpToolsExt,
        model_runtime::{
            goal_completion_hook::GoalCompletionHook, history_sync::HistorySyncHook,
            permissions::catalog::ToolPermissionCatalog, tool_permission_hook::ToolPermissionHook,
        },
    },
    entities::selected_model::selection::SelectedModel,
    infrastructure::model::ModelCatalog,
};

const MAX_AGENT_TURNS: usize = 12;

/// Builds one provider session from composition-root-selected capabilities.
pub(crate) struct Agent {
    pub(super) config: AgentConfig,
    pub(super) wiring: Arc<AgentWiring>,
}

impl Agent {
    /// Builds a fresh non-persistent subagent session.
    pub(crate) async fn new_subagent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<NonPersistentHistory>>,
        wiring: Arc<AgentWiring>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>> {
        Self::build_agent(
            config.with_tool_set(AgentToolSet::Subagent),
            chat_history,
            wiring,
        )
        .await
    }

    /// Builds a persistent interactive chat session.
    pub(crate) async fn new_chat_agent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<History>>,
        wiring: Arc<AgentWiring>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>> {
        Self::build_agent(
            config.with_tool_set(AgentToolSet::Chat),
            chat_history,
            wiring,
        )
        .await
    }

    async fn build_agent<P>(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<P>>,
        wiring: Arc<AgentWiring>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>>
    where
        P: HistoryPersistence,
    {
        let client = Self::build_client(&config, &wiring)?;
        let selected_model = Self::select_model(&config, &client, &wiring).await?;
        let config = Self::configure_model(config, &selected_model)?;
        let agent = Self { config, wiring };
        agent
            .build_session(client, selected_model, chat_history)
            .await
    }

    async fn build_session<P>(
        &self,
        client: rig::providers::openai::Client,
        selected_model: SelectedModel,
        chat_history: Arc<ChatHistory<P>>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>>
    where
        P: HistoryPersistence,
    {
        let builder = self
            .build_builder(&client, selected_model.id.clone())
            .await?;
        let (builder, catalog) = self.register_tools(builder).await?;
        let builder = self.add_hooks(builder, chat_history);
        let manager = self.wiring.permission_manager()?;
        let recovery_factory = self.wiring.recovery_policy_factory()?;
        let builder = builder.add_hook(ToolPermissionHook::new(
            self.wiring.terminal_io.clone(),
            catalog,
            manager,
        ));
        Ok(Self::finish_agent(
            builder,
            selected_model,
            recovery_factory,
        ))
    }

    fn configure_model(
        config: AgentConfig,
        selected_model: &SelectedModel,
    ) -> anyhow::Result<AgentConfig> {
        let mut config = config.with_model_id(selected_model.id.clone())?;
        config.model_context_length = selected_model.context_length;
        Ok(config)
    }

    async fn build_builder(
        &self,
        client: &rig::providers::openai::Client,
        model_id: String,
    ) -> anyhow::Result<AgentBuilder<CompletionModel>> {
        let provider = self.wiring.prompt_provider()?;
        let system_prompt = match self.config.tool_set {
            AgentToolSet::Chat => provider.chat_system().await?,
            AgentToolSet::Subagent => provider.subagent_system().await?,
        };
        let builder = client
            .clone()
            .completions_api()
            .agent(model_id)
            .preamble(system_prompt.as_str());
        Ok(self.config.apply_additional_params(builder))
    }

    async fn register_tools(
        &self,
        builder: AgentBuilder<CompletionModel>,
    ) -> anyhow::Result<(
        AgentBuilder<CompletionModel, WithBuilderTools>,
        ToolPermissionCatalog,
    )> {
        let mcp_tools = self.wiring.mcp_registry.select_tools().await;
        let mut catalog = ToolPermissionCatalog::default();
        let builder = match self.config.tool_set {
            AgentToolSet::Chat => self.add_chat_tools(builder, &mut catalog)?,
            AgentToolSet::Subagent => self.add_subagent_tools(builder, &mut catalog)?,
        };
        let builder = if self.config.tool_set == AgentToolSet::Chat {
            builder.add_hook(GoalCompletionHook::new(self.wiring.goal_completer()?))
        } else {
            builder
        };
        Ok((builder.mcp_tools(&mcp_tools, &mut catalog), catalog))
    }

    async fn select_model(
        config: &AgentConfig,
        client: &rig::providers::openai::Client,
        wiring: &AgentWiring,
    ) -> anyhow::Result<SelectedModel> {
        match config.model_id.clone() {
            Some(model_id) => Ok(SelectedModel::new(model_id, config.model_context_length)),
            None => {
                let models = client.models().await?;
                Ok(wiring.model_selector()?.select(&models).await?)
            }
        }
    }

    fn add_hooks<M, P>(
        &self,
        builder: AgentBuilder<M, WithBuilderTools>,
        chat_history: Arc<ChatHistory<P>>,
    ) -> AgentBuilder<M, WithBuilderTools>
    where
        M: CompletionModelTrait,
        P: HistoryPersistence,
    {
        builder
            .add_hook(InvalidResponseHook::new(self.wiring.terminal_io.clone()))
            .add_hook(HistorySyncHook::new(chat_history))
    }

    fn finish_agent(
        builder: AgentBuilder<CompletionModel, WithBuilderTools>,
        selected_model: SelectedModel,
        recovery_factory: Arc<dyn crate::infrastructure::recovery::ToolRecoveryPolicyFactory>,
    ) -> ConfiguredAgent<CompletionModel> {
        ConfiguredAgent {
            agent: builder
                .add_hook(ToolRecoveryHook::new(recovery_factory))
                .default_max_turns(MAX_AGENT_TURNS)
                .build(),
            max_context_tokens: selected_model.context_length,
            selected_model: Some(selected_model),
        }
    }
}
