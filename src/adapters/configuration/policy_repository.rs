use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    contracts::configuration::ToolPolicyRepository,
    entities::configuration::{configuration::RigelConfig, paths::ConfigurationPaths},
};

/// TOML policy adapter used by the permission workflow.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FileToolPolicyRepository;

impl FileToolPolicyRepository {
    /// Creates a policy adapter without selecting a config layer.
    pub(crate) const fn new() -> Self {
        Self
    }

    async fn merged_policy(
        global_path: PathBuf,
        project_path: PathBuf,
        tool: String,
    ) -> Option<bool> {
        let global = Self::read(global_path.as_path()).await.ok().flatten();
        let project = Self::read(project_path.as_path()).await.ok().flatten();
        RigelConfig::merge(global, project).policy_for(tool.as_str())
    }

    async fn read(path: &Path) -> anyhow::Result<Option<RigelConfig>> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => Ok(Some(toml::from_str(content.as_str())?)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to read Rigel configuration file '{}'",
                    path.display()
                )
            }),
        }
    }

    async fn update(path: PathBuf, tool: String) -> anyhow::Result<()> {
        let content = match tokio::fs::read_to_string(path.as_path()).await {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let mut document: toml::Table = if content.trim().is_empty() {
            toml::Table::new()
        } else {
            toml::from_str(content.as_str())?
        };
        let policies = document
            .entry("policies".to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("configuration field `policies` must be a table"))?;
        let policy = policies
            .entry(tool.clone())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("configuration policy `{tool}` must be a table"))?;
        policy.insert("allow".to_owned(), toml::Value::Boolean(true));
        let serialized = toml::to_string_pretty(&document)
            .context("failed to serialize Rigel policy configuration")?;
        let parent = path.parent().filter(|value| !value.as_os_str().is_empty());
        let parent = parent.unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent).await.with_context(|| {
            format!(
                "failed to create Rigel configuration directory '{}'",
                parent.display()
            )
        })?;
        tokio::fs::write(path.as_path(), serialized)
            .await
            .with_context(|| {
                format!(
                    "failed to write Rigel configuration file '{}'",
                    path.display()
                )
            })
    }
}

impl ToolPolicyRepository for FileToolPolicyRepository {
    fn policy_for(
        &self,
        paths: &ConfigurationPaths,
        tool_name: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<bool>> + Send + '_>> {
        let global_path = paths.global_config_path.clone();
        let project_path = paths.project_config_path.clone();
        let tool_name = tool_name.to_owned();
        Box::pin(Self::merged_policy(global_path, project_path, tool_name))
    }

    fn allow(
        &self,
        path: &Path,
        tool_name: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let path = path.to_path_buf();
        let tool_name = tool_name.to_owned();
        Box::pin(Self::update(path, tool_name))
    }
}
