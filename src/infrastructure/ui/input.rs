use futures::future::BoxFuture;

use crate::entities::application::input_event::InputEvent;

/// Delivers user input without exposing a terminal framework to the UI workflow.
pub(crate) trait InputSource: Send + Sync {
    /// Waits for the next input event, returning `None` when the stream is closed.
    fn next_event(&self) -> BoxFuture<'_, anyhow::Result<Option<InputEvent>>>;
}
