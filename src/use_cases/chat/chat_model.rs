use rig::completion::CompletionModel;

use crate::entities::context_usage::ContextUsage;

use super::chat::Chat;

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: crate::shared::history::history_persistence::HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<crate::shared::agent::agent::ConfiguredAgent<CM>> + 'static,
{
    pub(super) async fn handle_change_model(&self) -> anyhow::Result<()> {
        let new_agent = (self.change_model)().await?;
        let mut guard = self.agent.lock().await;
        *guard = new_agent.agent;
        drop(guard);
        self.update_context_usage(ContextUsage::new(None, new_agent.max_context_tokens))
            .await;
        Ok(())
    }
}
