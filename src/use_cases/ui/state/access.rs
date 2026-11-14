use crate::entities::agent_event::status::{AgentActivity, GoalView, McpServerStatus};
use crate::entities::selected_model::selection::SelectedModel;

use super::UiState;

impl UiState {
    /// Returns the configured model list shown by the palette.
    pub(crate) fn models(&self) -> Vec<SelectedModel> {
        self.models.clone()
    }

    /// Replaces the configured model list used by the palette.
    pub(crate) fn set_models(&mut self, models: Vec<SelectedModel>) {
        self.models = models;
    }

    /// Returns the currently active model identifier.
    pub(crate) fn current_model(&self) -> Option<String> {
        self.model.clone()
    }

    /// Returns the discovered skills shown by the palette.
    pub(crate) fn skills(&self) -> Vec<crate::entities::skill::definition::SkillDescriptor> {
        self.skills.clone()
    }

    /// Replaces the discovered skill list used by the palette.
    pub(crate) fn set_skills(
        &mut self,
        skills: Vec<crate::entities::skill::definition::SkillDescriptor>,
    ) {
        self.skills = skills;
    }

    /// Returns the current goal snapshot.
    pub(crate) const fn goal(&self) -> &GoalView {
        &self.goal
    }

    /// Clears a completed goal indicator before a new user prompt.
    pub(crate) fn clear_goal_status(&mut self) {
        self.goal = GoalView::inactive();
    }
    /// Returns the current agent activity.
    pub(crate) const fn activity(&self) -> AgentActivity {
        self.activity
    }

    /// Returns the latest MCP server statuses.
    pub(crate) fn mcp_statuses(&self) -> Vec<McpServerStatus> {
        self.mcp.clone()
    }

    /// Returns the persistence failure reason when durable writes are blocked.
    pub(crate) fn read_only_reason(&self) -> Option<String> {
        self.read_only.reason.clone()
    }

    /// Returns the number of transient notices currently shown.
    pub(crate) fn notice_count(&self) -> usize {
        self.notices.len()
    }
}
