use clap::Parser;

use crate::cmd::commands::Commands;

#[derive(Parser)]
#[command(name = "Rigel")]
#[command(about = "Cozy LLM agent")]
#[command(version)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}
