use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::shared::{
    consts::{CONFIG_FILE_NAME, RIGEL_DIRECTORY},
    mcp_registry::server_config::ServerConfig,
};

#[derive(Deserialize, Serialize, Default)]
pub(crate) struct RigelConfig {
    #[serde(rename = "mcpServers")]
    #[serde(default)]
    pub(crate) servers: HashMap<String, ServerConfig>,
}

impl RigelConfig {
    pub(crate) async fn from_default_path() -> RigelConfig {
        let Some(home_path) = std::env::home_dir() else {
            return RigelConfig::default();
        };
        let default_path = home_path.join(RIGEL_DIRECTORY).join(CONFIG_FILE_NAME);
        Self::from_path(default_path).await
    }

    pub(crate) async fn from_path(path: impl Into<PathBuf>) -> RigelConfig {
        match tokio::fs::read_to_string(path.into()).await {
            Ok(content) => Self::parse_config(content.as_str()),
            Err(_) => Default::default(),
        }
    }

    pub(crate) fn parse_config(content: &str) -> RigelConfig {
        toml::from_str(content).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unlimited_mixed_servers() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
            [mcpServers.local]
            type = "stdio"
            command = "node"
            args = ["server.js"]

            [mcpServers.remote]
            type = "http"
            url = "https://example.com/mcp"
            "#,
        );

        assert_eq!(config.servers.len(), 2);
        assert!(matches!(config.servers["local"], ServerConfig::Stdio(_)));
        assert!(matches!(config.servers["remote"], ServerConfig::Http(_)));
        Ok(())
    }
}
