use std::sync::Arc;

use reqwest::Client;
use rig::{
    Agent as RigAgent,
    agent::{AgentBuilder, WithBuilderTools},
    client::AgentClientExt,
    completion::CompletionModel as CompletionModelTrait,
    providers::openai::{self, CompletionModel},
};

use crate::{
    prompts::system::system_prompt,
    shared::{
        history::{ChatHistory, History, HistorySyncHook},
        mcp_registry::{McpRegistry, McpToolsExt},
        recovery::ToolRecoveryHook,
        response::InvalidResponseHook,
        terminal::TerminalIO,
        tool_permissions::{ToolPermissionCatalog, ToolPermissionHook},
    },
    tools::{
        tool_apply_patch::ApplyPatch, tool_create_directory::CreateDirectory,
        tool_create_file::CreateFile, tool_delete_path::DeletePath, tool_fetch_url::FetchUrl,
        tool_find_paths::FindPaths, tool_list_directory::ListDirectory, tool_read_file::ReadFile,
        tool_rename_path::RenamePath, tool_run_command::RunCommand, tool_search_text::SearchText,
    },
    use_cases::model_selector::model_selector::ModelSelector,
};

const MAX_AGENT_TURNS: usize = 12;

pub(crate) struct Agent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentToolSet {
    Subagent,
    Orchestrator,
    Chat,
}

impl AgentToolSet {
    async fn add_tools<M>(
        self,
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        http_client: Arc<Client>,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        match self {
            Self::Subagent => Agent::add_subagent_tools(builder, catalog, http_client).await,
            Self::Orchestrator => {
                Agent::add_orchestrator_tools(builder, catalog, http_client).await
            }
            Self::Chat => Agent::add_chat_tools(builder, catalog, http_client).await,
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentConfig {
    base_url: String,
    api_key: Option<String>,
    tool_set: AgentToolSet,
}

impl AgentConfig {
    pub(crate) fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            base_url,
            api_key,
            tool_set: AgentToolSet::Chat,
        }
    }

    fn with_tool_set(mut self, tool_set: AgentToolSet) -> Self {
        self.tool_set = tool_set;
        self
    }
}

#[derive(Clone)]
pub(crate) struct AgentDependencies {
    terminal_io: Arc<TerminalIO>,
    http_client: Arc<Client>,
    mcp_registry: Arc<McpRegistry>,
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
        }
    }
}

impl Agent {
    #[allow(dead_code)]
    pub(crate) async fn new_subagent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<RigAgent<CompletionModel>> {
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
    ) -> anyhow::Result<RigAgent<CompletionModel>> {
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
    ) -> anyhow::Result<RigAgent<CompletionModel>> {
        Self::build_agent(config.with_tool_set(AgentToolSet::Chat), chat_history, deps).await
    }

    async fn build_agent(
        config: AgentConfig,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<AgentDependencies>,
    ) -> anyhow::Result<RigAgent<CompletionModel>> {
        let client = Self::build_client(&config, &deps)?;
        let model_id = client.select_model(deps.terminal_io.clone()).await?;
        let system_prompt = system_prompt().await;
        println!("MCP tools loaded: {}", deps.mcp_registry.tools().len());

        let mcp_tools = deps.mcp_registry.select_tools();
        let mut permission_catalog = ToolPermissionCatalog::default();
        let builder = client
            .completions_api()
            .agent(model_id)
            .preamble(system_prompt.as_str());
        let builder = config
            .tool_set
            .add_tools(builder, &mut permission_catalog, deps.http_client.clone())
            .await?
            .mcp_tools(&mcp_tools, &mut permission_catalog);

        let builder = Self::add_hooks(builder, chat_history, deps.clone()).add_hook(
            ToolPermissionHook::new(deps.terminal_io.clone(), permission_catalog),
        );

        Ok(builder
            .add_hook(ToolRecoveryHook)
            .default_max_turns(MAX_AGENT_TURNS)
            .build())
    }

    fn add_hooks<M>(
        builder: AgentBuilder<M, WithBuilderTools>,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<AgentDependencies>,
    ) -> AgentBuilder<M, WithBuilderTools>
    where
        M: CompletionModelTrait,
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

    async fn add_chat_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        http_client: Arc<Client>,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let builder = builder
            .tool(catalog.register_builtin_tool(ApplyPatch::new().await?))
            .tool(catalog.register_builtin_tool(CreateDirectory::new().await?))
            .tool(catalog.register_builtin_tool(CreateFile::new().await?))
            .tool(catalog.register_builtin_tool(DeletePath::new().await?))
            .tool(catalog.register_builtin_tool(FindPaths::new().await?))
            .tool(catalog.register_builtin_tool(ListDirectory::new().await?))
            .tool(catalog.register_builtin_tool(RenamePath::new().await?))
            .tool(catalog.register_builtin_tool(ReadFile::new().await?))
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(catalog.register_builtin_tool(SearchText::new().await?))
            .tool(catalog.register_builtin_tool(FetchUrl::new(http_client)));
        Ok(builder)
    }

    async fn add_subagent_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        http_client: Arc<Client>,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let builder = builder
            .tool(catalog.register_builtin_tool(CreateDirectory::new().await?))
            .tool(catalog.register_builtin_tool(CreateFile::new().await?))
            .tool(catalog.register_builtin_tool(DeletePath::new().await?))
            .tool(catalog.register_builtin_tool(FindPaths::new().await?))
            .tool(catalog.register_builtin_tool(ListDirectory::new().await?))
            .tool(catalog.register_builtin_tool(ReadFile::new().await?))
            .tool(catalog.register_builtin_tool(RenamePath::new().await?));
        let builder = builder
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(catalog.register_builtin_tool(SearchText::new().await?))
            .tool(catalog.register_builtin_tool(FetchUrl::new(http_client)));
        Ok(builder)
    }

    async fn add_orchestrator_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        http_client: Arc<Client>,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let builder = builder
            .tool(catalog.register_builtin_tool(CreateDirectory::new().await?))
            .tool(catalog.register_builtin_tool(CreateFile::new().await?))
            .tool(catalog.register_builtin_tool(FindPaths::new().await?))
            .tool(catalog.register_builtin_tool(ListDirectory::new().await?))
            .tool(catalog.register_builtin_tool(ReadFile::new().await?))
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(catalog.register_builtin_tool(SearchText::new().await?))
            .tool(catalog.register_builtin_tool(FetchUrl::new(http_client)));
        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentConfig, AgentToolSet};

    #[test]
    fn agent_config_selects_each_declared_tool_set() {
        let config = AgentConfig::new("http://localhost/v1".to_string(), None);
        assert_eq!(config.tool_set, AgentToolSet::Chat);
        assert_eq!(
            config
                .clone()
                .with_tool_set(AgentToolSet::Subagent)
                .tool_set,
            AgentToolSet::Subagent
        );
        assert_eq!(
            config.with_tool_set(AgentToolSet::Orchestrator).tool_set,
            AgentToolSet::Orchestrator
        );
    }
}
