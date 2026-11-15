use crate::entities::{
    agent_command::command::AgentCommand,
    application::input_event::KeyStroke,
    application::input_reaction::InputReaction,
    render_model::modal::{ExitChoice, ModalView},
    tool_confirm_result::outcome::ToolConfirmResult,
};

use super::workflow::UiWorkflow;

impl UiWorkflow {
    /// Handles printable text while a modal owns keyboard input.
    pub(super) fn handle_modal_text(&mut self, text: String) -> InputReaction {
        let Some(modal) = self.state.active_modal().cloned() else {
            return InputReaction::consumed();
        };
        match modal {
            ModalView::ConfirmExit { .. } => self.handle_exit_text(text.as_str()),
            ModalView::ConfirmNew { .. } => self.handle_new_text(text.as_str()),
            ModalView::Goal { .. } => self.handle_goal_text(text.as_str()),
            ModalView::Permission { .. } => self.handle_permission_text(text.as_str()),
            ModalView::Question {
                request, selected, ..
            } if is_free_form(&request.answers, selected) => {
                if let Some(ModalView::Question { input, .. }) = self.state.active_modal_mut() {
                    input.push_str(text.as_str());
                }
                InputReaction::consumed()
            }
            ModalView::Question { .. } => self.handle_question_text(text.as_str()),
        }
    }

    /// Handles one key while a modal owns keyboard input.
    pub(super) fn handle_modal_key(
        &mut self,
        key: crate::entities::application::input_event::KeyStroke,
    ) -> InputReaction {
        let Some(modal) = self.state.active_modal().cloned() else {
            return InputReaction::consumed();
        };
        match modal {
            ModalView::ConfirmExit { .. } => self.handle_exit_key(key),
            ModalView::ConfirmNew { .. } => self.handle_new_key(key),
            ModalView::Goal { .. } => self.handle_goal_key(key),
            ModalView::Permission { .. } => self.handle_permission_key(key),
            ModalView::Question { .. } => self.handle_question_key(key),
        }
    }

    /// Handles clipboard text without interpreting it as a key chord.
    pub(super) fn handle_modal_paste(&mut self, text: &str) -> InputReaction {
        let Some(modal) = self.state.active_modal().cloned() else {
            return InputReaction::consumed();
        };
        match modal {
            ModalView::Goal { .. } => self.handle_goal_text(text),
            ModalView::Question {
                request, selected, ..
            } if is_free_form(&request.answers, selected) => {
                if let Some(ModalView::Question { input, .. }) = self.state.active_modal_mut() {
                    input.push_str(text);
                }
                InputReaction::consumed()
            }
            _ => InputReaction::consumed(),
        }
    }

    /// Handles printable confirmation text for exiting the chat.
    pub(super) fn handle_exit_text(&mut self, text: &str) -> InputReaction {
        match text {
            "y" | "Y" => self.finish_exit(ExitChoice::Exit),
            "n" | "N" => self.finish_exit(ExitChoice::Cancel),
            "j" => self.handle_exit_key(KeyStroke::Down),
            "k" => self.handle_exit_key(KeyStroke::Up),
            "l" => self.handle_exit_key(KeyStroke::Enter),
            _ => InputReaction::consumed(),
        }
    }

    /// Handles one key in the exit confirmation.
    pub(super) fn handle_exit_key(
        &mut self,
        key: crate::entities::application::input_event::KeyStroke,
    ) -> InputReaction {
        match key {
            KeyStroke::Escape => self.finish_exit(ExitChoice::Cancel),
            key if key.confirms_modal() => {
                let choice = match self.state.active_modal() {
                    Some(ModalView::ConfirmExit { selected }) => *selected,
                    _ => ExitChoice::Cancel,
                };
                self.finish_exit(choice)
            }
            KeyStroke::Up
            | KeyStroke::Down
            | KeyStroke::Left
            | KeyStroke::Right
            | KeyStroke::Tab => {
                if let Some(ModalView::ConfirmExit { selected }) = self.state.active_modal_mut() {
                    *selected = selected.next();
                }
                InputReaction::consumed()
            }
            _ => InputReaction::consumed(),
        }
    }

    fn finish_exit(&mut self, choice: ExitChoice) -> InputReaction {
        self.state.pop_modal();
        match choice {
            ExitChoice::Cancel => {
                if self.startup_palette {
                    self.open_startup_palette();
                }
                InputReaction::consumed()
            }
            ExitChoice::Exit => InputReaction::commands(vec![AgentCommand::Exit]),
        }
    }

    fn handle_permission_key(
        &mut self,
        key: crate::entities::application::input_event::KeyStroke,
    ) -> InputReaction {
        use crate::entities::application::input_event::KeyStroke;
        match key {
            KeyStroke::Escape => self.resolve_permission(ToolConfirmResult::Deny),
            key if key.confirms_modal() => {
                let decision = self
                    .state
                    .active_modal()
                    .and_then(ModalView::permission_decision)
                    .unwrap_or_default();
                self.resolve_permission(decision)
            }
            KeyStroke::Up => self.move_permission(-1),
            KeyStroke::Down | KeyStroke::Tab => self.move_permission(1),
            KeyStroke::PageUp => self.scroll_permission(-5),
            KeyStroke::PageDown => self.scroll_permission(5),
            _ => InputReaction::consumed(),
        }
    }

    fn handle_permission_text(&mut self, text: &str) -> InputReaction {
        match text {
            "j" => self.move_permission(1),
            "k" => self.move_permission(-1),
            "l" => self.handle_permission_key(KeyStroke::Enter),
            _ => InputReaction::consumed(),
        }
    }

    fn move_permission(&mut self, delta: i32) -> InputReaction {
        if let Some(ModalView::Permission { selected, .. }) = self.state.active_modal_mut() {
            let len = ToolConfirmResult::ALL.len() as i32;
            *selected = ((*selected as i32 + delta).rem_euclid(len)) as usize;
        }
        InputReaction::consumed()
    }

    fn scroll_permission(&mut self, delta: i32) -> InputReaction {
        if let Some(ModalView::Permission { scroll, .. }) = self.state.active_modal_mut() {
            let next = i32::from(*scroll) + delta;
            *scroll = u16::try_from(next.max(0)).unwrap_or(u16::MAX);
        }
        InputReaction::consumed()
    }

    fn resolve_permission(&mut self, decision: ToolConfirmResult) -> InputReaction {
        let Some(id) = self
            .state
            .active_modal()
            .and_then(ModalView::correlation_id)
        else {
            return InputReaction::consumed();
        };
        self.state.pop_modal();
        InputReaction::commands(vec![AgentCommand::ResolvePermission {
            correlation_id: id,
            decision,
        }])
    }

    fn handle_question_key(
        &mut self,
        key: crate::entities::application::input_event::KeyStroke,
    ) -> InputReaction {
        let Some(ModalView::Question {
            request,
            selected,
            input,
        }) = self.state.active_modal().cloned()
        else {
            return InputReaction::consumed();
        };
        let free_form = is_free_form(&request.answers, selected);
        match key {
            KeyStroke::Escape => self.cancel_question(),
            key if key.confirms_modal() && free_form => self.finish_question(input),
            KeyStroke::Backspace if free_form => self.backspace_question(),
            KeyStroke::Up => self.move_question(-1),
            KeyStroke::Down | KeyStroke::Tab => self.move_question(1),
            key if key.confirms_modal() => self.choose_question_answer(),
            _ => InputReaction::consumed(),
        }
    }

    fn backspace_question(&mut self) -> InputReaction {
        if let Some(ModalView::Question { input, .. }) = self.state.active_modal_mut() {
            input.pop();
        }
        InputReaction::consumed()
    }

    fn handle_question_text(&mut self, text: &str) -> InputReaction {
        match text {
            "j" => self.move_question(1),
            "k" => self.move_question(-1),
            "l" => self.handle_question_key(KeyStroke::Enter),
            _ => InputReaction::consumed(),
        }
    }

    fn move_question(&mut self, delta: i32) -> InputReaction {
        if let Some(ModalView::Question {
            request, selected, ..
        }) = self.state.active_modal_mut()
        {
            let len = request.answers.len() as i32 + 1;
            *selected = ((*selected as i32 + delta).rem_euclid(len)) as usize;
        }
        InputReaction::consumed()
    }

    fn choose_question_answer(&mut self) -> InputReaction {
        let Some(ModalView::Question {
            request,
            selected,
            input,
        }) = self.state.active_modal().cloned()
        else {
            return InputReaction::consumed();
        };
        if selected < request.answers.len() {
            return self.finish_question(selected.to_string());
        }
        if let Some(ModalView::Question { input: value, .. }) = self.state.active_modal_mut() {
            value.clear();
        }
        let _ = input;
        InputReaction::consumed()
    }

    fn finish_question(&mut self, answer: String) -> InputReaction {
        if answer.trim().is_empty() {
            return InputReaction::consumed();
        }
        let Some(id) = self
            .state
            .active_modal()
            .and_then(ModalView::correlation_id)
        else {
            return InputReaction::consumed();
        };
        self.state.pop_modal();
        InputReaction::commands(vec![AgentCommand::AnswerQuestion {
            correlation_id: id,
            answer,
        }])
    }

    fn cancel_question(&mut self) -> InputReaction {
        let Some(id) = self
            .state
            .active_modal()
            .and_then(ModalView::correlation_id)
        else {
            return InputReaction::consumed();
        };
        self.state.pop_modal();
        InputReaction::commands(vec![AgentCommand::CancelQuestion { correlation_id: id }])
    }
}

fn is_free_form(answers: &[String], selected: usize) -> bool {
    answers.is_empty() || selected >= answers.len()
}

#[cfg(test)]
mod tests {
    use super::{UiWorkflow, is_free_form};
    use crate::entities::{
        agent_event::{
            event::AgentEvent,
            payload::{InteractionOrigin, QuestionView},
            status::AgentActivity,
        },
        application::input_event::{InputEvent, KeyStroke, TerminalSize},
    };

    fn workflow_with_question() -> UiWorkflow {
        let mut workflow = UiWorkflow::new(TerminalSize::new(80, 24));
        workflow.apply_agent_event(AgentEvent::ActivityChanged(AgentActivity::Running));
        workflow.apply_agent_event(AgentEvent::QuestionAsked(QuestionView::new(
            4,
            "Pick one".to_string(),
            vec!["first".to_string(), "second".to_string()],
            InteractionOrigin::MainAgent,
        )));
        workflow
    }

    #[test]
    fn selecting_an_answer_sends_its_index() {
        let mut workflow = workflow_with_question();
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "1".to_string(),
                }
            ]
        );
    }

    #[test]
    fn vim_keys_move_the_question_selection() {
        let mut workflow = workflow_with_question();
        workflow.handle_input(InputEvent::Text("j".to_string()), "");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "1".to_string(),
                }
            ]
        );
    }

    #[test]
    fn vim_l_confirms_the_question_selection() {
        let mut workflow = workflow_with_question();
        let result = workflow.handle_input(InputEvent::Text("l".to_string()), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "0".to_string(),
                }
            ]
        );
    }

    #[test]
    fn modified_enter_confirms_the_question_selection() {
        let mut workflow = workflow_with_question();
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::CtrlEnter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "0".to_string(),
                }
            ]
        );
    }

    #[test]
    fn shift_enter_confirms_the_question_selection() {
        let mut workflow = workflow_with_question();
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::ShiftEnter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "0".to_string(),
                }
            ]
        );
    }

    #[test]
    fn vim_letters_remain_text_in_free_form_mode() {
        let mut workflow = workflow_with_question();
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
        workflow.handle_input(InputEvent::Text("j".to_string()), "");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "j".to_string(),
                }
            ]
        );
    }

    #[test]
    fn pasted_text_is_appended_in_free_form_mode() {
        let mut workflow = workflow_with_question();
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
        workflow.handle_input(InputEvent::Paste("custom".to_string()), "");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "custom".to_string(),
                }
            ]
        );
    }

    #[test]
    fn something_else_switches_to_free_form_text() {
        let mut workflow = workflow_with_question();
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        let switched = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
        assert!(switched.commands.is_empty());

        workflow.handle_input(InputEvent::Text("custom".to_string()), "");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::AnswerQuestion {
                    correlation_id: 4,
                    answer: "custom".to_string(),
                }
            ]
        );
    }

    #[test]
    fn escape_cancels_the_question_without_stopping_the_agent() {
        let mut workflow = workflow_with_question();
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");

        assert_eq!(
            result.commands,
            [
                crate::entities::agent_command::command::AgentCommand::CancelQuestion {
                    correlation_id: 4
                }
            ]
        );
    }

    #[test]
    fn free_form_mode_is_detected_at_the_answer_boundary() {
        assert!(is_free_form(&[], 0));
        assert!(!is_free_form(&["one".to_string()], 0));
        assert!(is_free_form(&["one".to_string()], 1));
    }
}
