use crate::entities::conversation::turn::TurnStatus;

use super::workflow::Chat;

impl Chat {
    pub(super) fn echo_prompt(&self, prompt: &str, echo: bool) {
        if echo {
            self.io.print(format!("{prompt}\n").as_str());
        }
    }

    pub(super) fn report_turn_status(&self, status: &TurnStatus) {
        if let Some(error) = status.error() {
            self.io
                .eprintln(format!("\n{}", format_recovery_stopped_notice(error)).as_str());
        }
    }

    pub(super) async fn display_history(&self) {
        let messages = self.history.snapshot().await;
        if !messages.is_empty() {
            self.io.display_history(messages.as_slice());
        }
    }
}

/// Formats the user-facing notice after automatic recovery stops.
///
/// The notice directs the user to provide guidance in a new turn or exit explicitly.
pub(crate) fn format_recovery_stopped_notice(error: &str) -> String {
    format!(
        "[automatic recovery stopped: {error}]\nReview the tool errors above, then enter a new message with instructions for the model, or /exit."
    )
}

#[cfg(test)]
mod tests {
    use super::format_recovery_stopped_notice;

    #[test]
    fn recovery_stopped_notice_tells_user_how_to_resume() {
        let message = format_recovery_stopped_notice("MaxTurnsError");

        assert!(message.contains("automatic recovery stopped: MaxTurnsError"));
        assert!(message.contains("enter a new message with instructions for the model"));
        assert!(message.contains("/exit"));
    }
}
