use crate::{
    adapters::model_runtime::chat::history::history_persistence::HistoryPersistence,
    entities::goal::goal_state::GoalReport,
};

use super::chat::Chat;

impl<P> Chat<P>
where
    P: HistoryPersistence,
{
    /// Runs one prompt through the opaque session and applies post-turn policies.
    pub(super) async fn run_prompt(
        &self,
        prompt: String,
        echo: bool,
    ) -> anyhow::Result<Option<GoalReport>> {
        self.echo_prompt(prompt.as_str(), echo);
        let context_usage = *self.context_usage.lock().await;
        let status = self
            .session
            .lock()
            .await
            .run_turn(prompt, context_usage)
            .await?;
        self.update_context_usage(status.context_usage()).await;
        self.compact_after_turn(&status).await?;
        self.report_turn_status(&status);
        self.terminal_io.eprintln("");
        Ok(self.take_goal_report())
    }
}
