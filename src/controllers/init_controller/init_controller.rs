use clap::Args;

use crate::{controllers::controller::Controller, shared::config::rigel_config::RigelConfig};

#[derive(Args, Clone, Debug)]
#[clap(rename_all = "kebab-case")]
pub struct InitController {
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

impl Controller<()> for InitController {
    async fn handle(&self, _: ()) -> anyhow::Result<()> {
        let mut config = RigelConfig::default().with_base_url(self.base_url.clone());
        if let Some(ref api_key_env) = self.api_key_env {
            config = config.with_env_key(api_key_env.clone());
        }
        config.write_default_path(true).await
    }
}
