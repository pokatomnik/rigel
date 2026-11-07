use std::sync::Arc;

use crate::{
    entities::application::chat_request::ChatRequest,
    infrastructure::application::{ApplicationOutput, ChatRunner},
};

/// Translates one CLI chat request into the composed chat workflow.
pub(crate) struct ChatController {
    workflow: Arc<dyn ChatRunner>,
    output: Arc<dyn ApplicationOutput>,
}

impl ChatController {
    /// Creates a controller from an application workflow and output transport.
    pub(crate) fn new(workflow: Arc<dyn ChatRunner>, output: Arc<dyn ApplicationOutput>) -> Self {
        Self { workflow, output }
    }

    /// Handles one provider-neutral chat request and presents its welcome message.
    pub(crate) async fn handle(&self, request: ChatRequest) -> anyhow::Result<()> {
        self.output.eprintln(include_str!("./welcome_message.txt"));
        self.workflow.run(request).await
    }
}
