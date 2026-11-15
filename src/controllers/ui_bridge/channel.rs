use std::sync::Arc;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::{
    entities::{agent_command::command::AgentCommand, agent_event::event::AgentEvent},
    infrastructure::ui::gateway::{AgentCommandSource, AgentEventSource, AgentGateway, EventSink},
};

/// Four endpoints of the in-process UI/agent boundary.
pub(crate) struct UiEndpoints {
    /// Sends UI commands to the agent.
    pub(crate) gateway: Arc<dyn AgentGateway>,
    /// Emits agent events to the UI.
    pub(crate) events: Arc<dyn EventSink>,
    /// Receives UI commands on the agent side.
    pub(crate) commands: Arc<dyn AgentCommandSource>,
    /// Receives agent events on the UI side.
    pub(crate) incoming: Arc<dyn AgentEventSource>,
}

/// Creates an unbounded command/event pair for one interactive run.
pub(crate) fn open() -> UiEndpoints {
    let (command_tx, command_rx) = mpsc::unbounded_channel::<AgentCommand>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<AgentEvent>();
    UiEndpoints {
        gateway: Arc::new(CommandSender { sender: command_tx }),
        events: Arc::new(EventSender { sender: event_tx }),
        commands: Arc::new(CommandReceiver::new(command_rx)),
        incoming: Arc::new(EventReceiver::new(event_rx)),
    }
}

struct CommandSender {
    sender: UnboundedSender<AgentCommand>,
}

impl AgentGateway for CommandSender {
    fn send(&self, command: AgentCommand) {
        let _ = self.sender.send(command);
    }
}

struct EventSender {
    sender: UnboundedSender<AgentEvent>,
}

impl EventSink for EventSender {
    fn emit(&self, event: AgentEvent) {
        let _ = self.sender.send(event);
    }
}

struct CommandReceiver {
    receiver: tokio::sync::Mutex<UnboundedReceiver<AgentCommand>>,
}

impl CommandReceiver {
    fn new(receiver: UnboundedReceiver<AgentCommand>) -> Self {
        Self {
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
}

impl AgentCommandSource for CommandReceiver {
    fn next_command(&self) -> futures::future::BoxFuture<'_, Option<AgentCommand>> {
        Box::pin(async move { self.receiver.lock().await.recv().await })
    }
}

struct EventReceiver {
    receiver: tokio::sync::Mutex<UnboundedReceiver<AgentEvent>>,
}

impl EventReceiver {
    fn new(receiver: UnboundedReceiver<AgentEvent>) -> Self {
        Self {
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
}

impl AgentEventSource for EventReceiver {
    fn next_event(&self) -> futures::future::BoxFuture<'_, Option<AgentEvent>> {
        Box::pin(async move { self.receiver.lock().await.recv().await })
    }

    fn try_next(&self) -> Option<AgentEvent> {
        self.receiver.try_lock().ok()?.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::open;
    use crate::entities::{agent_command::command::AgentCommand, agent_event::event::AgentEvent};

    #[tokio::test]
    async fn commands_and_events_cross_the_boundary_in_order() -> anyhow::Result<()> {
        let endpoints = open();
        endpoints.gateway.send(AgentCommand::Stop);
        endpoints.events.emit(AgentEvent::ActivityChanged(
            crate::entities::agent_event::status::AgentActivity::Stopping,
        ));

        assert_eq!(
            endpoints.commands.next_command().await,
            Some(AgentCommand::Stop)
        );
        assert!(matches!(
            endpoints.incoming.next_event().await,
            Some(AgentEvent::ActivityChanged(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn queued_events_can_be_drained_without_waiting() -> anyhow::Result<()> {
        let endpoints = open();
        endpoints.events.emit(AgentEvent::ActivityChanged(
            crate::entities::agent_event::status::AgentActivity::Idle,
        ));

        assert!(endpoints.incoming.try_next().is_some());
        assert!(endpoints.incoming.try_next().is_none());
        Ok(())
    }
}
