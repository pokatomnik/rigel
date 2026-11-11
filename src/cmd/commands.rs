use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

/// All CLI subcommands parsed before application request mapping.
#[derive(Subcommand, Clone, Debug)]
pub(crate) enum Commands {
    /// Create or overwrite the default configuration.
    Init(InitArgs),
    /// Start an interactive chat session.
    Chat(ChatArgs),
}

/// Transport-only arguments for `rigel init`.
#[derive(Args, Clone, Debug, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub(crate) struct InitArgs {
    #[arg(
        long = "base-url",
        short = 'u',
        required = false,
        default_value = "http://127.0.0.1:1234/v1",
        help = "OpenAI-compatible API base URL"
    )]
    base_url: String,
    #[arg(
        long = "api-key-env",
        short = 'k',
        help = "API key env to take env from"
    )]
    api_key_env: Option<String>,
}

impl InitArgs {
    /// Returns the endpoint supplied by the CLI, including its stable default.
    pub(crate) fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Returns the optional environment variable name supplied by the CLI.
    pub(crate) fn api_key_env(&self) -> Option<&str> {
        self.api_key_env.as_deref()
    }
}

/// Transport-only arguments for `rigel chat`.
#[derive(Args, Clone, Debug, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub(crate) struct ChatArgs {
    #[arg(
        long = "profile",
        short = 'p',
        required = false,
        help = "Path to configuration file"
    )]
    profile: Option<PathBuf>,
}

impl ChatArgs {
    /// Returns the explicit profile path, if the user supplied one.
    pub(crate) fn profile_path(&self) -> Option<&Path> {
        self.profile.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Commands;
    use crate::cmd::cli::Cli;

    #[test]
    fn init_defaults_are_mapped_without_a_controller_dependency() -> anyhow::Result<()> {
        let cli = Cli::try_parse_from(["rigel", "init"])?;
        let Cli {
            command: Commands::Init(args),
        } = cli
        else {
            return Err(anyhow::anyhow!("init arguments were not parsed"));
        };

        assert_eq!(args.base_url(), "http://127.0.0.1:1234/v1");
        assert_eq!(args.api_key_env(), None);
        Ok(())
    }

    #[test]
    fn short_and_long_chat_profile_flags_are_equivalent() -> anyhow::Result<()> {
        let short = Cli::try_parse_from(["rigel", "chat", "-p", "profile.toml"]);
        let long = Cli::try_parse_from(["rigel", "chat", "--profile", "profile.toml"]);
        let short = short?;
        let long = long?;
        let Cli {
            command: Commands::Chat(short),
        } = short
        else {
            return Err(anyhow::anyhow!("short chat arguments were not parsed"));
        };
        let Cli {
            command: Commands::Chat(long),
        } = long
        else {
            return Err(anyhow::anyhow!("long chat arguments were not parsed"));
        };

        assert_eq!(short, long);
        Ok(())
    }
}
