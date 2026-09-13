use crate::{
    adapters::model_runtime::chat::{
        history::history_persistence::HistoryPersistence,
        recovery::recovery_error::format_recovery_stopped_notice,
        streaming::{message_output::display_message, stream_output_state::StreamOutputState},
    },
    entities::conversation::turn::TurnStatus,
};

use super::chat::Chat;

impl<P> Chat<P>
where
    P: HistoryPersistence,
{
    pub(super) fn echo_prompt(&self, prompt: &str, echo: bool) {
        if echo {
            self.terminal_io.print(format!("{prompt}\n").as_str());
        }
    }

    pub(super) fn report_turn_status(&self, status: &TurnStatus) {
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
