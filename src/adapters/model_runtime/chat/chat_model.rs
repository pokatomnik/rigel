use crate::{
    adapters::model_runtime::chat::history::history_persistence::HistoryPersistence,
    contracts::model::ModelSessionKind,
};

use super::chat::Chat;

impl<P> Chat<P>
where
    P: HistoryPersistence,
{
    /// Replaces the session only after the factory has built a complete candidate.
    pub(super) async fn handle_change_model(&self) -> anyhow::Result<()> {
        let factory = self
            .model_factory
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("model session factory was not configured"))?;
        let replacement = factory.create(ModelSessionKind::ModelChange, None).await?;
        let max_context_tokens = replacement.max_context_tokens();
        let mut session = self.session.lock().await;
        *session = replacement;
        drop(session);
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
