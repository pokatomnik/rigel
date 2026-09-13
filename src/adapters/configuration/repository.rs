use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    contracts::configuration::ConfigurationRepository,
    entities::configuration::{configuration::RigelConfig, paths::ConfigurationPaths},
};

const RIGEL_DIRECTORY: &str = ".rigel";
const CONFIG_FILE_NAME: &str = "config.toml";

/// Filesystem/TOML adapter for global, explicit-profile, and project config layers.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FileConfigurationRepository;

impl FileConfigurationRepository {
    /// Creates the adapter; path resolution is deferred until a request is handled.
    pub(crate) const fn new() -> Self {
        Self
    }

    fn default_path() -> anyhow::Result<PathBuf> {
        let home = std::env::home_dir()
            .ok_or_else(|| anyhow::anyhow!("failed to determine the user home directory"))?;
        Ok(home.join(RIGEL_DIRECTORY).join(CONFIG_FILE_NAME))
    }

    fn project_path() -> anyhow::Result<PathBuf> {
        Ok(std::env::current_dir()?
            .join(RIGEL_DIRECTORY)
            .join(CONFIG_FILE_NAME))
    }

    async fn read_layer(path: PathBuf, required: bool) -> anyhow::Result<Option<RigelConfig>> {
        match tokio::fs::read_to_string(path.as_path()).await {
            Ok(content) => Ok(Some(Self::parse(content.as_str())?)),
            Err(error) if error.kind() == ErrorKind::NotFound && !required => Ok(None),
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

    fn parse(content: &str) -> anyhow::Result<RigelConfig> {
        let config: RigelConfig = toml::from_str(content)?;
        for (model_id, model) in &config.models {
            model.request_parameters(model_id)?;
        }
        Ok(config)
    }

    fn missing_file_error(path: &Path) -> anyhow::Error {
        anyhow::anyhow!(
            "Rigel configuration file '{}' is missing. Run `rigel init` to perform basic initialization.",
            path.display()
        )
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

    async fn should_write(path: &Path, confirm_override: bool) -> anyhow::Result<bool> {
        if !confirm_override || !tokio::fs::try_exists(path).await? {
            return Ok(true);
        }
        Ok(dialoguer::Confirm::new()
            .with_prompt(format!(
                "Overwrite Rigel configuration file '{}' ?",
                path.display()
            ))
            .report(false)
            .interact()?)
    }
}

impl ConfigurationRepository for FileConfigurationRepository {
    fn load(
        &self,
        profile: Option<&Path>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<RigelConfig>> + Send + '_>>
    {
        let profile = profile.map(Path::to_path_buf);
        Box::pin(async move {
            let paths = self.paths(profile.as_deref())?;
            let global = Self::read_layer(paths.global_config_path, profile.is_some()).await?;
            let project = Self::read_layer(paths.project_config_path, false).await?;
            Ok(RigelConfig::merge(global, project))
        })
    }

    fn save_initial(
        &self,
        config: &RigelConfig,
        confirm_override: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>> {
        let config = config.clone();
        Box::pin(async move {
            let path = Self::default_path()?;
            if !Self::should_write(path.as_path(), confirm_override).await? {
                return Ok(());
            }
            let content = toml::to_string_pretty(&config)
                .context("failed to serialize Rigel configuration")?;
            Self::write_path(path, content).await
        })
    }

    fn paths(&self, profile: Option<&Path>) -> anyhow::Result<ConfigurationPaths> {
        let global_config_path = match profile {
            Some(path) => path.to_path_buf(),
            None => Self::default_path()?,
        };
        Ok(ConfigurationPaths {
            global_config_path,
            project_config_path: Self::project_path()?,
        })
    }
}
