use std::{path::PathBuf, sync::Arc};

use clap::Args;
use reqwest::Client;
use rig::{Agent as RigAgent, providers::openai::CompletionModel};

use crate::{
    controllers::controller::Controller,
    shared::{
        agent::{Agent, AgentConfig, AgentDependencies},
        config::RigelConfig,
        history::{ChatHistory, History},
        mcp_registry::McpRegistry,
        terminal::TerminalIO,
    },
    use_cases::chat::chat::Chat,
};

#[derive(Clone)]
pub(crate) struct IndexControllerDeps {
    terminal_io: Arc<TerminalIO>,
    http_client: Arc<Client>,
    mcp_registry: Arc<McpRegistry>,
}

impl IndexControllerDeps {
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

    fn agent_dependencies(&self) -> AgentDependencies {
        AgentDependencies::new(
            self.terminal_io.clone(),
            self.http_client.clone(),
            self.mcp_registry.clone(),
        )
    }

    fn terminal_io(&self) -> Arc<TerminalIO> {
        self.terminal_io.clone()
    }
}

#[derive(Args, Clone, Debug)]
#[clap(rename_all = "kebab-case")]
pub struct ChatController {
    #[arg(
        long = "profile",
        short = 'p',
        required = false,
        help = "Path to configuration file"
    )]
    profile: Option<PathBuf>,
}

impl ChatController {
    async fn read_config(&self) -> anyhow::Result<RigelConfig> {
        match &self.profile {
            Some(path) => RigelConfig::from_path(path).await,
            None => Ok(RigelConfig::from_default_path().await),
        }
    }

    async fn create_agent(
        &self,
        chat_history: Arc<ChatHistory<Arc<History>>>,
        deps: Arc<IndexControllerDeps>,
    ) -> anyhow::Result<RigAgent<CompletionModel>> {
        let config = Arc::new(self.read_config().await?);
        let config = AgentConfig::new(config);
        Agent::new_chat_agent(config, chat_history, Arc::new(deps.agent_dependencies())).await
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
            .eprintln(include_str!("./welcome_message.txt"));
        chat.run().await
    }
}
