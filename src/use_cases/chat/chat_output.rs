use rig::completion::CompletionModel;

use crate::shared::{
    agent::dependencies::ConfiguredAgent,
    recovery::recovery_error::format_recovery_stopped_notice,
    streaming::{message_output::display_message, stream_output_state::StreamOutputState},
};

use super::chat::Chat;

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: crate::shared::history::history_persistence::HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<ConfiguredAgent<CM>> + 'static,
{
    pub(super) fn echo_prompt(&self, prompt: &str, echo: bool) {
        if echo {
            self.terminal_io.print(format!("{prompt}\n").as_str());
        }
    }

    pub(super) fn report_turn_status(
        &self,
        status: &crate::shared::recovery::turn_recovery::TurnStatus,
    ) {
        if let Some(error) = status.error() {
            let notice = format_recovery_stopped_notice(error);
            self.terminal_io.eprintln(format!("\n{notice}").as_str());
        }
    }

    pub(super) async fn display_history(&self) {
        let messages = self.history.snapshot().await;
        if messages.is_empty() {
            return;
        }

        let mut output_state = StreamOutputState::default();
        for message in &messages {
            if display_message(self.terminal_io.as_ref(), &mut output_state, message) {
                self.terminal_io.eprintln("");
            }
        }
    }
}
