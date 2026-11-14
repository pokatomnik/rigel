use std::sync::Arc;

use futures::future::BoxFuture;

use crate::{
    entities::{
        agent_event::payload::InteractionOrigin, tool::tool_entities::ToolPermissionRequest,
        tool_confirm_result::outcome::ToolConfirmResult,
    },
    infrastructure::{interaction::ToolPermissionPrompter, ui::gateway::EventSink},
};

use super::broker::EventInteractionBroker;

/// Publishes permission requests as events and waits for the modal decision.
pub(crate) struct EventPermissionPrompter {
    broker: Arc<EventInteractionBroker>,
    events: Arc<dyn EventSink>,
    origin: InteractionOrigin,
}

impl EventPermissionPrompter {
    /// Creates a prompter for one agent scope.
    pub(crate) fn new(
        broker: Arc<EventInteractionBroker>,
        events: Arc<dyn EventSink>,
        origin: InteractionOrigin,
    ) -> Self {
        Self {
            broker,
            events,
            origin,
        }
    }
}

impl ToolPermissionPrompter for EventPermissionPrompter {
    fn confirm<'a>(
        &'a self,
        request: &'a ToolPermissionRequest,
    ) -> BoxFuture<'a, ToolConfirmResult> {
        Box::pin(async move {
            self.broker
                .ask_permission(
                    self.events.as_ref(),
                    request.tool_name().to_string(),
                    request.arguments().clone(),
                    self.origin,
                )
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::EventPermissionPrompter;
    use crate::{
        controllers::ui_bridge::broker::EventInteractionBroker,
        entities::{
            agent_event::{event::AgentEvent, payload::InteractionOrigin},
            tool::tool_entities::ToolPermissionRequest,
            tool_confirm_result::outcome::ToolConfirmResult,
        },
        infrastructure::{interaction::ToolPermissionPrompter, ui::gateway::EventSink},
    };

    #[derive(Default)]
    struct RecordingSink {
        events: std::sync::Mutex<Vec<AgentEvent>>,
    }

    impl EventSink for RecordingSink {
        fn emit(&self, event: AgentEvent) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event);
            }
        }
    }

    #[tokio::test]
    async fn permission_prompt_round_trips_through_the_modal_event() -> anyhow::Result<()> {
        let broker = Arc::new(EventInteractionBroker::new());
        let sink = Arc::new(RecordingSink::default());
        let prompter =
            EventPermissionPrompter::new(broker.clone(), sink.clone(), InteractionOrigin::Subagent);
        let request = ToolPermissionRequest::new(
            "edit_file".to_string(),
            serde_json::json!({"path": "src/main.rs"}),
        );
        let pending = tokio::spawn(async move { prompter.confirm(&request).await });

        for _ in 0..100 {
            if !sink
                .events
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        let id = match sink
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .first()
        {
            Some(AgentEvent::PermissionRequested(request)) => request.correlation_id,
            _ => anyhow::bail!("permission request was not emitted"),
        };
        crate::infrastructure::ui::interaction::InteractionBroker::resolve_permission(
            broker.as_ref(),
            id,
            ToolConfirmResult::AllowForSession,
        )
        .await;

        assert_eq!(pending.await?, ToolConfirmResult::AllowForSession);
        Ok(())
    }
}
