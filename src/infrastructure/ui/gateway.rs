use crate::entities::{agent_command::command::AgentCommand, agent_event::event::AgentEvent};

/// Sends commands to the agent side of the in-process event boundary.
pub(crate) trait AgentGateway: Send + Sync {
    /// Queues one command without waiting; delivery does not prove the action succeeded.
    fn send(&self, command: AgentCommand);
}

/// Publishes agent events to the UI side of the in-process boundary.
pub(crate) trait EventSink: Send + Sync {
    /// Emits one immutable fact or interaction request to the UI.
    fn emit(&self, event: AgentEvent);
}

/// Receives UI commands on the agent side of the in-process boundary.
pub(crate) trait AgentCommandSource: Send + Sync {
    /// Waits for the next command, returning `None` when the UI is gone.
    fn next_command(&self) -> futures::future::BoxFuture<'_, Option<AgentCommand>>;
}

/// Receives agent events on the UI side of the in-process boundary.
pub(crate) trait AgentEventSource: Send + Sync {
    /// Waits for the next event, returning `None` when the agent is gone.
    fn next_event(&self) -> futures::future::BoxFuture<'_, Option<AgentEvent>>;

    /// Returns an already queued event without waiting.
    fn try_next(&self) -> Option<AgentEvent>;
}
