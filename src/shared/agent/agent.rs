use std::sync::Arc;

use reqwest::Client;
use rig::{
    Agent as RigAgent,
    agent::{AgentBuilder, WithBuilderTools},
    client::AgentClientExt,
    completion::CompletionModel as CompletionModelTrait,
    providers::openai::{self, CompletionModel},
};

use super::{
    agent_config::AgentConfig, agent_tool_set::AgentToolSet, tool_build_context::ToolBuildContext,
};

use crate::{
    entities::selected_model::SelectedModel,
    prompts::system::system_prompt,
    shared::{
        goal::{goal_completion_hook::GoalCompletionHook, goal_state::GoalState},
        history::{
            chat_history::ChatHistory,
            history::History,
            history_persistence::{HistoryPersistence, NonPersistentHistory},
            history_sync::HistorySyncHook,
        },
        mcp_registry::registry::{McpRegistry, McpToolsExt},
        recovery::tool_recovery::ToolRecoveryHook,
        response::invalid_response::InvalidResponseHook,
        terminal::terminal_io::TerminalIO,
        tool_permissions::{ToolPermissionCatalog, ToolPermissionHook},
    },
    tools::{
        tool_fetch_url::FetchUrl, tool_mark_goal_complete::MarkGoalComplete,
        tool_run_command::RunCommand, tool_spawn_subagent::SpawnSubagent,
    },
    use_cases::model_selector::model_selector::ModelSelector,
};

const MAX_AGENT_TURNS: usize = 12;

pub(crate) struct Agent;

pub(crate) struct ConfiguredAgent<M: CompletionModelTrait> {
    pub(crate) agent: RigAgent<M>,
    pub(crate) max_context_tokens: Option<u64>,
}

impl<M: CompletionModelTrait> ConfiguredAgent<M> {
    #[cfg(test)]
    pub(crate) fn from_agent(agent: RigAgent<M>) -> Self {
        Self {
            agent,
            max_context_tokens: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentDependencies {
    terminal_io: Arc<TerminalIO>,
    http_client: Arc<Client>,
    mcp_registry: Arc<McpRegistry>,
    goal_state: Option<Arc<GoalState>>,
}

impl AgentDependencies {
    pub(crate) fn new(
        terminal_io: Arc<TerminalIO>,
        http_client: Arc<Client>,
        mcp_registry: Arc<McpRegistry>,
    ) -> Self {
        Self {
            terminal_io,
            http_client,
            mcp_registry,
            goal_state: None,
        }
    }

    pub(crate) fn with_goal_state(mut self, goal_state: Arc<GoalState>) -> Self {
        self.goal_state = Some(goal_state);
        self
    }

    pub(crate) fn goal_state(&self) -> anyhow::Result<Arc<GoalState>> {
        self.goal_state
            .clone()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat agent"))
    }
}

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
        let client = Self::build_client(&config, &deps)?;
        let selected_model = match config.model_id.clone() {
            Some(model_id) => SelectedModel::new(model_id, config.model_context_length),
            None => client.select_model(deps.terminal_io.clone()).await?,
        };
        let mut config = config.with_model_id(selected_model.id.clone())?;
        config.model_context_length = selected_model.context_length;
        let system_prompt = system_prompt().await;

        let mcp_tools = deps.mcp_registry.select_tools().await;
        let mut permission_catalog = ToolPermissionCatalog::default();
        let builder = client
            .completions_api()
            .agent(selected_model.id)
            .preamble(system_prompt.as_str());
        let builder = config.apply_additional_params(builder);
        let tool_context = ToolBuildContext {
            config: config.clone(),
            dependencies: deps.clone(),
        };
        let builder = config
            .tool_set
            .add_tools(builder, &mut permission_catalog, tool_context)
            .await?
            .mcp_tools(&mcp_tools, &mut permission_catalog);

        let builder = Self::add_hooks(builder, chat_history, deps.clone()).add_hook(
            ToolPermissionHook::new(deps.terminal_io.clone(), permission_catalog),
        );

        Ok(ConfiguredAgent {
            agent: builder
                .add_hook(ToolRecoveryHook)
                .default_max_turns(MAX_AGENT_TURNS)
                .build(),
            max_context_tokens: selected_model.context_length,
        })
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

    fn build_client(
        config: &AgentConfig,
        deps: &AgentDependencies,
    ) -> anyhow::Result<openai::Client> {
        let api_key = match &config.api_key {
            Some(key) => key.clone(),
            None => String::new(),
        };
        Ok(openai::Client::builder()
            .api_key(api_key)
            .base_url(config.base_url.clone())
            .http_client(deps.http_client.as_ref().clone())
            .build()?)
    }

    pub(super) async fn add_chat_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        context: ToolBuildContext,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let goal_state = context.dependencies.goal_state()?;
        let builder = builder
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(
                catalog
                    .register_builtin_tool(FetchUrl::new(context.dependencies.http_client.clone())),
            )
            .tool(
                catalog.register_builtin_tool(
                    SpawnSubagent::new(
                        context.dependencies.terminal_io.clone(),
                        context.dependencies.clone(),
                        context.config,
                    )
                    .await?,
                ),
            )
            .tool(catalog.register_builtin_tool(MarkGoalComplete::new(goal_state.clone())))
            .add_hook(GoalCompletionHook::new(goal_state));
        Ok(builder)
    }

    pub(super) async fn add_subagent_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        context: ToolBuildContext,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let builder = builder
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(
                catalog
                    .register_builtin_tool(FetchUrl::new(context.dependencies.http_client.clone())),
            );
        Ok(builder)
    }

    pub(super) async fn add_orchestrator_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        context: ToolBuildContext,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let builder = builder
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(
                catalog
                    .register_builtin_tool(FetchUrl::new(context.dependencies.http_client.clone())),
            )
            .tool(
                catalog.register_builtin_tool(
                    SpawnSubagent::new(
                        context.dependencies.terminal_io.clone(),
                        context.dependencies.clone(),
                        context.config,
                    )
                    .await?,
                ),
            );
        Ok(builder)
    }
}
