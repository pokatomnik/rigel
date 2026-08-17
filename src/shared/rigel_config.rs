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
    base_url: String,

    #[serde(rename = "envKey")]
    env_key: Option<String>,

    #[serde(rename = "mcpServers")]
    #[serde(default)]
    servers: HashMap<String, ServerConfig>,
}

impl RigelConfig {
    fn default_base_url() -> String {
        "http://127.0.0.1:1234/v1".to_owned()
    }

    /// Loads the configuration from the default path, falling back to defaults on any error.
    pub(crate) async fn from_default_path() -> RigelConfig {
        let Ok(default_path) = Self::default_config_path() else {
            return Self::default();
        };
        Self::from_path(default_path).await.unwrap_or_default()
    }

    /// Loads and parses the configuration from the given path, returning errors to the caller.
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

    /// Serializes the configuration and writes it to the default path.
    pub(crate) async fn write_default_path(&self, confirm_override: bool) -> anyhow::Result<()> {
        if !Self::should_write_default_path(confirm_override).await? {
            return Ok(());
        }
        let path = Self::default_config_path()?;
        let content =
            toml::to_string_pretty(self).context("failed to serialize Rigel configuration")?;
        let directory = path
            .parent()
            .context("Rigel configuration path has no parent directory")?;

        tokio::fs::create_dir_all(directory)
            .await
            .with_context(|| {
                format!(
                    "failed to create Rigel configuration directory '{}'",
                    directory.display()
                )
            })?;
        tokio::fs::write(path.as_path(), content)
            .await
            .with_context(|| {
                format!(
                    "failed to write Rigel configuration file '{}'",
                    path.display()
                )
            })
    }

    async fn should_write_default_path(confirm_override: bool) -> anyhow::Result<bool> {
        let path = Self::default_config_path()?;
        if !confirm_override || !tokio::fs::try_exists(path.as_path()).await? {
            return Ok(true);
        }

        Ok(dialoguer::Confirm::new()
            .with_prompt(format!(
                "Overwrite Rigel configuration file '{}'?",
                path.as_path().display()
            ))
            .report(false)
            .interact()?)
    }

    fn default_config_path() -> anyhow::Result<PathBuf> {
        let home_path = std::env::home_dir().ok_or_else(|| {
            anyhow::anyhow!(
                "Rigel configuration file could not be located because the user home directory is unavailable. Run `rigel init` to perform basic initialization."
            )
        })?;
        Ok(home_path.join(RIGEL_DIRECTORY).join(CONFIG_FILE_NAME))
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

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    pub fn api_key(&self) -> Option<String> {
        let Some(ref api_env_key) = self.env_key else {
            return None;
        };
        let Ok(api_key) = std::env::var(api_env_key.as_str()) else {
            return None;
        };
        Some(api_key)
    }

    pub fn mcp_servers(&self) -> &HashMap<String, ServerConfig> {
        &self.servers
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_env_key(mut self, env_key: impl Into<String>) -> Self {
        self.env_key = Some(env_key.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_default_base_url_when_missing() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config("")?;

        assert_eq!(config.base_url, "http://127.0.0.1:1234/v1");
        Ok(())
    }
}
