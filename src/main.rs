use std::{sync::Arc, time::Duration};

use clap::Parser;

use crate::{
    cmd::cli::Cli,
    controllers::{controller::Controller, index_controller::IndexControllerDeps},
    shared::terminal_io::TerminalIO,
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
    let http_client = Arc::new(
        reqwest::ClientBuilder::new()
            .no_proxy()
            .gzip(true)
            .brotli(true)
            .connect_timeout(GLOBAL_TOOL_TIMEOUT)
            .build()?,
    );
    let index_deps = IndexControllerDeps::new(terminal_io.clone(), http_client);

    if let Err(e) = cli.index.handle(index_deps).await {
        terminal_io.eprintln(e.to_string().as_str());
    }

    Ok(())
}
