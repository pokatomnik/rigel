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
        history::History,
        mcp_registry::registry::{McpRegistry, McpToolsExt},
        terminal_io::TerminalIO,
        tool_permissions::{ToolPermissionCatalog, ToolPermissionHook},
    },
    tools::{
        tool_apply_patch::ApplyPatch, tool_create_directory::CreateDirectory,
        tool_create_file::CreateFile, tool_delete_path::DeletePath, tool_fetch_url::FetchUrl,
        tool_find_paths::FindPaths, tool_list_directory::ListDirectory, tool_read_file::ReadFile,
        tool_rename_path::RenamePath, tool_run_command::RunCommand, tool_search_text::SearchText,
    },
    use_cases::chat::{
        history_sync::{ChatHistory, HistorySyncHook},
        invalid_response::InvalidResponseHook,
        tool_recovery::ToolRecoveryHook,
    },
    use_cases::model_selector::model_selector::ModelSelector,
};

const MAX_AGENT_TURNS: usize = 12;

pub(crate) struct Agent;

#[derive(Clone)]
pub(crate) struct AgentConfig {
    base_url: String,
    api_key: Option<String>,
}

impl AgentConfig {
    pub(crate) fn new(base_url: String, api_key: Option<String>) -> Self {
        Self { base_url, api_key }
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

    pub(crate) fn terminal_io(&self) -> Arc<TerminalIO> {
        self.terminal_io.clone()
    }
}

impl Agent {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) async fn new(
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
        let builder = Self::add_builtin_tools(
            client
                .completions_api()
                .agent(model_id)
                .preamble(system_prompt.as_str()),
            &mut permission_catalog,
            deps.http_client.clone(),
        )
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

    async fn add_builtin_tools<M>(
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
}
