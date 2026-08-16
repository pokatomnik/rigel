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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_required() {
        assert!(Cli::try_parse_from(["rigel"]).is_err());
    }

    #[test]
    fn api_key_is_optional() {
        assert!(Cli::try_parse_from(["rigel", "--base-url", "http://localhost/v1"]).is_ok());
    }
}
