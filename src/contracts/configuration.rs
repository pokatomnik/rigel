use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use crate::entities::configuration::{configuration::RigelConfig, paths::ConfigurationPaths};

/// Loads and stores merged configuration without exposing TOML or filesystem details.
pub(crate) trait ConfigurationRepository: Send + Sync {
    /// Resolves paths and loads the requested profile plus optional project layer.
    fn load(
        &self,
        profile: Option<&Path>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<RigelConfig>> + Send + '_>>;

    /// Saves a newly initialized configuration, applying adapter-specific overwrite policy.
    fn save_initial(
        &self,
        config: &RigelConfig,
        confirm_override: bool,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Returns paths used by permission policy persistence for the selected profile.
    fn paths(&self, profile: Option<&Path>) -> anyhow::Result<ConfigurationPaths>;
}

/// Reads and updates one runtime tool's persisted permission policy.
pub(crate) trait ToolPolicyRepository: Send + Sync {
    /// Reads the merged policy, where project values override global values.
    fn policy_for(
        &self,
        paths: &ConfigurationPaths,
        tool_name: &str,
    ) -> Pin<Box<dyn Future<Output = Option<bool>> + Send + '_>>;

    /// Persists an allow decision to one selected configuration layer.
    fn allow(
        &self,
        path: &Path,
        tool_name: &str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
}
