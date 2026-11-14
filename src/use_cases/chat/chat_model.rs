use crate::entities::model::model_session_kind::ModelSessionKind;

use super::workflow::Chat;

impl Chat {
    /// Replaces the session only after the factory has built a complete candidate.
    pub(super) async fn handle_change_model(
        &self,
        model: Option<crate::entities::selected_model::selection::SelectedModel>,
    ) -> anyhow::Result<()> {
        let factory = self
            .model_factory
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("model session factory was not configured"))?;
        let replacement = factory.create(ModelSessionKind::ModelChange, model).await?;
        let max_context_tokens = replacement.max_context_tokens();
        let selected_model = replacement.selected_model();
        let mut session = self.session.lock().await;
        *session = replacement;
        drop(session);
        if let Some(model) = selected_model.clone() {
            self.emit(crate::entities::agent_event::event::AgentEvent::ModelChanged(model));
        }
        *self.selected_model.write().await = selected_model;
        self.update_context_usage(
            crate::entities::conversation::context_usage::ContextUsage::new(
                None,
                max_context_tokens,
            ),
        )
        .await;
        Ok(())
    }
}
