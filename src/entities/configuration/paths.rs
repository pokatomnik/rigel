use std::path::PathBuf;

/// Resolved global and project configuration locations used by persistence adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigurationPaths {
    /// The global configuration file or explicit profile path.
    pub(crate) global_config_path: PathBuf,
    /// The project-local configuration file.
    pub(crate) project_config_path: PathBuf,
}
