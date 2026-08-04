use std::sync::Arc;

use crate::{
    controllers::controller::Controller,
    prompts::system::system_prompt,
    shared::{history::History, terminal_io::TerminalIO},
    tools::{
        add::Adder, apply_patch::ApplyPatch, create_directory::CreateDirectory,
        create_file::CreateFile, delete_directory::DeleteDirectory, delete_file::DeleteFile,
        fetch_webpage::FetchWebpage, find_paths::FindPaths, list_directory::ListDirectory,
        move_path::MovePath, read_file::ReadFile, run_in_terminal::RunInTerminal,
        search_text::SearchText, stat::Stat,
    },
    use_cases::{
        chat::{
            chat::Chat,
            history_sync::{ChatHistory, HistorySyncHook},
            tool_recovery::ToolRecoveryHook,
        },
        model_selector::model_selector::ModelSelector,
    },
};
use clap::Args;
use reqwest::Client;
use rig::{
    Agent,
    client::{AgentClientExt, ModelListingClient},
    providers::ollama::{self, CompletionModel},
};

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const MAX_AGENT_TURNS: usize = 12;

#[derive(Args, Clone, Debug)]
#[clap(rename_all = "kebab-case")]
pub struct IndexController {
    #[arg(long = "base-url", short = 'b', default_value_t = DEFAULT_OLLAMA_URL.to_owned(), help = "Ollama server base URL")]
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
        let client = ollama::Client::builder()
            .api_key(self.api_key.clone().unwrap_or_default())
            .base_url(self.base_url.clone())
            .build()?;
        let models = client.list_models().await?;
        let model_id = models.select_model_sync(deps.terminal_io.clone())?;

        let apply_patch = ApplyPatch::new().await?;
        let create_directory = CreateDirectory::new().await?;
        let create_file = CreateFile::new().await?;
        let delete_directory = DeleteDirectory::new(deps.terminal_io.clone()).await?;
        let delete_file = DeleteFile::new(deps.terminal_io.clone()).await?;
        let find_paths = FindPaths::new().await?;
        let list_directory = ListDirectory::new().await?;
        let move_path = MovePath::new(deps.terminal_io.clone()).await?;
        let read_file = ReadFile::new().await?;
        let run_in_terminal = RunInTerminal::new(deps.terminal_io.clone()).await?;
        let search_text = SearchText::new().await?;
        let stat = Stat::new().await?;
        let fetch_webpage = FetchWebpage::new(deps.http_client.clone());
        let system_prompt = system_prompt().await;

        let agent = client
            .agent(model_id)
            .preamble(system_prompt.as_str())
            .add_hook(HistorySyncHook::new(chat_history.clone()))
            .add_hook(ToolRecoveryHook)
            .tool(Adder)
            .tool(apply_patch)
            .tool(create_directory)
            .tool(create_file)
            .tool(delete_directory)
            .tool(delete_file)
            .tool(find_paths)
            .tool(list_directory)
            .tool(move_path)
            .tool(read_file)
            .tool(run_in_terminal)
            .tool(search_text)
            .tool(stat)
            .tool(fetch_webpage)
            .default_max_turns(MAX_AGENT_TURNS)
            .build();

        Ok(agent)
    }
}

#[derive(Clone)]
pub(crate) struct IndexControllerDeps {
    terminal_io: Arc<TerminalIO>,
    http_client: Arc<Client>,
}

impl IndexControllerDeps {
    pub fn new(terminal_io: Arc<TerminalIO>, http_client: Arc<Client>) -> Self {
        Self {
            terminal_io,
            http_client,
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
