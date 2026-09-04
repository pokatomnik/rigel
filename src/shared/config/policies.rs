use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::rigel_config::RigelConfig;

#[derive(Deserialize, Serialize, Default)]
pub(super) struct PolicyConfig {
    allow: bool,
}

impl RigelConfig {
    pub(crate) async fn policy_from_paths(
        global_path: &Path,
        project_path: &Path,
        tool: &str,
    ) -> Option<bool> {
        let global = Self::read_layer(global_path.to_path_buf(), false)
            .await
            .ok()
            .flatten();
        let project = Self::read_layer(project_path.to_path_buf(), false)
            .await
            .ok()
            .flatten();
        Self::merge_optional(global, project).policy_for(tool)
    }

    pub(crate) async fn update_policy_file(
        path: impl Into<PathBuf>,
        tool: &str,
        allow: bool,
    ) -> anyhow::Result<()> {
        let path = path.into();
        let content = Self::read_document(path.as_path()).await?;
        let mut document = Self::parse_document(content.as_str())?;
        Self::set_policy(&mut document, tool, allow)?;
        let content = toml::to_string_pretty(&document)
            .context("failed to serialize Rigel policy configuration")?;
        Self::write_document(path, content).await
    }

    pub(crate) fn policy_for(&self, tool: &str) -> Option<bool> {
        self.policies_ref().get(tool).map(|policy| policy.allow)
    }

    pub(super) fn merge_policies(
        mut global: HashMap<String, PolicyConfig>,
        project: HashMap<String, PolicyConfig>,
    ) -> HashMap<String, PolicyConfig> {
        global.extend(project);
        global
    }

    fn parse_document(content: &str) -> anyhow::Result<toml::Table> {
        if content.trim().is_empty() {
            return Ok(toml::Table::new());
        }
        Ok(toml::from_str(content)?)
    }

    async fn read_document(path: &Path) -> anyhow::Result<String> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => Ok(content),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to read Rigel configuration file '{}'",
                    path.display()
                )
            }),
        }
    }

    async fn write_document(path: PathBuf, content: String) -> anyhow::Result<()> {
        let directory = Self::parent_directory(path.as_path());
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

    fn set_policy(document: &mut toml::Table, tool: &str, allow: bool) -> anyhow::Result<()> {
        let policies = document
            .entry("policies".to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let policies = policies
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("configuration field `policies` must be a table"))?;
        let policy = policies
            .entry(tool.to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let policy = policy
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("configuration policy `{tool}` must be a table"))?;
        policy.insert("allow".to_owned(), toml::Value::Boolean(allow));
        Ok(())
    }

    fn parent_directory(path: &Path) -> &Path {
        match path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            Some(parent) => parent,
            None => Path::new("."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::rigel_config::RigelConfig;

    #[test]
    fn default_config_has_no_policies() {
        assert_eq!(RigelConfig::default().policy_for("missing"), None);
    }

    #[test]
    fn policies_keep_explicit_true_and_false_values() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[policies.allowed]
allow = true

[policies.denied]
allow = false
"#,
        )?;

        assert_eq!(config.policy_for("allowed"), Some(true));
        assert_eq!(config.policy_for("denied"), Some(false));
        assert_eq!(config.policy_for("missing"), None);
        Ok(())
    }

    #[test]
    fn document_policy_update_preserves_other_fields() -> anyhow::Result<()> {
        let mut document = RigelConfig::parse_document(
            r#"
baseUrl = "https://provider.example/v1"
envKey = "API_KEY"

[policies.existing]
allow = false
note = "keep"
"#,
        )?;

        RigelConfig::set_policy(&mut document, "run_command", true)?;

        assert_eq!(
            document["baseUrl"].as_str(),
            Some("https://provider.example/v1")
        );
        assert_eq!(document["envKey"].as_str(), Some("API_KEY"));
        assert_eq!(
            document["policies"]["existing"]["note"].as_str(),
            Some("keep")
        );
        assert_eq!(
            document["policies"]["run_command"]["allow"].as_bool(),
            Some(true)
        );
        Ok(())
    }
}
