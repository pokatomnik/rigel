use std::sync::Arc;

use crate::{
    controllers::controller::Controller,
    prompts::system::system_prompt,
    shared::terminal_io::TerminalIO,
    tools::{
        add::Adder, apply_patch::ApplyPatch, create_directory::CreateDirectory,
        create_file::CreateFile, delete_directory::DeleteDirectory, find_paths::FindPaths,
        list_directory::ListDirectory, move_path::MovePath, read_file::ReadFile,
        search_text::SearchText, stat::Stat,
    },
    use_cases::{
        chat::{chat::Chat, tool_recovery::ToolRecoveryHook},
        model_selector::model_selector::ModelSelector,
    },
};
use clap::Args;
use rig::{
    client::{AgentClientExt, ModelListingClient},
    message::Message,
    providers::ollama,
};

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const MAX_AGENT_TURNS: usize = 12;

#[derive(Args, Debug)]
#[clap(rename_all = "kebab-case")]
pub struct IndexController {
    #[arg(long = "base-url", short = 'b', default_value_t = DEFAULT_OLLAMA_URL.to_owned(), help = "Ollama server base URL")]
    base_url: String,

    #[arg(long = "api-key", short = 'k', help = "API key for authentication")]
    api_key: Option<String>,
}

#[derive(Clone)]
pub(crate) struct IndexControllerDeps {
    terminal_io: Arc<TerminalIO>,
}

impl IndexControllerDeps {
    pub fn new(terminal_io: Arc<TerminalIO>) -> Self {
        Self { terminal_io }
    }
}

impl Controller<IndexControllerDeps> for IndexController {
    async fn handle(&self, deps: IndexControllerDeps) -> anyhow::Result<()> {
        let client = ollama::Client::builder()
            .api_key(self.api_key.clone().unwrap_or_default())
            .base_url(self.base_url.clone())
            .build()?;

        let models = client.list_models().await?;

        let model_id = models.select_model_sync(deps.terminal_io.clone())?;

        // TODO restore from file
        let chat_history = Vec::<Message>::new();
        let apply_patch = ApplyPatch::new().await?;
        let create_directory = CreateDirectory::new().await?;
        let create_file = CreateFile::new().await?;
        let delete_directory = DeleteDirectory::new().await?;
        let find_paths = FindPaths::new().await?;
        let list_directory = ListDirectory::new().await?;
        let move_path = MovePath::new().await?;
        let read_file = ReadFile::new().await?;
        let search_text = SearchText::new().await?;
        let stat = Stat::new().await?;

        let agent = client
            .agent(model_id)
            .preamble(system_prompt())
            .add_hook(ToolRecoveryHook)
            .tool(Adder)
            .tool(apply_patch)
            .tool(create_directory)
            .tool(create_file)
            .tool(delete_directory)
            .tool(find_paths)
            .tool(list_directory)
            .tool(move_path)
            .tool(read_file)
            .tool(search_text)
            .tool(stat)
            .default_max_turns(MAX_AGENT_TURNS)
            .build();

        let chat = Chat::new(agent, deps.terminal_io.clone(), chat_history);

        chat.run().await?;

        Ok(())
    }
}
