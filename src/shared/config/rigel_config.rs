use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::shared::{
    config::{model_config::ModelConfig, policies::PolicyConfig},
    mcp_registry::server_config::ServerConfig,
};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:1234/v1";

#[derive(Clone)]
pub(crate) struct ToolPermissionPaths {
    pub(crate) global_config_path: PathBuf,
    pub(crate) project_config_path: PathBuf,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct RigelConfig {
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,

    #[serde(rename = "envKey")]
    env_key: Option<String>,

    #[serde(rename = "mcpServers")]
    #[serde(default)]
    servers: HashMap<String, ServerConfig>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    models: HashMap<String, ModelConfig>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    policies: HashMap<String, PolicyConfig>,
}

impl Default for RigelConfig {
    fn default() -> Self {
        Self {
            base_url: Some(Self::default_base_url()),
            env_key: None,
            servers: HashMap::new(),
            models: HashMap::new(),
            policies: HashMap::new(),
        }
    }
}

impl RigelConfig {
    fn default_base_url() -> String {
        DEFAULT_BASE_URL.to_owned()
    }

    pub(crate) async fn from_profile(profile: Option<&Path>) -> anyhow::Result<RigelConfig> {
        let paths = Self::config_paths(profile)?;
        Self::from_layer_paths(
            paths.global_config_path,
            paths.project_config_path,
            profile.is_some(),
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn from_default_path() -> RigelConfig {
        Self::from_profile(None).await.unwrap_or_default()
    }

    pub(crate) fn config_paths(profile: Option<&Path>) -> anyhow::Result<ToolPermissionPaths> {
        let global_path = match profile {
            Some(path) => path.to_path_buf(),
            None => Self::default_config_path()?,
        };
        let project_path = Self::project_config_path()?;
        Ok(ToolPermissionPaths {
            global_config_path: global_path,
            project_config_path: project_path,
        })
    }

    pub(crate) async fn write_default_path(&self, confirm_override: bool) -> anyhow::Result<()> {
        let path = Self::default_config_path()?;
        if !Self::should_write_path(path.as_path(), confirm_override).await? {
            return Ok(());
        }
        let content = Self::serialize(self)?;
        Self::write_path(path, content).await
    }

    async fn should_write_path(path: &Path, confirm_override: bool) -> anyhow::Result<bool> {
        if !confirm_override || !tokio::fs::try_exists(path).await? {
            return Ok(true);
        }
        Ok(dialoguer::Confirm::new()
            .with_prompt(format!(
                "Overwrite Rigel configuration file '{}'?",
                path.display()
            ))
            .report(false)
            .interact()?)
    }

    fn serialize(config: &RigelConfig) -> anyhow::Result<String> {
        toml::to_string_pretty(config).context("failed to serialize Rigel configuration")
    }

    async fn write_path(path: PathBuf, content: String) -> anyhow::Result<()> {
        let directory = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Rigel configuration path has no parent directory"))?;
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

    async fn from_layer_paths(
        global_path: PathBuf,
        project_path: PathBuf,
        explicit_global: bool,
    ) -> anyhow::Result<RigelConfig> {
        let global = Self::read_layer(global_path, explicit_global).await?;
        let project = Self::read_layer(project_path, false).await.ok().flatten();
        Ok(Self::merge_optional(global, project))
    }

    pub(crate) async fn from_path(path: impl Into<PathBuf>) -> anyhow::Result<RigelConfig> {
        let path = path.into();
        match tokio::fs::read_to_string(path.as_path()).await {
            Ok(content) => Self::parse_config(content.as_str()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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

    fn default_config_path() -> anyhow::Result<PathBuf> {
        let home_path = std::env::home_dir().ok_or_else(|| {
            anyhow::anyhow!(
                "Rigel configuration file could not be located because the user home directory is unavailable. Run `rigel init` to perform basic initialization."
            )
        })?;
        Ok(home_path
            .join(crate::shared::config::consts::RIGEL_DIRECTORY)
            .join(crate::shared::config::consts::CONFIG_FILE_NAME))
    }

    fn project_config_path() -> anyhow::Result<PathBuf> {
        Ok(std::env::current_dir()?
            .join(crate::shared::config::consts::RIGEL_DIRECTORY)
            .join(crate::shared::config::consts::CONFIG_FILE_NAME))
    }

    fn missing_file_error(path: &Path) -> anyhow::Error {
        anyhow::anyhow!(
            "Rigel configuration file '{}' is missing. Run `rigel init` to perform basic initialization.",
            path.display()
        )
    }

    pub(super) fn parse_config(content: &str) -> anyhow::Result<RigelConfig> {
        let config: RigelConfig = toml::from_str(content)?;
        config.validate_model_params()?;
        Ok(config)
    }

    pub(super) async fn read_layer(
        path: PathBuf,
        required: bool,
    ) -> anyhow::Result<Option<RigelConfig>> {
        match Self::from_path(path).await {
            Ok(config) => Ok(Some(config)),
            Err(error) if required => Err(error),
            Err(_) => Ok(None),
        }
    }

    pub(super) fn merge_optional(
        global: Option<RigelConfig>,
        project: Option<RigelConfig>,
    ) -> RigelConfig {
        match (global, project) {
            (Some(global), Some(project)) => Self::merge(global, project),
            (Some(global), None) => global,
            (None, Some(project)) => project,
            (None, None) => Self::default(),
        }
    }

    fn merge(global: RigelConfig, project: RigelConfig) -> RigelConfig {
        let RigelConfig {
            base_url: global_base_url,
            env_key: global_env_key,
            servers: global_servers,
            models: global_models,
            policies: global_policies,
        } = global;
        let RigelConfig {
            base_url: project_base_url,
            env_key: project_env_key,
            servers: project_servers,
            models: project_models,
            policies: project_policies,
        } = project;
        RigelConfig {
            base_url: project_base_url.or(global_base_url),
            env_key: project_env_key.or(global_env_key),
            servers: Self::merge_servers(global_servers, project_servers),
            models: Self::merge_models(global_models, project_models),
            policies: Self::merge_policies(global_policies, project_policies),
        }
    }

    pub(super) fn servers_ref(&self) -> &HashMap<String, ServerConfig> {
        &self.servers
    }

    pub(super) fn models_ref(&self) -> &HashMap<String, ModelConfig> {
        &self.models
    }

    pub(super) fn policies_ref(&self) -> &HashMap<String, PolicyConfig> {
        &self.policies
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL)
    }

    pub fn api_key(&self) -> Option<String> {
        let api_env_key = self.env_key.as_ref()?;
        let Ok(api_key) = std::env::var(api_env_key.as_str()) else {
            return None;
        };
        Some(api_key)
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn with_env_key(mut self, env_key: impl Into<String>) -> Self {
        self.env_key = Some(env_key.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::RigelConfig;
    use crate::shared::mcp_registry::server_config::ServerConfig;

    #[test]
    fn uses_default_base_url_when_missing() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config("")?;

        assert_eq!(config.base_url(), "http://127.0.0.1:1234/v1");
        Ok(())
    }

    #[test]
    fn available_config_layer_is_used_when_the_other_is_missing() -> anyhow::Result<()> {
        let project = RigelConfig::parse_config(
            r#"
baseUrl = "https://project.example/v1"

[policies.run_command]
allow = true
"#,
        )?;

        let merged = RigelConfig::merge_optional(None, Some(project));

        assert_eq!(merged.base_url(), "https://project.example/v1");
        assert_eq!(merged.policy_for("run_command"), Some(true));
        Ok(())
    }

    #[test]
    fn global_config_layer_is_kept_when_project_is_missing() -> anyhow::Result<()> {
        let global = RigelConfig::parse_config(
            r#"
baseUrl = "https://global.example/v1"

[policies.run_command]
allow = false
"#,
        )?;

        let merged = RigelConfig::merge_optional(Some(global), None);

        assert_eq!(merged.base_url(), "https://global.example/v1");
        assert_eq!(merged.policy_for("run_command"), Some(false));
        Ok(())
    }

    #[test]
    fn no_available_config_layer_keeps_default_behavior() {
        let merged = RigelConfig::merge_optional(None, None);

        assert_eq!(merged.base_url(), "http://127.0.0.1:1234/v1");
        assert_eq!(merged.policy_for("run_command"), None);
    }

    #[test]
    fn missing_project_fields_inherit_global_values() -> anyhow::Result<()> {
        let global = RigelConfig::parse_config(
            r#"
baseUrl = "https://global.example/v1"
envKey = "GLOBAL_KEY"

[policies.run_command]
allow = true

[mcpServers.local]
type = "stdio"
command = "global-command"

[mcpServers.remote]
type = "http"
url = "https://global.example/mcp"
"#,
        )?;
        let project = RigelConfig::parse_config(
            r#"
[policies.run_command]

[mcpServers.local]
type = "stdio"
args = ["project-arg"]

[mcpServers.remote]
type = "http"
"#,
        )?;

        let merged = RigelConfig::merge_optional(Some(global), Some(project));

        assert_eq!(merged.base_url(), "https://global.example/v1");
        assert_eq!(merged.env_key.as_deref(), Some("GLOBAL_KEY"));
        assert_eq!(merged.policy_for("run_command"), Some(true));
        let Some(ServerConfig::Stdio(local)) = merged.mcp_servers().get("local") else {
            anyhow::bail!("merged local MCP server is not stdio")
        };
        assert_eq!(local.command.as_deref(), Some("global-command"));
        assert_eq!(local.args, ["project-arg"]);
        let Some(ServerConfig::Http(remote)) = merged.mcp_servers().get("remote") else {
            anyhow::bail!("merged remote MCP server is not http")
        };
        assert_eq!(remote.url.as_deref(), Some("https://global.example/mcp"));
        Ok(())
    }

    #[test]
    fn default_config_serialization_omits_empty_policies() -> anyhow::Result<()> {
        let content = toml::to_string_pretty(&RigelConfig::default())?;

        assert!(!content.contains("policies"));
        Ok(())
    }

    #[test]
    fn project_layer_merges_maps_and_nested_arrays() -> anyhow::Result<()> {
        let global = RigelConfig::parse_config(
            r#"
baseUrl = "https://global.example/v1"
envKey = "GLOBAL_KEY"

[models.shared.params]
stop = ["global"]
[models.shared.params.reasoning]
effort = "low"
global_only = true

[policies.run_command]
allow = false
[policies.fetch_url]
allow = true

[mcpServers.local]
type = "stdio"
command = "global-command"
args = ["global-arg"]
[mcpServers.local.env]
SHARED = "global"
GLOBAL_ONLY = "yes"
"#,
        )?;
        let project = RigelConfig::parse_config(
            r#"
[models.shared.params]
stop = ["project"]
project_only = 7
[models.shared.params.reasoning]
effort = "high"

[policies.run_command]
allow = true

[mcpServers.local]
type = "stdio"
command = "project-command"
args = ["project-arg"]
[mcpServers.local.env]
SHARED = "project"
PROJECT_ONLY = "yes"
"#,
        )?;

        let merged = RigelConfig::merge_optional(Some(global), Some(project));

        assert_eq!(merged.base_url(), "https://global.example/v1");
        assert_eq!(merged.env_key.as_deref(), Some("GLOBAL_KEY"));
        assert_eq!(merged.api_key(), None);
        assert_eq!(merged.policy_for("run_command"), Some(true));
        assert_eq!(merged.policy_for("fetch_url"), Some(true));
        assert_eq!(
            merged.model_params("shared")?,
            Some(serde_json::json!({
                "stop": ["global", "project"],
                "project_only": 7,
                "reasoning": {
                    "effort": "high",
                    "global_only": true
                }
            }))
        );

        let Some(ServerConfig::Stdio(server)) = merged.mcp_servers().get("local") else {
            anyhow::bail!("merged MCP server is not stdio")
        };
        assert_eq!(server.command.as_deref(), Some("project-command"));
        assert_eq!(server.args, ["global-arg", "project-arg"]);
        assert_eq!(
            server.env.get("SHARED").map(String::as_str),
            Some("project")
        );
        assert_eq!(
            server.env.get("GLOBAL_ONLY").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            server.env.get("PROJECT_ONLY").map(String::as_str),
            Some("yes")
        );
        Ok(())
    }
}
