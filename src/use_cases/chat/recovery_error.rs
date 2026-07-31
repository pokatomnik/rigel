//! Converts Rig runtime errors into recovery data used by the interactive chat loop.

use rig::{agent::StreamingError, completion::PromptError, message::Message};

/// Model-facing error text together with the conversation state available at the failure point.
///
/// Some Rig errors carry a canonical history containing the attempted tool calls and their
/// results. Keeping that history allows the next user message to continue from the failed run
/// instead of losing its diagnostic context.
pub(crate) struct RecoveryErrorContext {
    pub(crate) message: String,
    pub(crate) chat_history: Option<Vec<Message>>,
}

/// Converts a non-streaming prompt error into data that can be retained for user-guided recovery.
///
/// The history is extracted only from variants that expose a canonical conversation state.
/// Provider and memory errors contain no recoverable history, so callers must retain their
/// existing history in those cases.
pub(crate) fn recovery_context_from_prompt_error(error: PromptError) -> RecoveryErrorContext {
    let message = error.to_string();
    let chat_history = match error {
        PromptError::MaxTurnsError { chat_history, .. }
        | PromptError::UnknownToolCall { chat_history, .. } => Some(*chat_history),
        PromptError::PromptCancelled { chat_history, .. } => Some(chat_history),
        _ => None,
    };

    RecoveryErrorContext {
        message,
        chat_history,
    }
}

/// Converts a streaming failure into the same recovery context used for prompt failures.
///
/// Prompt failures are unwrapped because they may contain the conversation accumulated during
/// the stream. Transport or provider streaming failures have only their displayable error text.
pub(crate) fn recovery_context_from_streaming_error(error: StreamingError) -> RecoveryErrorContext {
    match error {
        StreamingError::Prompt(error) => recovery_context_from_prompt_error(*error),
        error => RecoveryErrorContext {
            message: error.to_string(),
            chat_history: None,
        },
    }
}

/// Formats the terminal notice shown after automatic model recovery is exhausted.
///
/// The notice deliberately returns control to the user by explaining that the next input should
/// guide the model, while preserving `/exit` as the explicit way to leave the chat.
pub(crate) fn format_recovery_stopped_notice(error: &str) -> String {
    format!(
        "[automatic recovery stopped: {error}]\n\
         Review the tool errors above, then enter a new message with instructions for the model, \
         or /exit."
    )
}

#[cfg(test)]
mod tests {
    use super::{format_recovery_stopped_notice, recovery_context_from_prompt_error};
    use rig::{
        completion::PromptError,
        message::{Message, UserContent},
    };

    #[test]
    fn max_turns_error_preserves_history_for_user_follow_up() {
        let history = vec![
            Message::user("original request"),
            Message::user("tool error"),
        ];
        let error = PromptError::MaxTurnsError {
            max_turns: 12,
            chat_history: Box::new(history),
            prompt: Box::new(Message::user("undispatched prompt")),
        };

        let context = recovery_context_from_prompt_error(error);
        let messages = context
            .chat_history
            .expect("max-turns errors should expose their history");

        assert_eq!(
            context.message,
            "MaxTurnsError: reached max turns limit: 12"
        );
        assert_eq!(messages.len(), 2);
        assert!(matches!(
            &messages[0],
            Message::User { content }
                if matches!(content.first(), UserContent::Text(text) if text.text == "original request")
        ));
    }

    #[test]
    fn recovery_failure_tells_user_how_to_resume() {
        let message = format_recovery_stopped_notice("MaxTurnsError");

        assert!(message.contains("automatic recovery stopped: MaxTurnsError"));
        assert!(message.contains("enter a new message with instructions for the model"));
        assert!(message.contains("/exit"));
    }
}
