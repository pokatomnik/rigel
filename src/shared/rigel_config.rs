use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::shared::{
    consts::{CONFIG_FILE_NAME, RIGEL_DIRECTORY},
    mcp_registry::server_config::ServerConfig,
};

#[derive(Deserialize, Serialize, Default)]
pub(crate) struct RigelConfig {
    #[serde(rename = "baseUrl")]
    #[serde(default = "RigelConfig::default_base_url")]
    pub(crate) base_url: String,

    #[serde(rename = "envKey")]
    pub(crate) env_key: Option<String>,

    #[serde(rename = "mcpServers")]
    #[serde(default)]
    pub(crate) servers: HashMap<String, ServerConfig>,
}

impl RigelConfig {
    fn default_base_url() -> String {
        "http://127.0.0.1:1234".to_owned()
    }

    pub(crate) async fn from_default_path() -> anyhow::Result<RigelConfig> {
        let home_path = std::env::home_dir()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Rigel configuration file could not be located because the user home directory is unavailable. Run `rigel init` to perform basic initialization."
                )
            })?;
        let default_path = home_path.join(RIGEL_DIRECTORY).join(CONFIG_FILE_NAME);
        Self::from_path(default_path).await
    }

    pub(crate) async fn from_path(path: impl Into<PathBuf>) -> anyhow::Result<RigelConfig> {
        let path = path.into();
        match tokio::fs::read_to_string(path.as_path()).await {
            Ok(content) => Self::parse_config(content.as_str()),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Err(Self::missing_file_error(path.as_path()))
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to read Rigel configuration file '{}'",
                    path.display()
                )
            }),
        }
    }

    fn missing_file_error(path: &Path) -> anyhow::Error {
        anyhow::anyhow!(
            "Rigel configuration file '{}' is missing. Run `rigel init` to perform basic initialization.",
            path.display()
        )
    }

    fn parse_config(content: &str) -> anyhow::Result<RigelConfig> {
        Ok(toml::from_str(content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_default_base_url_when_missing() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config("")?;

        assert_eq!(config.base_url, "http://127.0.0.1:1234");
        Ok(())
    }
}
