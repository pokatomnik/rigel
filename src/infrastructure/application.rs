use std::pin::Pin;

use crate::entities::application::init_request::InitRequest;

/// Initializes configuration from a provider-neutral request.
pub(crate) trait ConfigurationInitializer: Send + Sync {
    /// Persists initial configuration using the configured repository adapter.
    fn initialize(
        &self,
        request: InitRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
}
