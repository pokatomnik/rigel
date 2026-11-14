use crate::entities::{
    agent_command::command::AgentCommand, agent_event::event::AgentEvent,
    application::input_event::InputEvent, application::input_reaction::InputReaction,
    render_model::model::RenderModel,
};

/// Owns UI state and translates input and agent events without exposing the concrete workflow.
pub(crate) trait UiStateMachine: Send + Sync {
    /// Applies one terminal event and returns commands or composer effects.
    fn handle_input(&mut self, event: InputEvent, draft: &str) -> InputReaction;

    /// Applies one agent event and returns UI-initiated commands.
    fn apply_agent_event(&mut self, event: AgentEvent) -> Vec<AgentCommand>;

    /// Returns the immutable frame for the current state.
    fn render_model(&self) -> RenderModel;

    /// Reports whether the UI should stop after the agent shut down.
    fn shutdown(&self) -> bool;
}
