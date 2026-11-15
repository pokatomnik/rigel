use crate::entities::{
    agent_command::command::AgentCommand, application::input_event::KeyStroke,
    application::input_reaction::InputReaction, application::input_reaction::ScrollIntent,
    palette::item::PaletteAction, render_model::modal::ModalView,
};

use super::workflow::UiWorkflow;

impl UiWorkflow {
    /// Handles one key while the command palette is open.
    pub(super) fn handle_palette_key(&mut self, key: KeyStroke) -> InputReaction {
        if matches!(key, KeyStroke::Escape | KeyStroke::Ctrl('k')) {
            self.state.set_palette(None);
            if self.startup_palette {
                return InputReaction::commands(vec![AgentCommand::Exit]);
            }
            return InputReaction::consumed();
        }
        match key {
            KeyStroke::Up => self.move_palette(-1),
            KeyStroke::Down | KeyStroke::Tab => self.move_palette(1),
            key if key.confirms_modal() => self.activate_palette(),
            _ => InputReaction::consumed(),
        }
    }

    /// Handles printable vim navigation while the palette is open.
    pub(super) fn handle_palette_text(&mut self, text: &str) -> InputReaction {
        match text {
            "j" => self.move_palette(1),
            "k" => self.move_palette(-1),
            "h" => self.leave_palette_if_nested(),
            "l" => self.activate_palette(),
            _ => InputReaction::consumed(),
        }
    }

    fn move_palette(&mut self, delta: i32) -> InputReaction {
        if let Some(palette) = self.state.palette_mut() {
            palette.move_selection(delta);
        }
        InputReaction::consumed()
    }

    fn activate_palette(&mut self) -> InputReaction {
        let Some(action) = self
            .state
            .palette()
            .and_then(super::palette::PaletteState::selected_action)
        else {
            return InputReaction::consumed();
        };
        match action {
            PaletteAction::Command(command) => {
                self.state.set_palette(None);
                if matches!(command, AgentCommand::SelectModel { .. }) {
                    self.startup_palette = false;
                }
                InputReaction::commands(vec![command])
            }
            PaletteAction::Open(level) => {
                if let Some(palette) = self.state.palette_mut() {
                    palette.open(level);
                }
                InputReaction::consumed()
            }
            PaletteAction::Back => self.leave_palette_level(),
            PaletteAction::ConfirmExit => {
                self.state.set_palette(None);
                self.state.push_modal(ModalView::confirm_exit());
                InputReaction::consumed()
            }
            PaletteAction::ConfirmNewConversation => {
                self.state.set_palette(None);
                self.state.push_modal(ModalView::confirm_new());
                self.startup_palette = false;
                InputReaction::consumed()
            }
            PaletteAction::OpenGoal => {
                self.state.set_palette(None);
                self.state.push_modal(ModalView::goal());
                self.startup_palette = false;
                InputReaction::consumed()
            }
            PaletteAction::JumpToLatest => {
                self.state.set_palette(None);
                self.startup_palette = false;
                InputReaction::scroll(ScrollIntent::Tail)
            }
            PaletteAction::Copy(target) => {
                self.state.set_palette(None);
                self.startup_palette = false;
                self.copy_to_clipboard(target);
                InputReaction::consumed()
            }
            PaletteAction::DismissNotices => {
                self.state.set_palette(None);
                self.startup_palette = false;
                self.state.clear_notices();
                InputReaction::consumed()
            }
            PaletteAction::RetryPersistence => {
                self.state.set_palette(None);
                self.startup_palette = false;
                InputReaction::commands(vec![AgentCommand::RetryPersistence])
            }
            PaletteAction::Info => InputReaction::consumed(),
        }
    }

    fn leave_palette_if_nested(&mut self) -> InputReaction {
        if self
            .state
            .palette()
            .is_some_and(|palette| palette.has_parent())
        {
            self.leave_palette_level()
        } else {
            InputReaction::consumed()
        }
    }

    fn leave_palette_level(&mut self) -> InputReaction {
        let returned = self
            .state
            .palette_mut()
            .is_some_and(super::palette::PaletteState::back);
        if !returned {
            self.state.set_palette(None);
            self.startup_palette = false;
        }
        InputReaction::consumed()
    }

    fn copy_to_clipboard(&mut self, target: crate::entities::palette::item::CopyTarget) {
        use crate::entities::agent_event::status::NoticeLevel;
        let Some(payload) = self.state.copy_payload(target) else {
            self.state.notice(NoticeLevel::Warning, "Nothing to copy.");
            return;
        };
        let Some(clipboard) = self.clipboard.as_ref() else {
            self.state
                .notice(NoticeLevel::Error, "Clipboard is not available.");
            return;
        };
        match clipboard.copy(&payload) {
            Ok(()) => self
                .state
                .notice(NoticeLevel::Success, "Copied to clipboard."),
            Err(error) => self.state.notice(
                NoticeLevel::Error,
                format!("Clipboard unavailable: {error}"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::UiWorkflow;
    use crate::entities::{
        agent_event::event::AgentEvent,
        application::input_event::{InputEvent, KeyStroke, TerminalSize},
        palette::level::PaletteLevel,
        transcript::entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
    };
    use crate::infrastructure::ui::clipboard::Clipboard;

    #[derive(Default)]
    struct FakeClipboard {
        copied: Mutex<Vec<String>>,
    }

    impl Clipboard for FakeClipboard {
        fn copy(&self, text: &str) -> anyhow::Result<()> {
            self.copied
                .lock()
                .map_err(|_| anyhow::anyhow!("poisoned"))
                .map(|mut copied| copied.push(text.to_string()))
        }
    }

    fn palette_workflow() -> UiWorkflow {
        let mut workflow = UiWorkflow::new(TerminalSize::new(80, 24)).with_models(vec![
            crate::entities::selected_model::selection::SelectedModel::new(
                "model-a".to_string(),
                Some(32_768),
            ),
        ]);
        workflow.handle_input(InputEvent::Key(KeyStroke::Ctrl('k')), "");
        workflow
    }

    fn with_answer(workflow: &mut UiWorkflow) {
        workflow.apply_agent_event(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            EntryId::new(1),
            TranscriptEntryKind::Assistant {
                text: "answer".to_string(),
                reasoning: None,
            },
        )));
    }

    fn select_copy_answer(workflow: &mut UiWorkflow) {
        activate_palette_item(workflow, "copy-answer");
    }

    fn activate_palette_item(workflow: &mut UiWorkflow, id: &str) {
        navigate_to_palette_item(workflow, id);
        workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
    }

    fn navigate_to_palette_item(workflow: &mut UiWorkflow, id: &str) {
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
    }

    #[test]
    fn escape_closes_the_palette_without_stopping_the_agent() {
        let mut workflow = palette_workflow();
        workflow.handle_input(InputEvent::Key(KeyStroke::Escape), "");
        assert!(workflow.render_model().palette.is_none());
    }

    #[test]
    fn vim_keys_navigate_and_page_keys_do_not_move_the_selection() {
        let mut workflow = palette_workflow();
        workflow.handle_input(InputEvent::Text("j".to_string()), "");
        assert_eq!(
            workflow
                .render_model()
                .palette
                .map(|palette| palette.selected),
            Some(1)
        );

        workflow.handle_input(InputEvent::Key(KeyStroke::PageDown), "");
        assert_eq!(
            workflow
                .render_model()
                .palette
                .map(|palette| palette.selected),
            Some(1)
        );

        workflow.handle_input(InputEvent::Text("k".to_string()), "");
        assert_eq!(
            workflow
                .render_model()
                .palette
                .map(|palette| palette.selected),
            Some(0)
        );
    }

    #[test]
    fn pasted_vim_letter_does_not_move_the_palette_selection() {
        let mut workflow = palette_workflow();
        workflow.handle_input(InputEvent::Paste("j".to_string()), "");

        assert_eq!(
            workflow
                .render_model()
                .palette
                .map(|palette| palette.selected),
            Some(0)
        );
    }

    #[test]
    fn vim_h_does_not_close_the_root_palette() {
        let mut workflow = palette_workflow();
        workflow.handle_input(InputEvent::Text("h".to_string()), "");

        assert_eq!(
            workflow.render_model().palette.map(|palette| palette.level),
            Some(PaletteLevel::Root)
        );
    }

    #[test]
    fn vim_l_opens_the_selected_level() {
        let mut workflow = palette_workflow();
        workflow.handle_input(InputEvent::Text("j".to_string()), "");
        workflow.handle_input(InputEvent::Text("l".to_string()), "");

        assert_eq!(
            workflow.render_model().palette.map(|palette| palette.level),
            Some(PaletteLevel::Models)
        );
    }

    #[test]
    fn back_item_returns_to_the_parent_level() {
        let mut workflow = palette_workflow();
        workflow.handle_input(InputEvent::Key(KeyStroke::Down), "");
        workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
        assert_eq!(
            workflow.render_model().palette.map(|palette| palette.level),
            Some(PaletteLevel::Models)
        );

        workflow.handle_input(InputEvent::Key(KeyStroke::Up), "");
        let _ = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");
        assert_eq!(
            workflow.render_model().palette.map(|palette| palette.level),
            Some(PaletteLevel::Root)
        );
    }

    #[test]
    fn exit_item_opens_confirmation_instead_of_sending_the_command() {
        let mut workflow = palette_workflow();
        navigate_to_palette_item(&mut workflow, "exit");
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert!(result.commands.is_empty());
        assert!(workflow.render_model().modal.is_some());
    }

    #[test]
    fn disabled_items_do_nothing_when_activated() {
        let mut workflow =
            UiWorkflow::new(TerminalSize::new(80, 24)).with_startup_models(Vec::new());
        let result = workflow.handle_input(InputEvent::Key(KeyStroke::Enter), "");

        assert!(result.commands.is_empty());
        assert!(workflow.render_model().palette.is_some());
    }

    #[test]
    fn copy_action_sends_the_full_payload_to_the_clipboard() -> anyhow::Result<()> {
        let clipboard = Arc::new(FakeClipboard::default());
        let mut workflow =
            UiWorkflow::new(TerminalSize::new(80, 24)).with_clipboard(clipboard.clone());
        with_answer(&mut workflow);
        workflow.handle_input(InputEvent::Key(KeyStroke::Ctrl('k')), "");
        select_copy_answer(&mut workflow);

        let copied = clipboard
            .copied
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?;
        assert_eq!(copied.as_slice(), ["answer"]);
        assert!(workflow.render_model().palette.is_none());
        Ok(())
    }

    #[test]
    fn copy_without_a_clipboard_reports_an_explicit_failure() {
        let mut workflow = palette_workflow();
        with_answer(&mut workflow);
        select_copy_answer(&mut workflow);

        let model = workflow.render_model();
        assert!(
            model
                .status
                .notices
                .iter()
                .any(|notice| notice.text.contains("Clipboard is not available"))
        );
    }
}
