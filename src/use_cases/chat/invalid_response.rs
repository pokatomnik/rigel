use std::sync::Arc;

use rig::{
    agent::{AgentHook, HookContext, ModelTurnAction, ModelTurnFinished},
    message::AssistantContent,
};

use crate::shared::terminal_io::TerminalIO;

const MAX_RESPONSE_ATTEMPTS: usize = 10;
const STOP_SEQUENCES: &str = include_str!("invalid_response_sequences.txt");

pub(crate) struct InvalidResponseHook {
    terminal_io: Arc<TerminalIO>,
}

impl InvalidResponseHook {
    pub(crate) fn new(terminal_io: Arc<TerminalIO>) -> Self {
        Self { terminal_io }
    }
}

#[derive(Clone, Default)]
struct InvalidResponseState {
    rejected_attempts: usize,
}

impl AgentHook for InvalidResponseHook {
    async fn on_model_turn_finished(
        &self,
        ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        if invalid_sequence(event).is_none() {
            return ModelTurnAction::continue_run();
        }
        if has_tool_call(event) {
            return ModelTurnAction::stop("invalid response sequence appeared in a tool turn");
        }

        let attempts = record_rejection(ctx);
        if attempts < MAX_RESPONSE_ATTEMPTS {
            self.terminal_io
                .eprintln("\nModel made a mistake, retrying.");
            ModelTurnAction::repeat()
        } else {
            ModelTurnAction::stop(format!(
                "model produced an invalid response {MAX_RESPONSE_ATTEMPTS} times"
            ))
        }
    }
}

fn invalid_sequence(event: ModelTurnFinished<'_>) -> Option<&'static str> {
    let text = event
        .content
        .iter()
        .filter_map(answer_text)
        .collect::<String>();
    stop_sequences().find(|sequence| text.contains(sequence))
}

fn answer_text(content: &AssistantContent) -> Option<&str> {
    match content {
        AssistantContent::Text(text) => Some(text.text.as_str()),
        _ => None,
    }
}

fn has_tool_call(event: ModelTurnFinished<'_>) -> bool {
    event
        .content
        .iter()
        .any(|content| matches!(content, AssistantContent::ToolCall(_)))
}

fn record_rejection(ctx: &HookContext) -> usize {
    ctx.scratchpad().update::<InvalidResponseState, _>(|state| {
        state.rejected_attempts += 1;
        state.rejected_attempts
    })
}

fn stop_sequences() -> impl Iterator<Item = &'static str> {
    STOP_SEQUENCES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use rig::{OneOrMany, agent::ModelTurnFinished, completion::Usage, message::AssistantContent};

    use super::{invalid_sequence, stop_sequences};

    #[test]
    fn configured_sequences_are_non_empty() {
        assert!(stop_sequences().all(|sequence| !sequence.is_empty()));
    }

    #[test]
    fn invalid_answer_text_is_detected() {
        let content = OneOrMany::one(AssistantContent::text("before <|tool_call|> after"));
        let event = ModelTurnFinished {
            turn: 1,
            content: &content,
            usage: Usage::new(),
        };

        assert_eq!(invalid_sequence(event), Some("<|tool_call|>"));
    }

    #[test]
    fn reasoning_text_is_ignored() {
        let content = OneOrMany::one(AssistantContent::reasoning("<|tool_call|>"));
        let event = ModelTurnFinished {
            turn: 1,
            content: &content,
            usage: Usage::new(),
        };

        assert_eq!(invalid_sequence(event), None);
    }
}
