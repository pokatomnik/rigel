use std::sync::Arc;

use futures::future::BoxFuture;

use crate::{
    entities::{
        agent_event::payload::InteractionOrigin, application::input_outcome::InputOutcome,
        conversation::context_usage::ContextUsage,
    },
    infrastructure::{interaction::UserInteraction, ui::gateway::EventSink},
};

use super::broker::EventInteractionBroker;

/// Publishes `ask_user` questions as events and waits for the modal answer.
pub(crate) struct EventUserInteraction {
    broker: Arc<EventInteractionBroker>,
    events: Arc<dyn EventSink>,
    origin: InteractionOrigin,
}

impl EventUserInteraction {
    /// Creates an interaction adapter for one agent scope.
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

impl UserInteraction for EventUserInteraction {
    fn select<'a>(
        &'a self,
        question: &'a str,
        items: &'a [String],
    ) -> BoxFuture<'a, anyhow::Result<Option<usize>>> {
        Box::pin(async move {
            Ok(self
                .broker
                .ask_selection(
                    self.events.as_ref(),
                    question.to_string(),
                    items.to_vec(),
                    self.origin,
                )
                .await)
        })
    }

    fn readline<'a>(
        &'a self,
        _context_usage: ContextUsage,
    ) -> BoxFuture<'a, anyhow::Result<InputOutcome>> {
        Box::pin(async move {
            match self
                .broker
                .ask_text(
                    self.events.as_ref(),
                    "Type an answer".to_string(),
                    self.origin,
                )
                .await
            {
                Some(answer) if !answer.trim().is_empty() => Ok(InputOutcome::Text(answer)),
                Some(_) => Ok(InputOutcome::Text(String::new())),
                None => Ok(InputOutcome::Cancelled),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::EventUserInteraction;
    use crate::{
        controllers::ui_bridge::broker::EventInteractionBroker,
        entities::{
            agent_event::{event::AgentEvent, payload::InteractionOrigin},
            conversation::context_usage::ContextUsage,
        },
        infrastructure::{
            interaction::UserInteraction,
            ui::{gateway::EventSink, interaction::InteractionBroker},
        },
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
    async fn selection_waits_for_the_modal_answer() -> anyhow::Result<()> {
        let broker = Arc::new(EventInteractionBroker::new());
        let sink = Arc::new(RecordingSink::default());
        let interaction =
            EventUserInteraction::new(broker.clone(), sink.clone(), InteractionOrigin::MainAgent);
        let items = vec!["first".to_string(), "second".to_string()];
        let pending = tokio::spawn(async move { interaction.select("pick", &items).await });

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
            Some(AgentEvent::QuestionAsked(question)) => question.correlation_id,
            _ => anyhow::bail!("question was not emitted"),
        };
        InteractionBroker::resolve_question(broker.as_ref(), id, "1".to_string()).await;

        let selected = pending.await??;
        assert_eq!(selected, Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn free_form_cancel_returns_cancelled() -> anyhow::Result<()> {
        let broker = Arc::new(EventInteractionBroker::new());
        let sink = Arc::new(RecordingSink::default());
        let interaction =
            EventUserInteraction::new(broker.clone(), sink.clone(), InteractionOrigin::MainAgent);
        let pending =
            tokio::spawn(async move { interaction.readline(ContextUsage::new(None, None)).await });

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
            Some(AgentEvent::QuestionAsked(question)) => question.correlation_id,
            _ => anyhow::bail!("question was not emitted"),
        };
        InteractionBroker::cancel_question(broker.as_ref(), id).await;

        let outcome = pending.await??;
        assert!(matches!(
            outcome,
            crate::entities::application::input_outcome::InputOutcome::Cancelled
        ));
        Ok(())
    }
}
