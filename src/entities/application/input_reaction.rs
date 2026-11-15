use crate::entities::agent_command::command::AgentCommand;

/// Presentation-only viewport movement requested by one input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollIntent {
    /// Moves the viewport one page towards older rows.
    PageUp,
    /// Moves the viewport one page towards newer rows.
    PageDown,
    /// Re-enables follow-tail and shows the newest rows.
    Tail,
}

/// Commands and composer effects produced by one terminal input event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputReaction {
    /// Commands to send across the event boundary.
    pub(crate) commands: Vec<AgentCommand>,
    /// Whether the terminal input should be applied to the composer editor.
    pub(crate) forward_to_composer: bool,
    /// Whether the composer must be cleared after a successful submission.
    pub(crate) clear_composer: bool,
    /// Viewport movement owned by the renderer rather than agent state.
    pub(crate) scroll: Option<ScrollIntent>,
}

impl InputReaction {
    /// Creates a reaction that performs no action.
    pub(crate) fn consumed() -> Self {
        Self {
            commands: Vec::new(),
            forward_to_composer: false,
            clear_composer: false,
            scroll: None,
        }
    }

    /// Creates a reaction that forwards the event to the composer.
    pub(crate) fn composer() -> Self {
        Self {
            commands: Vec::new(),
            forward_to_composer: true,
            clear_composer: false,
            scroll: None,
        }
    }

    /// Creates a reaction that only sends commands.
    pub(crate) fn commands(commands: Vec<AgentCommand>) -> Self {
        Self {
            commands,
            forward_to_composer: false,
            clear_composer: false,
            scroll: None,
        }
    }

    /// Creates a reaction that submits the draft and clears the composer.
    pub(crate) fn submitted(text: String) -> Self {
        Self {
            commands: vec![AgentCommand::SubmitPrompt { text }],
            forward_to_composer: false,
            clear_composer: true,
            scroll: None,
        }
    }

    /// Creates a reaction that only moves the transcript viewport.
    pub(crate) fn scroll(intent: ScrollIntent) -> Self {
        Self {
            commands: Vec::new(),
            forward_to_composer: false,
            clear_composer: false,
            scroll: Some(intent),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InputReaction;
    use crate::entities::agent_command::command::AgentCommand;

    #[test]
    fn submitted_reaction_clears_the_composer_and_sends_one_prompt() {
        let reaction = InputReaction::submitted("hello".to_string());

        assert!(reaction.clear_composer);
        assert_eq!(
            reaction.commands,
            [AgentCommand::SubmitPrompt {
                text: "hello".to_string()
            }]
        );
    }
}
