use std::sync::Arc;

use clap::Parser;

use crate::{
    cmd::cli::Cli,
    controllers::{controller::Controller, index_controller::IndexControllerDeps},
    shared::terminal_io::TerminalIO,
};

mod cmd;
mod controllers;
mod prompts;
mod shared;
mod tools;
mod use_cases;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let terminal_io = Arc::new(TerminalIO::default());
    let index_deps = IndexControllerDeps::new(terminal_io.clone());

    if let Err(e) = cli.index.handle(index_deps).await {
        terminal_io.eprintln(e.to_string().as_str());
    }

    Ok(())
}
