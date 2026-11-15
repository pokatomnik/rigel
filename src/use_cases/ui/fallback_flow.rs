use crate::entities::{
    application::input_event::KeyStroke, application::input_reaction::InputReaction,
    render_model::modal::ModalView,
};

use super::workflow::UiWorkflow;

impl UiWorkflow {
    /// Handles printable text on the small-terminal fallback.
    pub(super) fn handle_fallback_text(&mut self, text: String) -> InputReaction {
        if matches!(
            self.state.active_modal(),
            Some(ModalView::ConfirmExit { .. })
        ) {
            return self.handle_exit_text(text.as_str());
        }
        InputReaction::consumed()
    }

    /// Handles one key on the small-terminal fallback.
    pub(super) fn handle_fallback_key(&mut self, key: KeyStroke) -> InputReaction {
        if matches!(
            self.state.active_modal(),
            Some(ModalView::ConfirmExit { .. })
        ) {
            return self.handle_exit_key(key);
        }
        match key {
            KeyStroke::Escape => self.request_stop(),
            _ => InputReaction::consumed(),
        }
    }
}
