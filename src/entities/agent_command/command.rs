use serde::{Deserialize, Serialize};

use crate::entities::tool_confirm_result::outcome::ToolConfirmResult;

/// Intent sent from the UI to the agent side of the in-process boundary.
///
/// A command only requests an action; the authoritative result arrives later as an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AgentCommand {
    /// Sends one prompt to the agent.
    SubmitPrompt {
        /// Exact prompt text produced by the composer.
        text: String,
    },
    /// Sends the instructions of one selected skill as a prompt.
    UseSkill {
        /// Skill identity selected in the palette.
        skill: crate::entities::skill::definition::SkillDescriptor,
    },
    /// Requests a soft stop at the next safe boundary.
    Stop,
    /// Requests manual context compaction.
    Compact,
    /// Deletes the current history after the UI has confirmed the action.
    NewConversation,
    /// Replaces the session model after the user selects it in the palette.
    SelectModel {
        /// Complete configured model, including its context limit.
        model: crate::entities::selected_model::selection::SelectedModel,
    },
    /// Starts an autonomous goal.
    StartGoal {
        /// Goal text entered in the goal modal.
        goal: String,
    },
    /// Resolves one outstanding permission request.
    ResolvePermission {
        /// Correlation identifier from the original request.
        correlation_id: u64,
        /// Explicit user decision.
        decision: ToolConfirmResult,
    },
    /// Resolves one outstanding user question.
    AnswerQuestion {
        /// Correlation identifier from the original question.
        correlation_id: u64,
        /// Exact answer text entered or selected by the user.
        answer: String,
    },
    /// Cancels one outstanding user question without an answer.
    CancelQuestion {
        /// Correlation identifier from the original question.
        correlation_id: u64,
    },
    /// Retries a failed durable write.
    RetryPersistence,
    /// Retries loading the provider model catalog.
    RetryModelCatalog,
    /// Exits after the agent reaches a safe boundary.
    Exit,
}

#[cfg(test)]
mod tests {
    use crate::entities::tool_confirm_result::outcome::ToolConfirmResult;

    use super::AgentCommand;

    fn round_trip(command: &AgentCommand) -> anyhow::Result<AgentCommand> {
        let json = serde_json::to_value(command)?;
        Ok(serde_json::from_value(json)?)
    }

    #[test]
    fn prompt_and_model_commands_round_trip() -> anyhow::Result<()> {
        let commands = [
            AgentCommand::SubmitPrompt {
                text: "hello".to_string(),
            },
            AgentCommand::UseSkill {
                skill: crate::entities::skill::definition::SkillDescriptor::new(
                    "configure-rigel".to_string(),
                    crate::entities::skill::definition::SkillOrigin::Builtin,
                ),
            },
            AgentCommand::SelectModel {
                model: crate::entities::selected_model::selection::SelectedModel::new(
                    "model-a".to_string(),
                    Some(32_768),
                ),
            },
            AgentCommand::StartGoal {
                goal: "finish the task".to_string(),
            },
            AgentCommand::AnswerQuestion {
                correlation_id: 4,
                answer: "yes".to_string(),
            },
            AgentCommand::CancelQuestion { correlation_id: 5 },
        ];

        for command in commands {
            assert_eq!(round_trip(&command)?, command);
        }
        Ok(())
    }

    #[test]
    fn permission_command_keeps_the_explicit_decision() -> anyhow::Result<()> {
        let command = AgentCommand::ResolvePermission {
            correlation_id: 3,
            decision: ToolConfirmResult::AllowForSession,
        };

        assert_eq!(round_trip(&command)?, command);
        Ok(())
    }

    #[test]
    fn lifecycle_commands_have_stable_names() -> anyhow::Result<()> {
        for command in [
            AgentCommand::Stop,
            AgentCommand::Compact,
            AgentCommand::NewConversation,
            AgentCommand::RetryPersistence,
            AgentCommand::RetryModelCatalog,
            AgentCommand::Exit,
        ] {
            assert_eq!(round_trip(&command)?, command);
        }
        Ok(())
    }
}
