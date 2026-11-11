use std::{
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    entities::configuration::{config::RigelConfig, paths::ConfigurationPaths},
    infrastructure::configuration::ConfigurationRepository,
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
        let read_result = tokio::fs::read_to_string(path.as_path()).await;
        Self::read_result(read_result, path.as_path(), required)
    }

    fn read_result(
        read_result: io::Result<String>,
        path: &Path,
        required: bool,
    ) -> anyhow::Result<Option<RigelConfig>> {
        let content = match read_result {
            Ok(content) => content,
            Err(_) if !required => return Ok(None),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(Self::missing_file_error(path));
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read Rigel configuration file '{}'",
                        path.display()
                    )
                });
            }
        };
        match Self::parse(content.as_str()) {
            Ok(config) => Ok(Some(config)),
            Err(_) if !required => Ok(None),
            Err(error) => Err(error),
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

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "config.toml";

    fn config(base_url: &str) -> anyhow::Result<RigelConfig> {
        FileConfigurationRepository::parse(&format!("baseUrl = \"{base_url}\""))
    }

    #[test]
    fn optional_read_failures_are_ignored() -> anyhow::Result<()> {
        for kind in [ErrorKind::NotFound, ErrorKind::PermissionDenied] {
            let result = FileConfigurationRepository::read_result(
                Err(io::Error::new(kind, "unavailable")),
                Path::new(PATH),
                false,
            )?;
            assert!(result.is_none());
        }
        Ok(())
    }

    #[test]
    fn optional_malformed_toml_is_ignored() -> anyhow::Result<()> {
        let result = FileConfigurationRepository::read_result(
            Ok("[broken".to_string()),
            Path::new(PATH),
            false,
        )?;
        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn malformed_project_layer_preserves_valid_global_layer() -> anyhow::Result<()> {
        let global = config("https://global.example/v1")?;
        let project = FileConfigurationRepository::read_result(
            Ok("[broken".to_string()),
            Path::new(PATH),
            false,
        )?;
        let merged = RigelConfig::merge(Some(global), project);
        assert_eq!(merged.base_url(), "https://global.example/v1");
        Ok(())
    }

    #[test]
    fn malformed_default_global_falls_back_to_project_or_defaults() -> anyhow::Result<()> {
        let global = FileConfigurationRepository::read_result(
            Ok("[broken".to_string()),
            Path::new(PATH),
            false,
        )?;
        let project = Some(config("https://project.example/v1")?);
        assert_eq!(
            RigelConfig::merge(global, project).base_url(),
            "https://project.example/v1"
        );
        let global = FileConfigurationRepository::read_result(
            Err(io::Error::new(ErrorKind::NotFound, "missing")),
            Path::new(PATH),
            false,
        )?;
        assert_eq!(RigelConfig::merge(global, None), RigelConfig::default());
        Ok(())
    }

    #[test]
    fn required_profile_errors_propagate() {
        for kind in [ErrorKind::NotFound, ErrorKind::PermissionDenied] {
            assert!(
                FileConfigurationRepository::read_result(
                    Err(io::Error::new(kind, "unavailable")),
                    Path::new(PATH),
                    true,
                )
                .is_err()
            );
        }
        assert!(
            FileConfigurationRepository::read_result(
                Ok("[broken".to_string()),
                Path::new(PATH),
                true,
            )
            .is_err()
        );
    }
}
