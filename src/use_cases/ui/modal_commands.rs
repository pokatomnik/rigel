use crate::entities::{
    agent_command::command::AgentCommand,
    application::input_reaction::InputReaction,
    render_model::modal::{ExitChoice, ModalView},
};

use super::workflow::UiWorkflow;

impl UiWorkflow {
    /// Handles printable confirmation text for a new conversation.
    pub(super) fn handle_new_text(&mut self, text: &str) -> InputReaction {
        match text {
            "y" | "Y" => self.finish_new(ExitChoice::Exit),
            "n" | "N" => self.finish_new(ExitChoice::Cancel),
            _ => InputReaction::consumed(),
        }
    }

    /// Handles one key in the new-conversation confirmation.
    pub(super) fn handle_new_key(
        &mut self,
        key: crate::entities::application::input_event::KeyStroke,
    ) -> InputReaction {
        use crate::entities::application::input_event::KeyStroke;
        match key {
            KeyStroke::Escape => self.finish_new(ExitChoice::Cancel),
            KeyStroke::Enter => {
                let choice = match self.state.active_modal() {
                    Some(ModalView::ConfirmNew { selected }) => *selected,
                    _ => ExitChoice::Cancel,
                };
                self.finish_new(choice)
            }
            KeyStroke::Up | KeyStroke::Down | KeyStroke::Tab => {
                if let Some(ModalView::ConfirmNew { selected }) = self.state.active_modal_mut() {
                    *selected = selected.next();
                }
                InputReaction::consumed()
            }
            _ => InputReaction::consumed(),
        }
    }

    fn finish_new(&mut self, choice: ExitChoice) -> InputReaction {
        self.state.pop_modal();
        match choice {
            ExitChoice::Cancel => InputReaction::consumed(),
            ExitChoice::Exit => InputReaction::commands(vec![AgentCommand::NewConversation]),
        }
    }

    /// Appends pasted or typed text to the goal modal.
    pub(super) fn handle_goal_text(&mut self, text: &str) -> InputReaction {
        if let Some(ModalView::Goal { input, error }) = self.state.active_modal_mut() {
            input.push_str(text);
            *error = false;
        }
        InputReaction::consumed()
    }

    /// Handles one key in the goal modal, including newlines and submission.
    pub(super) fn handle_goal_key(
        &mut self,
        key: crate::entities::application::input_event::KeyStroke,
    ) -> InputReaction {
        use crate::entities::application::input_event::KeyStroke;
        match key {
            KeyStroke::Escape => {
                self.state.pop_modal();
                InputReaction::consumed()
            }
            KeyStroke::CtrlEnter | KeyStroke::CmdEnter => self.submit_goal(),
            KeyStroke::Enter => {
                if let Some(ModalView::Goal { input, .. }) = self.state.active_modal_mut() {
                    input.push('\n');
                }
                InputReaction::consumed()
            }
            KeyStroke::Backspace => {
                if let Some(ModalView::Goal { input, error }) = self.state.active_modal_mut() {
                    input.pop();
                    *error = false;
                }
                InputReaction::consumed()
            }
            _ => InputReaction::consumed(),
        }
    }

    fn submit_goal(&mut self) -> InputReaction {
        let Some(ModalView::Goal { input, .. }) = self.state.active_modal().cloned() else {
            return InputReaction::consumed();
        };
        if input.trim().is_empty() {
            if let Some(ModalView::Goal { error, .. }) = self.state.active_modal_mut() {
                *error = true;
            }
            return InputReaction::consumed();
        }
        self.state.pop_modal();
        InputReaction::commands(vec![AgentCommand::StartGoal { goal: input }])
    }
}
