use crate::entities::goal::goal_state::GoalReport;

use super::workflow::Chat;

impl Chat {
    pub(super) fn take_goal_report(&self) -> Option<GoalReport> {
        self.goal_state
            .as_ref()
            .and_then(|goal_state| goal_state.take_report())
    }

    fn activate_goal(&self, goal: String) -> anyhow::Result<()> {
        let goal_state = self
            .goal_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat"))?;
        goal_state.start(goal)?;
        Ok(())
    }

    fn next_goal_prompt(&self) -> anyhow::Result<String> {
        let goal_state = self
            .goal_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat"))?;
        if !goal_state.is_active() {
            return Err(anyhow::anyhow!(
                "active goal disappeared before its completion"
            ));
        }
        let goal = goal_state
            .current_goal()
            .ok_or_else(|| anyhow::anyhow!("active goal has no goal text"))?;
        let provider = self
            .prompt_provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("prompt provider was not configured"))?;
        Ok(provider.goal_follow_up(goal.as_str()))
    }

    pub(super) async fn pursue_goal(&self, goal: String) -> anyhow::Result<()> {
        self.activate_goal(goal.clone())?;
        let mut prompt = goal;
        loop {
            if self.run_prompt(prompt, false).await?.is_some() {
                return Ok(());
            }
            prompt = self.next_goal_prompt()?;
        }
    }

    pub(super) async fn handle_goal(&self, goal: String) -> anyhow::Result<bool> {
        self.pursue_goal(goal).await?;
        Ok(true)
    }
}
