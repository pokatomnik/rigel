use std::{sync::Arc, time::Duration};

use clap::Parser;

use crate::{
    cmd::{cli::Cli, commands::Commands},
    controllers::{chat_controller::IndexControllerDeps, controller::Controller},
    shared::{
        mcp_registry::registry::McpRegistry, rigel_config::RigelConfig, terminal_io::TerminalIO,
    },
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
    let config_result = RigelConfig::from_default_path().await;

    let config = match config_result {
        Ok(config) => config,
        Err(e) => {
            anyhow::bail!(e);
        }
    };

    let config = Arc::new(config);

    let terminal_io = Arc::new(TerminalIO::default());
    let http_client = Arc::new(
        reqwest::ClientBuilder::new()
            .no_proxy()
            .gzip(true)
            .brotli(true)
            .connect_timeout(GLOBAL_TOOL_TIMEOUT)
            .user_agent(include_str!("./user_agents.txt"))
            .build()?,
    );
    let mcp_registry = Arc::new(McpRegistry::from_config(config).await?);
    let chat_deps = IndexControllerDeps::new(terminal_io.clone(), http_client, mcp_registry);

    let result = match cli.command {
        Commands::Chat(chat_controller) => chat_controller.handle(chat_deps).await,
    };

    if let Err(e) = result {
        terminal_io.eprintln(e.to_string().as_str());
    }

    Ok(())
}
