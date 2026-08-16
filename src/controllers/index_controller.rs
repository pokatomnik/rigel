use std::sync::Arc;

use crate::{
    controllers::controller::Controller,
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
    use_cases::{
        chat::{
            chat::Chat,
            history_sync::{ChatHistory, HistorySyncHook},
            invalid_response::InvalidResponseHook,
            tool_recovery::ToolRecoveryHook,
        },
        model_selector::model_selector::ModelSelector,
    },
};
use clap::Args;
use reqwest::Client;
use rig::{
    Agent,
    agent::{AgentBuilder, WithBuilderTools},
    client::AgentClientExt,
    completion::CompletionModel as CompletionModelTrait,
    providers::openai::{self, CompletionModel},
};

const MAX_AGENT_TURNS: usize = 12;

#[derive(Args, Clone, Debug)]
#[clap(rename_all = "kebab-case")]
pub struct IndexController {
    #[arg(
        long = "base-url",
        short = 'u',
        required = true,
        help = "OpenAI-compatible API base URL"
    )]
    base_url: String,

    #[arg(long = "api-key", short = 'k', help = "API key for authentication")]
    api_key: Option<String>,
}

impl IndexController {
    async fn build_agent(
        &self,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<IndexControllerDeps>,
    ) -> anyhow::Result<Agent<CompletionModel>> {
        let api_key = match &self.api_key {
            Some(key) => key.clone(),
            None => String::new(),
        };
        let client = openai::Client::builder()
            .api_key(api_key)
            .base_url(self.base_url.clone())
            .http_client(deps.http_client.as_ref().clone())
            .build()?;
        let model_id = client.select_model(deps.terminal_io.clone()).await?;
        let client = client.completions_api();

        let system_prompt = system_prompt().await;

        println!("MCP tools loaded: {}", &deps.mcp_registry.tools().len());

        let mcp_tools = deps.mcp_registry.select_tools();
        let mut permission_catalog = ToolPermissionCatalog::default();

        let builder = Self::add_builtin_tools(
            client.agent(model_id).preamble(system_prompt.as_str()),
            &mut permission_catalog,
            deps.http_client.clone(),
        )
        .await?
        .mcp_tools(&mcp_tools, &mut permission_catalog);

        let agent = builder
            .add_hook(InvalidResponseHook::new(deps.terminal_io.clone()))
            .add_hook(HistorySyncHook::new(chat_history.clone()))
            .add_hook(ToolPermissionHook::new(
                deps.terminal_io.clone(),
                permission_catalog,
            ))
            .add_hook(ToolRecoveryHook)
            .default_max_turns(MAX_AGENT_TURNS)
            .build();

        Ok(agent)
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

#[derive(Clone)]
pub(crate) struct IndexControllerDeps {
    terminal_io: Arc<TerminalIO>,
    http_client: Arc<Client>,
    mcp_registry: Arc<McpRegistry>,
}

impl IndexControllerDeps {
    pub fn new(
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

impl Controller<IndexControllerDeps> for IndexController {
    async fn handle(&self, deps: IndexControllerDeps) -> anyhow::Result<()> {
        let deps = Arc::new(deps);
        let (history, chat_history) = History::bootstrap().await?;
        let chat_history = Arc::new(ChatHistory::new(chat_history, Arc::new(history)));
        let agent = self.build_agent(chat_history.clone(), deps.clone()).await?;

        let controller = self.clone();
        let ch_clone = chat_history.clone();
        let deps_clone = deps.clone();
        let create_new_agent = async move || {
            controller
                .build_agent(ch_clone.clone(), deps_clone.clone())
                .await
        };

        let chat = Chat::from_history(
            agent,
            deps.terminal_io.clone(),
            chat_history,
            create_new_agent,
        );

        deps.terminal_io
            .clone()
            .eprintln("Let's chat. Type /help to get available commands");

        chat.run().await?;

        Ok(())
    }
}
