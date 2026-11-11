use std::sync::Arc;

use crate::{
    entities::{application::init_request::InitRequest, configuration::config::RigelConfig},
    infrastructure::{
        application::ConfigurationInitializer, configuration::ConfigurationRepository,
    },
};

/// Application workflow that maps `InitRequest` into a repository-owned config write.
pub(crate) struct InitializeConfiguration<R> {
    repository: Arc<R>,
}

impl<R> InitializeConfiguration<R>
where
    R: ConfigurationRepository + 'static,
{
    /// Creates the initializer with one configuration repository implementation.
    pub(crate) fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    fn build_config(request: &InitRequest) -> RigelConfig {
        let config = RigelConfig::default().with_base_url(request.base_url().to_string());
        match request.api_key_env() {
            Some(name) => config.with_api_key_env(name.to_string()),
            None => config,
        }
    }
}

impl<R> ConfigurationInitializer for InitializeConfiguration<R>
where
    R: ConfigurationRepository + 'static,
{
    fn initialize(
        &self,
        request: InitRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>> {
        let repository = self.repository.clone();
        Box::pin(async move {
            let config = Self::build_config(&request);
            repository.save_initial(&config, true).await
        })
    }
}
