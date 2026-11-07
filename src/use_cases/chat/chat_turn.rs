use crate::entities::goal::goal_state::GoalReport;

use super::workflow::Chat;

impl Chat {
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
        self.io.eprintln("");
        Ok(self.take_goal_report())
    }
}
