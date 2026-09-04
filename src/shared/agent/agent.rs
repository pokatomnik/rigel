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
    dependencies::{AgentDependencies, ConfiguredAgent},
    tool_build_context::ToolBuildContext,
};
use crate::{
    entities::selected_model::SelectedModel,
    prompts::system::system_prompt,
    shared::{
        history::{
            chat_history::ChatHistory,
            history::History,
            history_persistence::{HistoryPersistence, NonPersistentHistory},
            history_sync::HistorySyncHook,
        },
        mcp_registry::registry::McpToolsExt,
        recovery::tool_recovery::ToolRecoveryHook,
        response::invalid_response::InvalidResponseHook,
        tool_permissions::{
            catalog::ToolPermissionCatalog, hook::ToolPermissionHook,
            manager::ToolPermissionManager,
        },
    },
    use_cases::model_selector::model_selector::ModelSelector,
};

const MAX_AGENT_TURNS: usize = 12;

pub(crate) struct Agent;

impl Agent {
    #[allow(dead_code)]
    pub(crate) async fn new_subagent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<NonPersistentHistory>>,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>> {
        Self::build_agent(
            config.with_tool_set(AgentToolSet::Subagent),
            chat_history,
            deps,
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn new_orchestrator_agent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>> {
        Self::build_agent(
            config.with_tool_set(AgentToolSet::Orchestrator),
            chat_history,
            deps,
        )
        .await
    }

    pub(crate) async fn new_chat_agent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>> {
        Self::build_agent(config.with_tool_set(AgentToolSet::Chat), chat_history, deps).await
    }

    async fn build_agent<P>(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<P>>,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<ConfiguredAgent<CompletionModel>>
    where
        P: HistoryPersistence,
    {
        let permission_paths = deps.tool_permission_paths()?;
        let client = Self::build_client(&config, &deps)?;
        let selected_model = Self::select_model(&config, &client, &deps).await?;
        let config = Self::configure_model(config, &selected_model)?;
        let builder = Self::build_builder(&client, &config, selected_model.id.clone()).await;
        let (builder, catalog) = Self::register_tools(builder, &config, deps.clone()).await?;
        let manager = ToolPermissionManager::new(permission_paths);
        let builder = Self::add_hooks(builder, chat_history, deps.clone()).add_hook(
            ToolPermissionHook::new(deps.terminal_io.clone(), catalog, manager),
        );
        Ok(Self::finish_agent(builder, selected_model.context_length))
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
        client: &rig::providers::openai::Client,
        config: &AgentConfig,
        model_id: String,
    ) -> AgentBuilder<CompletionModel> {
        let system_prompt = system_prompt().await;
        let builder = client
            .clone()
            .completions_api()
            .agent(model_id)
            .preamble(system_prompt.as_str());
        config.apply_additional_params(builder)
    }

    async fn register_tools(
        builder: AgentBuilder<CompletionModel>,
        config: &AgentConfig,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<(
        AgentBuilder<CompletionModel, WithBuilderTools>,
        ToolPermissionCatalog,
    )> {
        let mcp_tools = deps.mcp_registry.select_tools().await;
        let mut catalog = ToolPermissionCatalog::default();
        let context = ToolBuildContext {
            config: config.clone(),
            dependencies: deps,
        };
        let builder = config
            .tool_set
            .add_tools(builder, &mut catalog, context)
            .await?
            .mcp_tools(&mcp_tools, &mut catalog);
        Ok((builder, catalog))
    }

    async fn select_model(
        config: &AgentConfig,
        client: &rig::providers::openai::Client,
        deps: &AgentDependencies,
    ) -> anyhow::Result<SelectedModel> {
        match config.model_id.clone() {
            Some(model_id) => Ok(SelectedModel::new(model_id, config.model_context_length)),
            None => Ok(client.select_model(deps.terminal_io.clone()).await?),
        }
    }

    fn add_hooks<M, P>(
        builder: AgentBuilder<M, WithBuilderTools>,
        chat_history: Arc<ChatHistory<P>>,
        deps: Arc<AgentDependencies>,
    ) -> AgentBuilder<M, WithBuilderTools>
    where
        M: CompletionModelTrait,
        P: HistoryPersistence,
    {
        builder
            .add_hook(InvalidResponseHook::new(deps.terminal_io.clone()))
            .add_hook(HistorySyncHook::new(chat_history))
    }

    fn finish_agent(
        builder: AgentBuilder<CompletionModel, WithBuilderTools>,
        max_context_tokens: Option<u64>,
    ) -> ConfiguredAgent<CompletionModel> {
        ConfiguredAgent {
            agent: builder
                .add_hook(ToolRecoveryHook)
                .default_max_turns(MAX_AGENT_TURNS)
                .build(),
            max_context_tokens,
        }
    }
}
