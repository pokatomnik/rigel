use std::sync::Arc;

use crate::{
    contracts::application::{ChatRunner, ChatWorkflow},
    entities::application::chat_request::ChatRequest,
};

/// Provider-neutral application boundary for one interactive chat session.
pub(crate) struct Chat {
    runner: Arc<dyn ChatRunner>,
}

impl Chat {
    /// Creates a chat workflow from a runtime-independent runner selected by composition.
    pub(crate) fn new(runner: Arc<dyn ChatRunner>) -> Self {
        Self { runner }
    }

    /// Runs one composed chat session and keeps request interpretation outside runtime adapters.
    pub(crate) async fn run(&self, request: ChatRequest) -> anyhow::Result<()> {
        self.runner.run(request).await
    }
}

impl ChatWorkflow for Chat {
    fn run(
        &self,
        request: ChatRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + '_>> {
        Box::pin(Chat::run(self, request))
    }
}
