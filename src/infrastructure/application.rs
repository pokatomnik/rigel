use std::future::Future;
use std::pin::Pin;

use crate::entities::application::{
    chat_action::ChatAction, chat_request::ChatRequest, init_request::InitRequest,
};

/// Parses one interactive line into a provider-neutral chat action.
pub(crate) trait ChatCommandParser: Send + Sync {
    /// Parses input without performing I/O, discovery, or output.
    fn parse(&self, input: &str, echo: bool) -> ChatAction;
}

/// Runs a fully composed chat session without constructing runtime collaborators.
pub(crate) trait ChatRunner: Send + Sync {
    /// Starts the session selected by the application request.
    fn run(&self, request: ChatRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + '_>>;
}

/// Emits user-facing application messages without binding controllers to a terminal.
pub(crate) trait ApplicationOutput: Send + Sync {
    /// Writes an application message to the selected error transport.
    fn eprintln(&self, message: &str);
}

/// Initializes configuration from a provider-neutral request.
pub(crate) trait ConfigurationInitializer: Send + Sync {
    /// Persists initial configuration using the configured repository adapter.
    fn initialize(
        &self,
        request: InitRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
}
