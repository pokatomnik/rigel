use std::sync::Arc;

use crate::{
    entities::application::init_request::InitRequest,
    infrastructure::application::ConfigurationInitializer,
};

/// Invokes the already composed configuration initialization workflow.
pub(crate) struct InitController {
    initializer: Arc<dyn ConfigurationInitializer>,
}

impl InitController {
    /// Creates a controller around a contract-selected initializer.
    pub(crate) fn new(initializer: Arc<dyn ConfigurationInitializer>) -> Self {
        Self { initializer }
    }

    /// Handles one provider-neutral initialization request.
    pub(crate) async fn handle(&self, request: InitRequest) -> anyhow::Result<()> {
        self.initializer.initialize(request).await
    }
}
