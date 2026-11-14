use crate::entities::{
    agent_command::command::AgentCommand,
    agent_event::{event::AgentEvent, status::AgentActivity},
    application::input_event::{InputEvent, KeyStroke, TerminalSize},
    application::input_reaction::{InputReaction, ScrollIntent},
    render_model::model::RenderModel,
};
use crate::infrastructure::ui::clipboard::Clipboard;
use std::sync::Arc;

use super::{palette::PaletteState, state::UiState};

/// Presentation workflow that translates input and agent events without mutating agent state.
pub(crate) struct UiWorkflow {
    /// Presentation state shared with the render model builder.
    pub(super) state: UiState,
    /// Whether the multiline editor owns focus.
    pub(super) composer_focused: bool,
    /// Whether the Send control owns focus.
    pub(super) submit_focused: bool,
    /// Clipboard capability used by palette copy actions.
    pub(super) clipboard: Option<Arc<dyn Clipboard>>,
    /// Whether the currently visible palette belongs to startup model selection.
    pub(super) startup_palette: bool,
    stop_sent: bool,
}

impl UiWorkflow {
    /// Creates an idle workflow for the given terminal size.
    pub(crate) fn new(size: TerminalSize) -> Self {
        Self {
            state: UiState::new(size),
            composer_focused: true,
            submit_focused: false,
            clipboard: None,
            startup_palette: false,
            stop_sent: false,
        }
    }

    /// Installs the clipboard capability used by palette copy actions.
    pub(crate) fn with_clipboard(mut self, clipboard: Arc<dyn Clipboard>) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    /// Sets the configured model list without opening the palette.
    #[cfg(test)]
    pub(crate) fn with_models(
        mut self,
        models: Vec<crate::entities::selected_model::selection::SelectedModel>,
    ) -> Self {
        self.state.set_models(models);
        self
    }

    /// Sets the discovered skills without opening the palette.
    pub(crate) fn with_skills(
        mut self,
        skills: Vec<crate::entities::skill::definition::SkillDescriptor>,
    ) -> Self {
        self.state.set_skills(skills);
        self
    }

    /// Opens the startup palette on the configured model list.
    pub(crate) fn with_startup_models(
        mut self,
        models: Vec<crate::entities::selected_model::selection::SelectedModel>,
    ) -> Self {
        self.state.set_models(models);
        self.open_startup_palette();
        self.startup_palette = true;
        self
    }

    /// Applies one immutable agent event and returns any UI-initiated command.
    pub(crate) fn apply_agent_event(&mut self, event: AgentEvent) -> Vec<AgentCommand> {
        if let AgentEvent::ActivityChanged(activity) = &event
            && matches!(activity, AgentActivity::Idle | AgentActivity::Running)
        {
            self.stop_sent = false;
        }
        self.state.apply(event);
        Vec::new()
    }

    /// Applies one terminal event, using the current draft for submissions.
    pub(crate) fn handle_input(&mut self, event: InputEvent, draft: &str) -> InputReaction {
        match event {
            InputEvent::Resize(size) => {
                self.state.resize(size);
                InputReaction::consumed()
            }
            InputEvent::Closed => InputReaction::commands(vec![AgentCommand::Exit]),
            InputEvent::Text(text) => self.handle_text(text, draft),
            InputEvent::Key(key) => self.handle_key(key, draft),
        }
    }

    /// Builds the immutable frame for the renderer.
    pub(crate) fn render_model(&self) -> RenderModel {
        self.state.render_model().with_composer(
            crate::entities::render_model::views::ComposerView {
                text: String::new(),
                focused: self.composer_focused,
                submit_focused: self.submit_focused,
            },
        )
    }

    /// Reports whether the agent completed its shutdown sequence.
    pub(crate) fn shutdown(&self) -> bool {
        self.state.shutdown()
    }

    /// Reports whether printable input belongs to the composer.
    pub(crate) fn composer_accepts_text(&self) -> bool {
        self.composer_focused && !self.submit_focused
    }

    fn handle_text(&mut self, text: String, draft: &str) -> InputReaction {
        if self.state.active_modal().is_some() {
            return self.handle_modal_text(text);
        }
        if self.state.is_fallback() {
            return InputReaction::consumed();
        }
        if self.state.palette().is_some() {
            return InputReaction::consumed();
        }
        if text == "/" && draft.is_empty() {
            self.open_palette();
            return InputReaction::consumed();
        }
        if self.composer_accepts_text() {
            InputReaction::composer()
        } else {
            InputReaction::consumed()
        }
    }

    fn handle_key(&mut self, key: KeyStroke, draft: &str) -> InputReaction {
        if matches!(key, KeyStroke::Ctrl('c') | KeyStroke::Ctrl('q')) {
            self.open_exit_confirmation();
            return InputReaction::consumed();
        }
        if self.state.active_modal().is_some() {
            return self.handle_modal_key(key);
        }
        if self.state.is_fallback() {
            return match key {
                KeyStroke::Escape => self.request_stop(),
                _ => InputReaction::consumed(),
            };
        }
        if self.state.palette().is_some() {
            return self.handle_palette_key(key);
        }
        match key {
            KeyStroke::Ctrl('k') => {
                self.open_palette();
                InputReaction::consumed()
            }
            KeyStroke::Escape => self.request_stop(),
            KeyStroke::Tab => {
                self.toggle_focus();
                InputReaction::consumed()
            }
            KeyStroke::Enter if self.submit_focused => self.submit(draft),
            KeyStroke::CtrlEnter | KeyStroke::CmdEnter => self.submit(draft),
            KeyStroke::Ctrl('o') => {
                self.state.toggle_last_block();
                InputReaction::consumed()
            }
            KeyStroke::PageUp => InputReaction::scroll(ScrollIntent::PageUp),
            KeyStroke::PageDown => InputReaction::scroll(ScrollIntent::PageDown),
            KeyStroke::Ctrl(_) => InputReaction::consumed(),
            _ if self.composer_accepts_text() => InputReaction::composer(),
            _ => InputReaction::consumed(),
        }
    }

    fn submit(&mut self, draft: &str) -> InputReaction {
        if draft.trim().is_empty() {
            return InputReaction::consumed();
        }
        self.state.clear_notices();
        self.state.clear_goal_status();
        InputReaction::submitted(draft.to_string())
    }

    fn request_stop(&mut self) -> InputReaction {
        if self.stop_sent
            || self.state.activity().is_idle()
            || self.state.activity() == AgentActivity::Stopping
        {
            return InputReaction::consumed();
        }
        self.stop_sent = true;
        InputReaction::commands(vec![AgentCommand::Stop])
    }

    fn toggle_focus(&mut self) {
        if self.submit_focused {
            self.submit_focused = false;
            self.composer_focused = true;
        } else {
            self.composer_focused = false;
            self.submit_focused = true;
        }
    }

    fn open_palette(&mut self) {
        let goal_active =
            self.state.goal().phase == crate::entities::agent_event::status::GoalPhase::Active;
        let palette = PaletteState::root(self.state.activity(), self.state.mcp_statuses())
            .with_models(self.state.models())
            .with_skills(self.state.skills())
            .with_current_model(self.state.current_model())
            .with_read_only(self.state.read_only_reason())
            .with_notices(self.state.notice_count())
            .with_goal_active(goal_active);
        self.startup_palette = false;
        self.state.set_palette(Some(palette));
    }

    /// Reopens the startup model list after a cancelled exit or nested navigation.
    pub(super) fn open_startup_palette(&mut self) {
        let models = self.state.models();
        let skills = self.state.skills();
        let current = self.state.current_model();
        self.state.set_palette(Some(
            PaletteState::startup(models, current).with_skills(skills),
        ));
    }

    fn open_exit_confirmation(&mut self) {
        if matches!(
            self.state.active_modal(),
            Some(crate::entities::render_model::modal::ModalView::ConfirmExit { .. })
        ) {
            return;
        }
        self.state
            .push_modal(crate::entities::render_model::modal::ModalView::confirm_exit());
    }
}

impl crate::infrastructure::ui::presenter::UiStateMachine for UiWorkflow {
    fn handle_input(&mut self, event: InputEvent, draft: &str) -> InputReaction {
        UiWorkflow::handle_input(self, event, draft)
    }

    fn apply_agent_event(&mut self, event: AgentEvent) -> Vec<AgentCommand> {
        UiWorkflow::apply_agent_event(self, event)
    }

    fn render_model(&self) -> RenderModel {
        UiWorkflow::render_model(self)
    }

    fn shutdown(&self) -> bool {
        UiWorkflow::shutdown(self)
    }
}

#[cfg(test)]
mod tests {
    use super::UiWorkflow;
    use crate::entities::{
        agent_event::{
            event::AgentEvent,
            payload::{InteractionOrigin, PermissionRequestView},
            status::AgentActivity,
        },
        application::input_event::{InputEvent, KeyStroke, TerminalSize},
    };

    fn workflow() -> UiWorkflow {
        UiWorkflow::new(TerminalSize::new(80, 24))
    }

    fn activate_palette_item(workflow: &mut UiWorkflow, id: &str) {
        let Some(index) = workflow
            .render_model()
            .palette
            .and_then(|palette| palette.items.iter().position(|item| item.id == id))
        else {
            return;
        };
        for _ in 0..index {
            workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        }
        workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
    }

    fn permission() -> PermissionRequestView {
        PermissionRequestView::new(
            7,
            "run_command".to_string(),
            serde_json::json!({"command": "pwd"}),
            InteractionOrigin::MainAgent,
        )
    }

    #[test]
    fn submit_chord_sends_the_exact_draft_and_clears_the_editor() {
        let mut workflow = workflow();
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::CtrlEnter), "hello");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::SubmitPrompt {
                    text: "hello".to_string()
                }
            ]
        );
        assert!(result.clear_composer);
    }

    #[test]
    fn escape_stops_a_running_agent_only_once() {
        let mut workflow = workflow();
        workflow.apply_agent_event(AgentEvent::ActivityChanged(AgentActivity::Running));
        let first = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");
        let second = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");

        assert_eq!(
            first.commands,
            [crate::entities::agent_command::command::AgentCommand::Stop]
        );
        assert!(second.commands.is_empty());
    }

    #[test]
    fn escape_in_the_permission_modal_denies_instead_of_stopping() {
        let mut workflow = workflow();
        workflow.apply_agent_event(AgentEvent::ActivityChanged(AgentActivity::Running));
        workflow.apply_agent_event(AgentEvent::PermissionRequested(permission()));
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::ResolvePermission {
                    correlation_id: 7,
                    decision:
                        crate::entities::tool_confirm_result::outcome::ToolConfirmResult::Deny,
                }
            ]
        );
    }

    #[test]
    fn ctrl_c_opens_and_confirms_exit_without_stopping_immediately() {
        let mut workflow = workflow();
        workflow.apply_agent_event(AgentEvent::ActivityChanged(AgentActivity::Running));
        let opened = workflow.handle_input(InputEvent::Key(KeyStroke::Ctrl('c')), "");
        assert!(opened.commands.is_empty());
        assert!(workflow.render_model().modal.is_some());

        let confirmed = workflow.handle_input(InputEvent::Text("y".to_string()), "");
        assert_eq!(
            confirmed.commands,
            [crate::entities::agent_command::command::AgentCommand::Exit]
        );
    }

    #[test]
    fn slash_opens_the_palette_only_when_the_draft_is_empty() {
        let mut workflow = workflow();
        let opened = workflow.handle_input(InputEvent::Text("/".to_string()), "");
        assert!(!opened.forward_to_composer);
        assert!(workflow.render_model().palette.is_some());

        let mut other = UiWorkflow::new(TerminalSize::new(80, 24));
        let typed = other.handle_input(InputEvent::Text("/".to_string()), "draft");
        assert!(typed.forward_to_composer);
    }

    #[test]
    fn palette_navigation_opens_a_nested_level_and_returns_back() {
        let mut workflow = workflow();
        workflow.handle_input(InputEvent::Key(KeyStroke::Ctrl('k')), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        let entered = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert!(entered.commands.is_empty());
        let model = workflow.render_model();
        let Some(palette) = model.palette else {
            return;
        };
        assert_eq!(
            palette.level,
            crate::entities::palette::level::PaletteLevel::Models
        );
    }

    fn selected_model(id: &str) -> crate::entities::selected_model::selection::SelectedModel {
        crate::entities::selected_model::selection::SelectedModel::new(id.to_string(), Some(1_024))
    }

    #[test]
    fn startup_palette_opens_on_models_and_escape_exits() {
        let mut workflow = UiWorkflow::new(TerminalSize::new(80, 24))
            .with_startup_models(vec![selected_model("model-a")]);
        let palette = workflow.render_model().palette;
        assert_eq!(
            palette.map(|palette| palette.level),
            Some(crate::entities::palette::level::PaletteLevel::Models)
        );

        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");
        assert_eq!(
            result.commands,
            [crate::entities::agent_command::command::AgentCommand::Exit]
        );
    }

    #[test]
    fn selecting_a_model_sends_its_complete_metadata() {
        let mut workflow = UiWorkflow::new(TerminalSize::new(80, 24))
            .with_startup_models(vec![selected_model("model-a"), selected_model("model-b")]);
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::SelectModel {
                    model: selected_model("model-b"),
                }
            ]
        );
        assert!(workflow.render_model().palette.is_none());
    }

    #[test]
    fn unconfigured_startup_palette_offers_only_exit() {
        let mut workflow =
            UiWorkflow::new(TerminalSize::new(80, 24)).with_startup_models(Vec::new());
        let items = workflow
            .render_model()
            .palette
            .map(|palette| palette.items)
            .unwrap_or_default();
        assert!(items.iter().any(|item| item.id == "models-unconfigured"));

        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");
        assert_eq!(
            result.commands,
            [crate::entities::agent_command::command::AgentCommand::Exit]
        );
    }

    #[test]
    fn goal_modal_rejects_empty_and_sends_non_empty_text() {
        let mut workflow = workflow();
        workflow.handle_input(InputEvent::Key(KeyStroke::Ctrl('k')), "");
        activate_palette_item(&mut workflow, "goal");
        assert!(matches!(
            workflow.render_model().modal,
            Some(crate::entities::render_model::modal::ModalView::Goal { .. })
        ));

        let empty = workflow.handle_input(InputEvent::Key(KeyStroke::CtrlEnter), "");
        assert!(empty.commands.is_empty());
        assert!(matches!(
            workflow.render_model().modal,
            Some(crate::entities::render_model::modal::ModalView::Goal { error: true, .. })
        ));

        workflow.handle_input(InputEvent::Text("finish the task".to_string()), "");
        let sent = workflow.handle_input(InputEvent::Key(KeyStroke::CtrlEnter), "");
        assert_eq!(
            sent.commands,
            [
                crate::entities::agent_command::command::AgentCommand::StartGoal {
                    goal: "finish the task".to_string(),
                }
            ]
        );
    }

    #[test]
    fn new_conversation_requires_confirmation() {
        let mut workflow = workflow();
        workflow.handle_input(InputEvent::Key(KeyStroke::Ctrl('k')), "");
        activate_palette_item(&mut workflow, "new");
        assert!(matches!(
            workflow.render_model().modal,
            Some(crate::entities::render_model::modal::ModalView::ConfirmNew { .. })
        ));

        let cancelled = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");
        assert!(cancelled.commands.is_empty());
        assert!(workflow.render_model().modal.is_none());
    }
}
