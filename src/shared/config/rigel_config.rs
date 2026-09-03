use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::shared::{
    config::consts::{CONFIG_FILE_NAME, RIGEL_DIRECTORY},
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

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    models: HashMap<String, ModelConfig>,
}

#[derive(Deserialize, Serialize, Default)]
struct ModelConfig {
    #[serde(default)]
    params: toml::Table,
}

const TRANSPORT_OWNED_PARAMS: [&str; 7] = [
    "model",
    "messages",
    "tools",
    "tool_choice",
    "temperature",
    "max_tokens",
    "stream",
];

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
        let config: RigelConfig = toml::from_str(content)?;
        config.validate_model_params()?;
        Ok(config)
    }

    fn validate_model_params(&self) -> anyhow::Result<()> {
        for (model_id, model) in &self.models {
            Self::validate_param_names(model_id, &model.params)?;
            serde_json::to_value(&model.params)?;
        }
        Ok(())
    }

    fn validate_param_names(model_id: &str, params: &toml::Table) -> anyhow::Result<()> {
        if let Some(name) = params
            .keys()
            .find(|name| TRANSPORT_OWNED_PARAMS.contains(&name.as_str()))
        {
            anyhow::bail!(
                "model `{model_id}` params cannot override transport-owned field `{name}`"
            );
        }
        Ok(())
    }

    pub(crate) fn model_params(&self, model_id: &str) -> anyhow::Result<Option<Value>> {
        let Some(model) = self.models.get(model_id) else {
            return Ok(None);
        };
        if model.params.is_empty() {
            return Ok(None);
        }
        Self::validate_param_names(model_id, &model.params)?;
        Ok(Some(serde_json::to_value(&model.params)?))
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
        assert_eq!(config.model_params("model-a")?, None);
        Ok(())
    }

    #[test]
    fn exact_model_id_returns_provider_parameters_without_conversion() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[models."gpt-5.5".params]
reasoning_effort = "high"
temperature_hint = 0.25
stop = ["END", "DONE"]

[models."gpt-5.5".params.reasoning]
effort = "high"
enabled = true
"#,
        )?;

        assert_eq!(
            config.model_params("gpt-5.5")?,
            Some(serde_json::json!({
                "reasoning_effort": "high",
                "temperature_hint": 0.25,
                "stop": ["END", "DONE"],
                "reasoning": {"effort": "high", "enabled": true}
            }))
        );
        Ok(())
    }

    #[test]
    fn model_lookup_is_exact_and_case_sensitive() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[models."Model-A".params]
think = true
"#,
        )?;

        assert!(config.model_params("model-a")?.is_none());
        assert!(config.model_params("Model-A")?.is_some());
        assert!(config.model_params("Model-A-extra")?.is_none());
        Ok(())
    }

    #[test]
    fn missing_and_empty_parameters_do_not_add_request_parameters() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[models."empty".params]
"#,
        )?;

        assert!(config.model_params("empty")?.is_none());
        assert!(config.model_params("missing")?.is_none());
        Ok(())
    }

    #[test]
    fn transport_owned_parameters_are_rejected() {
        for name in TRANSPORT_OWNED_PARAMS {
            let source = format!("[models.unsafe.params]\n{name} = \"configured\"\n");
            assert!(
                RigelConfig::parse_config(source.as_str()).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn models_and_params_must_be_tables() {
        assert!(RigelConfig::parse_config("models = []").is_err());
        assert!(RigelConfig::parse_config("[models.test]\nparams = true").is_err());
    }
}
