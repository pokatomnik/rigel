use std::sync::Arc;

use rig::completion::CompletionModel;

use crate::shared::{
    agent::dependencies::ConfiguredAgent,
    goal::goal_state::{GoalState, Report},
};

use super::chat::Chat;

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: crate::shared::history::history_persistence::HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<ConfiguredAgent<CM>> + 'static,
{
    pub(crate) fn with_goal_state(mut self, goal_state: Arc<GoalState>) -> Self {
        self.goal_state = Some(goal_state);
        self
    }

    pub(super) fn take_goal_report(&self) -> Option<Report> {
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
        Ok(crate::prompts::goal::goal_follow_up(goal.as_str()))
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
