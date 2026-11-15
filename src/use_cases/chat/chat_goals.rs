use crate::entities::goal::goal_state::GoalReport;

use super::workflow::Chat;

impl Chat {
    pub(super) fn take_goal_report(&self) -> Option<GoalReport> {
        self.goal_state
            .as_ref()
            .and_then(|goal_state| goal_state.take_report())
    }

    pub(super) fn activate_goal(&self, goal: String) -> anyhow::Result<()> {
        let goal_state = self
            .goal_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat"))?;
        goal_state.start(goal)?;
        Ok(())
    }

    pub(super) fn next_goal_prompt(&self) -> anyhow::Result<String> {
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

    /// Clears an active goal after a stop or model failure.
    pub(super) fn abandon_goal(&self) -> anyhow::Result<()> {
        let goal_state = self
            .goal_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat"))?;
        goal_state.cancel();
        self.emit(
            crate::entities::agent_event::event::AgentEvent::GoalChanged(
                crate::entities::agent_event::status::GoalView::inactive(),
            ),
        );
        Ok(())
    }
}
