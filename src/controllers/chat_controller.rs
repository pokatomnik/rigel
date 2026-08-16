use std::sync::Arc;

use clap::Args;
use rig::{Agent as RigAgent, providers::openai::CompletionModel};

use crate::{
    controllers::controller::Controller,
    shared::history::History,
    use_cases::{
        agent::{Agent, AgentConfig},
        chat::{chat::Chat, history_sync::ChatHistory},
    },
};

pub(crate) use crate::use_cases::agent::AgentDependencies as IndexControllerDeps;

#[derive(Args, Clone, Debug)]
#[clap(rename_all = "kebab-case")]
pub struct ChatController {
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

impl ChatController {
    async fn create_agent(
        &self,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<IndexControllerDeps>,
    ) -> anyhow::Result<RigAgent<CompletionModel>> {
        let config = AgentConfig::new(self.base_url.clone(), self.api_key.clone());
        Agent::new(config, chat_history, deps).await
    }
}

impl Controller<IndexControllerDeps> for ChatController {
    async fn handle(&self, deps: IndexControllerDeps) -> anyhow::Result<()> {
        let deps = Arc::new(deps);
        let (history, chat_history) = History::bootstrap().await?;
        let chat_history = Arc::new(ChatHistory::new(chat_history, Arc::new(history)));
        let agent = self
            .create_agent(chat_history.clone(), deps.clone())
            .await?;

        let controller = self.clone();
        let chat_history_clone = chat_history.clone();
        let deps_clone = deps.clone();
        let chat = Chat::from_history(agent, deps.terminal_io(), chat_history, async move || {
            controller
                .create_agent(chat_history_clone.clone(), deps_clone.clone())
                .await
        });

        deps.terminal_io()
            .eprintln("Let's chat. Type /help to get available commands");
        chat.run().await
    }
}
