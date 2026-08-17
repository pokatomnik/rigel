use std::{sync::Arc, time::Duration};

use clap::Parser;
use reqwest::Client;

use crate::{
    cmd::{cli::Cli, commands::Commands},
    controllers::{chat_controller::chat_controller::IndexControllerDeps, controller::Controller},
    shared::{config::RigelConfig, mcp_registry::McpRegistry, terminal::TerminalIO},
};

mod cmd;
mod controllers;
mod entities;
mod prompts;
mod shared;
mod tools;
mod use_cases;

const GLOBAL_TOOL_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let terminal_io = Arc::new(TerminalIO::default());

    let result = match cli.command {
        Commands::Init(init_controller) => init_controller.handle(()).await,
        Commands::Chat(chat_controller) => {
            let config = get_config().await;
            let http_client = get_http_client().await?;
            let mcp_registry = get_mcp_registry(config).await?;
            let chat_deps =
                IndexControllerDeps::new(terminal_io.clone(), http_client, mcp_registry);
            chat_controller.handle(chat_deps).await
        }
    };

    if let Err(e) = result {
        terminal_io.eprintln(e.to_string().as_str());
    }

    Ok(())
}

async fn get_mcp_registry(config: Arc<RigelConfig>) -> anyhow::Result<Arc<McpRegistry>> {
    Ok(Arc::new(McpRegistry::from_config(config).await?))
}

async fn get_config() -> Arc<RigelConfig> {
    Arc::new(RigelConfig::from_default_path().await)
}

async fn get_http_client() -> anyhow::Result<Arc<Client>> {
    let config = reqwest::ClientBuilder::new()
        .no_proxy()
        .gzip(true)
        .brotli(true)
        .connect_timeout(GLOBAL_TOOL_TIMEOUT)
        .user_agent(include_str!("./user_agents.txt"))
        .build()?;
    Ok(Arc::new(config))
}
