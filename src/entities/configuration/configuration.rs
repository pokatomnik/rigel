use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{model_config::ModelConfig, server_config::McpServerConfig};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:1234/v1";

/// Immutable data model for merged Rigel configuration layers.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub(crate) struct RigelConfig {
    /// OpenAI-compatible API base URL.
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub(crate) base_url: Option<String>,
    /// Environment variable containing the provider API key.
    #[serde(rename = "envKey", skip_serializing_if = "Option::is_none")]
    pub(crate) api_key_env: Option<String>,
    /// Named MCP server definitions.
    #[serde(rename = "mcpServers", default)]
    pub(crate) servers: HashMap<String, McpServerConfig>,
    /// Model-specific provider parameters.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub(crate) models: HashMap<String, ModelConfig>,
    /// Explicit tool permission policies.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub(crate) policies: HashMap<String, PolicyConfig>,
}

/// A persisted allow/deny policy for one runtime tool name.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PolicyConfig {
    /// Explicit decision; `None` means no configured policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) allow: Option<bool>,
}

impl Default for RigelConfig {
    fn default() -> Self {
        Self {
            base_url: Some(DEFAULT_BASE_URL.to_string()),
            api_key_env: None,
            servers: HashMap::new(),
            models: HashMap::new(),
            policies: HashMap::new(),
        }
    }
}

impl RigelConfig {
    /// Returns the configured endpoint, falling back to the local default.
    pub(crate) fn base_url(&self) -> &str {
        self.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL)
    }

    /// Returns the environment variable name for the provider key, if configured.
    pub(crate) fn api_key_env(&self) -> Option<&str> {
        self.api_key_env.as_deref()
    }

    /// Returns configured MCP servers for the transport adapter.
    pub(crate) fn mcp_servers(&self) -> &HashMap<String, McpServerConfig> {
        &self.servers
    }

    /// Returns one tool policy without consulting the environment or filesystem.
    pub(crate) fn policy_for(&self, tool: &str) -> Option<bool> {
        self.policies.get(tool).and_then(|policy| policy.allow)
    }

    /// Returns provider parameters for an exact model id after validation.
    pub(crate) fn model_params(&self, model_id: &str) -> anyhow::Result<Option<Value>> {
        self.models
            .get(model_id)
            .map(|config| config.request_parameters(model_id))
            .transpose()
            .map(|value| value.flatten())
    }

    /// Builds a config with a replacement endpoint for `init` requests.
    pub(crate) fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Builds a config with an API-key environment variable name.
    pub(crate) fn with_api_key_env(mut self, api_key_env: String) -> Self {
        self.api_key_env = Some(api_key_env);
        self
    }

    /// Merges global and project layers, with project scalar fields taking precedence.
    pub(crate) fn merge(global: Option<Self>, project: Option<Self>) -> Self {
        match (global, project) {
            (Some(global), Some(project)) => Self::merge_layers(global, project),
            (Some(global), None) => global,
            (None, Some(project)) => project,
            (None, None) => Self::default(),
        }
    }

    fn merge_layers(global: Self, project: Self) -> Self {
        Self {
            base_url: project.base_url.or(global.base_url),
            api_key_env: project.api_key_env.or(global.api_key_env),
            servers: merge_servers(global.servers, project.servers),
            models: merge_models(global.models, project.models),
            policies: merge_policies(global.policies, project.policies),
        }
    }
}

fn merge_servers(
    mut global: HashMap<String, McpServerConfig>,
    project: HashMap<String, McpServerConfig>,
) -> HashMap<String, McpServerConfig> {
    for (name, project_server) in project {
        let merged = match global.remove(&name) {
            Some(global_server) => McpServerConfig::merge(global_server, project_server),
            None => project_server,
        };
        global.insert(name, merged);
    }
    global
}

fn merge_models(
    mut global: HashMap<String, ModelConfig>,
    project: HashMap<String, ModelConfig>,
) -> HashMap<String, ModelConfig> {
    for (name, project_model) in project {
        let merged = match global.remove(&name) {
            Some(global_model) => ModelConfig::merge(global_model, project_model),
            None => project_model,
        };
        global.insert(name, merged);
    }
    global
}

fn merge_policies(
    mut global: HashMap<String, PolicyConfig>,
    project: HashMap<String, PolicyConfig>,
) -> HashMap<String, PolicyConfig> {
    for (name, project_policy) in project {
        let merged = match global.remove(&name) {
            Some(global_policy) => PolicyConfig {
                allow: project_policy.allow.or(global_policy.allow),
            },
            None => project_policy,
        };
        global.insert(name, merged);
    }
    global
}

#[cfg(test)]
mod tests {
    use super::RigelConfig;

    #[test]
    fn default_config_has_local_base_url_and_no_policy() {
        let config = RigelConfig::default();
        assert_eq!(config.base_url(), "http://127.0.0.1:1234/v1");
        assert_eq!(config.policy_for("missing"), None);
    }

    #[test]
    fn merge_keeps_available_layer_when_the_other_is_missing() -> anyhow::Result<()> {
        let project: RigelConfig = toml::from_str(
            r#"
baseUrl = "https://project.example/v1"

[policies.run_command]
allow = true
"#,
        )?;
        let merged = RigelConfig::merge(None, Some(project));
        assert_eq!(merged.base_url(), "https://project.example/v1");
        assert_eq!(merged.policy_for("run_command"), Some(true));
        Ok(())
    }

    #[test]
    fn project_values_override_global_values_but_nested_data_is_preserved() -> anyhow::Result<()> {
        let global: RigelConfig = toml::from_str(
            r#"
baseUrl = "https://global.example/v1"
envKey = "GLOBAL_KEY"

[models."Model-A".params]
think = true

[mcpServers.local]
type = "stdio"
command = "global-command"

[policies.run_command]
allow = true
"#,
        )?;
        let project: RigelConfig = toml::from_str(
            r#"
[models."Model-A".params]
reasoning = "high"

[mcpServers.local]
type = "stdio"
args = ["project-arg"]

[policies.run_command]
"#,
        )?;
        let merged = RigelConfig::merge(Some(global), Some(project));

        assert_eq!(merged.base_url(), "https://global.example/v1");
        assert_eq!(merged.api_key_env(), Some("GLOBAL_KEY"));
        assert_eq!(merged.policy_for("run_command"), Some(true));
        assert!(merged.model_params("Model-A")?.is_some());
        assert_eq!(merged.mcp_servers().len(), 1);
        Ok(())
    }

    #[test]
    fn model_lookup_is_exact_and_transport_fields_are_rejected() -> anyhow::Result<()> {
        let config: RigelConfig = toml::from_str(
            r#"
[models."Model-A".params]
think = true
"#,
        )?;
        assert!(config.model_params("model-a")?.is_none());
        assert!(config.model_params("Model-A")?.is_some());

        let invalid: RigelConfig = toml::from_str(
            r#"
[models.unsafe.params]
stream = true
"#,
        )?;
        assert!(invalid.model_params("unsafe").is_err());
        Ok(())
    }

    #[test]
    fn both_missing_layers_restore_default_configuration() {
        let merged = RigelConfig::merge(None, None);
        assert_eq!(merged, RigelConfig::default());
    }

    #[test]
    fn global_layer_is_preserved_when_project_layer_is_missing() {
        let global = RigelConfig::default().with_api_key_env("GLOBAL_KEY".to_string());
        let merged = RigelConfig::merge(Some(global.clone()), None);
        assert_eq!(merged, global);
    }

    #[test]
    fn empty_model_parameters_do_not_create_request_fields() -> anyhow::Result<()> {
        let config: RigelConfig = toml::from_str("[models.empty]")?;
        assert_eq!(config.model_params("empty")?, None);
        Ok(())
    }

    #[test]
    fn model_parameters_preserve_nested_values_without_conversion() -> anyhow::Result<()> {
        let config: RigelConfig = toml::from_str(
            r#"
[models.example.params]
reasoning = { effort = "high" }
values = ["first", "second"]
"#,
        )?;
        assert_eq!(
            config.model_params("example")?,
            Some(serde_json::json!({
                "reasoning": {"effort": "high"},
                "values": ["first", "second"]
            }))
        );
        Ok(())
    }

    #[test]
    fn default_serialization_omits_empty_policies() -> anyhow::Result<()> {
        let serialized = toml::to_string(&RigelConfig::default())?;
        assert!(!serialized.contains("policies"));
        Ok(())
    }

    #[test]
    fn false_policy_is_distinct_from_missing_policy() -> anyhow::Result<()> {
        let config: RigelConfig = toml::from_str("[policies.run_command]\nallow = false\n")?;
        assert_eq!(config.policy_for("run_command"), Some(false));
        assert_eq!(config.policy_for("read_file"), None);
        Ok(())
    }

    #[test]
    fn mcp_merge_preserves_global_transport_and_project_arguments() -> anyhow::Result<()> {
        let global: RigelConfig = toml::from_str(
            "[mcpServers.local]\ntype = \"stdio\"\ncommand = \"server\"\nenv = { GLOBAL = \"yes\" }\n",
        )?;
        let project: RigelConfig = toml::from_str(
            "[mcpServers.local]\ntype = \"stdio\"\nargs = [\"project\"]\nenv = { PROJECT = \"yes\" }\n",
        )?;
        let merged = RigelConfig::merge(Some(global), Some(project));
        let server = merged
            .mcp_servers()
            .get("local")
            .ok_or_else(|| anyhow::anyhow!("missing server"))?;
        let crate::entities::configuration::server_config::McpServerConfig::Stdio(server) = server
        else {
            anyhow::bail!("expected stdio server")
        };
        assert_eq!(server.command.as_deref(), Some("server"));
        assert_eq!(server.args, ["project".to_string()]);
        Ok(())
    }
}
