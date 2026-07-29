use std::sync::Arc;

use crate::{
    controllers::controller::Controller,
    prompts::system::SYSTEM_PROMPT,
    shared::terminal_io::TerminalIO,
    tools::add::Adder,
    use_cases::{chat::chat::Chat, model_selector::model_selector::ModelSelector},
};
use clap::Args;
use rig::{
    client::{AgentClientExt, ModelListingClient},
    message::Message,
    providers::ollama,
};

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

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

        let agent = client
            .agent(model_id)
            .preamble(SYSTEM_PROMPT)
            .tool(Adder)
            .default_max_turns(usize::MAX)
            .build();

        let chat = Chat::new(agent, deps.terminal_io.clone(), chat_history);

        chat.run().await?;

        Ok(())
    }
}
