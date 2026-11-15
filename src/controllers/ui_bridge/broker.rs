use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use futures::future::BoxFuture;
use tokio::sync::{Mutex, oneshot};

use crate::{
    entities::{
        agent_event::payload::{InteractionOrigin, PermissionRequestView, QuestionView},
        tool_confirm_result::outcome::ToolConfirmResult,
    },
    infrastructure::ui::{gateway::EventSink, interaction::InteractionBroker},
};

enum Pending {
    Permission(oneshot::Sender<ToolConfirmResult>),
    Selection(oneshot::Sender<Option<usize>>),
    Text(oneshot::Sender<Option<String>>),
}

/// Correlates event-based interaction requests with the user's eventual response.
#[derive(Default)]
pub(crate) struct EventInteractionBroker {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
}

impl EventInteractionBroker {
    /// Creates a broker with no outstanding requests.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publishes one permission request and waits for its decision.
    pub(crate) async fn ask_permission(
        &self,
        events: &dyn EventSink,
        tool_name: String,
        arguments: serde_json::Value,
        origin: InteractionOrigin,
    ) -> ToolConfirmResult {
        let id = self.allocate_id();
        let view = PermissionRequestView::new(id, tool_name, arguments, origin);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .await
            .insert(id, Pending::Permission(sender));
        events.emit(crate::entities::agent_event::event::AgentEvent::PermissionRequested(view));
        let decision = receiver.await.unwrap_or_default();
        events.emit(
            crate::entities::agent_event::event::AgentEvent::PermissionResolved {
                correlation_id: id,
            },
        );
        decision
    }

    /// Publishes one question with candidate answers and waits for a selection.
    pub(crate) async fn ask_selection(
        &self,
        events: &dyn EventSink,
        question: String,
        answers: Vec<String>,
        origin: InteractionOrigin,
    ) -> Option<usize> {
        let id = self.allocate_id();
        let view = QuestionView::new(id, question, answers, origin);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .await
            .insert(id, Pending::Selection(sender));
        events.emit(crate::entities::agent_event::event::AgentEvent::QuestionAsked(view));
        let answer = receiver.await.ok().flatten();
        events.emit(
            crate::entities::agent_event::event::AgentEvent::QuestionResolved {
                correlation_id: id,
            },
        );
        answer
    }

    /// Publishes one free-form question and waits for the typed answer.
    pub(crate) async fn ask_text(
        &self,
        events: &dyn EventSink,
        question: String,
        origin: InteractionOrigin,
    ) -> Option<String> {
        let id = self.allocate_id();
        let view = QuestionView::new(id, question, Vec::new(), origin);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, Pending::Text(sender));
        events.emit(crate::entities::agent_event::event::AgentEvent::QuestionAsked(view));
        let answer = receiver.await.ok().flatten();
        events.emit(
            crate::entities::agent_event::event::AgentEvent::QuestionResolved {
                correlation_id: id,
            },
        );
        answer
    }

    fn allocate_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1)
    }
}

impl InteractionBroker for EventInteractionBroker {
    fn resolve_permission(
        &self,
        correlation_id: u64,
        decision: ToolConfirmResult,
    ) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let pending = self.pending.lock().await.remove(&correlation_id);
            match pending {
                Some(Pending::Permission(sender)) => sender.send(decision).is_ok(),
                Some(Pending::Selection(_)) | Some(Pending::Text(_)) | None => false,
            }
        })
    }

    fn resolve_question(&self, correlation_id: u64, answer: String) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let pending = self.pending.lock().await.remove(&correlation_id);
            match pending {
                Some(Pending::Selection(sender)) => answer
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| sender.send(Some(index)).ok())
                    .is_some(),
                Some(Pending::Text(sender)) => sender.send(Some(answer)).is_ok(),
                Some(Pending::Permission(_)) | None => false,
            }
        })
    }

    fn cancel_question(&self, correlation_id: u64) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let pending = self.pending.lock().await.remove(&correlation_id);
            match pending {
                Some(Pending::Selection(sender)) => sender.send(None).is_ok(),
                Some(Pending::Text(sender)) => sender.send(None).is_ok(),
                Some(Pending::Permission(_)) | None => false,
            }
        })
    }

    fn cancel_all(&self) -> BoxFuture<'_, usize> {
        Box::pin(async move {
            let pending = std::mem::take(&mut *self.pending.lock().await);
            let count = pending.len();
            for (_, request) in pending {
                match request {
                    Pending::Permission(sender) => {
                        let _ = sender.send(ToolConfirmResult::Deny);
                    }
                    Pending::Selection(sender) => {
                        let _ = sender.send(None);
                    }
                    Pending::Text(sender) => {
                        let _ = sender.send(None);
                    }
                }
            }
            count
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::EventInteractionBroker;
    use crate::{
        entities::{
            agent_event::{event::AgentEvent, payload::InteractionOrigin},
            tool_confirm_result::outcome::ToolConfirmResult,
        },
        infrastructure::ui::{gateway::EventSink, interaction::InteractionBroker},
    };

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<AgentEvent>>,
    }

    impl EventSink for RecordingSink {
        fn emit(&self, event: AgentEvent) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event);
            }
        }
    }

    #[tokio::test]
    async fn permission_request_is_correlated_with_one_response() -> anyhow::Result<()> {
        let broker = Arc::new(EventInteractionBroker::new());
        let sink = Arc::new(RecordingSink::default());
        let request = tokio::spawn({
            let broker = broker.clone();
            let sink = sink.clone();
            async move {
                broker
                    .ask_permission(
                        sink.as_ref(),
                        "run_command".to_string(),
                        serde_json::json!({"command": "pwd"}),
                        InteractionOrigin::MainAgent,
                    )
                    .await
            }
        });

        wait_for_request(&sink).await?;
        let id = first_correlation_id(&sink)?;
        assert!(
            broker
                .resolve_permission(id, ToolConfirmResult::AllowOnce)
                .await
        );
        assert_eq!(request.await?, ToolConfirmResult::AllowOnce);
        assert!(!broker.resolve_permission(id, ToolConfirmResult::Deny).await);
        Ok(())
    }

    async fn wait_for_request(sink: &RecordingSink) -> anyhow::Result<()> {
        for _ in 0..100 {
            if !sink
                .events
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .is_empty()
            {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        anyhow::bail!("request was not emitted")
    }

    fn first_correlation_id(sink: &RecordingSink) -> anyhow::Result<u64> {
        let events = sink
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?;
        let Some(event) = events.first() else {
            anyhow::bail!("no events recorded");
        };
        match event {
            AgentEvent::PermissionRequested(request) => Ok(request.correlation_id),
            _ => anyhow::bail!("first event was not a permission request"),
        }
    }
}
