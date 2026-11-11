use crate::{
    entities::application::chat_action::ChatAction, infrastructure::application::ChatCommandParser,
};

/// Parses interactive slash commands without performing I/O, discovery, or output.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommandParser;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KnownCommand {
    Exit,
    Skills,
    Help,
    Editor,
    New,
    Compact,
    Agent,
    Goal,
}

impl KnownCommand {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "exit" => Some(Self::Exit),
            "skills" | "skill" => Some(Self::Skills),
            "help" => Some(Self::Help),
            "editor" => Some(Self::Editor),
            "new" => Some(Self::New),
            "compact" => Some(Self::Compact),
            "agent" => Some(Self::Agent),
            "goal" => Some(Self::Goal),
            _ => None,
        }
    }
}

impl ChatCommandParser for CommandParser {
    fn parse(&self, raw_input: &str, echo: bool) -> ChatAction {
        Self::parse(self, raw_input, echo)
    }
}

impl CommandParser {
    /// Parses one terminal line, preserving prompt text and editor echo intent.
    pub(crate) fn parse(&self, raw_input: &str, echo: bool) -> ChatAction {
        if raw_input.trim().is_empty() {
            return ChatAction::Continue;
        }
        let command_input = raw_input.trim_start();
        let Some((command_name, arguments)) = Self::command_parts(command_input) else {
            return ChatAction::Prompt {
                text: raw_input.to_string(),
                echo,
            };
        };
        match KnownCommand::from_name(command_name.as_str()) {
            Some(KnownCommand::Goal) => Self::goal_action(arguments),
            Some(KnownCommand::Exit) if arguments.is_none() => ChatAction::Exit,
            Some(KnownCommand::Skills) if arguments.is_none() => ChatAction::Skills,
            Some(KnownCommand::Help) if arguments.is_none() => ChatAction::Help,
            Some(KnownCommand::Editor) if arguments.is_none() => ChatAction::Editor,
            Some(KnownCommand::New) if arguments.is_none() => ChatAction::New,
            Some(KnownCommand::Compact) if arguments.is_none() => ChatAction::Compact,
            Some(KnownCommand::Agent) if arguments.is_none() => ChatAction::ChangeModel,
            _ => ChatAction::Unknown,
        }
    }

    fn goal_action(arguments: Option<&str>) -> ChatAction {
        match arguments.filter(|value| !value.trim().is_empty()) {
            Some(goal) => ChatAction::Goal(goal.to_string()),
            None => ChatAction::Continue,
        }
    }

    fn command_parts(input: &str) -> Option<(String, Option<&str>)> {
        let command = input.strip_prefix('/')?;
        let Some(separator) = command.find(char::is_whitespace) else {
            return Some((command.to_ascii_lowercase(), None));
        };
        let (name, arguments) = command.split_at(separator);
        Some((name.to_ascii_lowercase(), Some(arguments.trim_start())))
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandParser, KnownCommand};
    use crate::entities::application::chat_action::ChatAction;

    fn parser() -> CommandParser {
        CommandParser
    }

    #[test]
    fn compact_command_is_recognized() {
        assert_eq!(parser().parse("/compact", false), ChatAction::Compact);
    }

    #[test]
    fn plain_text_stays_a_prompt_without_echo() {
        assert_eq!(
            parser().parse("hello", false),
            ChatAction::Prompt {
                text: "hello".to_string(),
                echo: false
            }
        );
    }

    #[test]
    fn editor_goal_input_uses_the_regular_command_parser() {
        assert_eq!(
            parser().parse("/goal inspect  the repository carefully", true),
            ChatAction::Goal("inspect  the repository carefully".to_string())
        );
    }

    #[test]
    fn editor_plain_text_remains_an_echoed_prompt() {
        assert_eq!(
            parser().parse("describe the repository", true),
            ChatAction::Prompt {
                text: "describe the repository".to_string(),
                echo: true
            }
        );
    }

    #[test]
    fn blank_input_stays_in_the_loop_without_becoming_a_prompt() {
        for input in ["", " ", "\n", "\t\n"] {
            assert_eq!(
                parser().parse(input, false),
                ChatAction::Continue,
                "{input:?}"
            );
        }
    }

    #[test]
    fn exit_command_stops_the_chat() {
        assert_eq!(parser().parse("/exit", false), ChatAction::Exit);
    }

    #[test]
    fn new_command_clears_the_history() {
        assert_eq!(parser().parse("/new", false), ChatAction::New);
    }

    #[test]
    fn model_command_requests_model_change() {
        assert_eq!(parser().parse("/agent", false), ChatAction::ChangeModel);
    }

    #[test]
    fn goal_command_keeps_the_text_after_its_name() {
        assert_eq!(
            parser().parse("/goal inspect  the repository carefully", false),
            ChatAction::Goal("inspect  the repository carefully".to_string())
        );
    }

    #[test]
    fn goal_command_accepts_leading_whitespace_and_case_variants() {
        for input in [
            " /GOAL inspect the repository",
            "\t/GoAl inspect the repository",
        ] {
            assert_eq!(
                parser().parse(input, false),
                ChatAction::Goal("inspect the repository".to_string())
            );
        }
    }

    #[test]
    fn empty_goal_stays_in_the_command_loop() {
        for input in ["/goal", "/goal   ", "/goal\t"] {
            assert_eq!(parser().parse(input, false), ChatAction::Continue);
        }
    }

    #[test]
    fn help_command_accepts_leading_whitespace() {
        for input in ["/help", "\t/help", " /help", "\t /help"] {
            assert_eq!(parser().parse(input, false), ChatAction::Help);
        }
    }

    #[test]
    fn input_starting_with_exclamation_before_slash_stays_a_prompt() {
        assert!(matches!(
            parser().parse("\t!/help", false),
            ChatAction::Prompt { .. }
        ));
    }

    #[test]
    fn unknown_command_accepts_leading_space_but_is_not_a_prompt() {
        for input in ["/unknown", " /unknown"] {
            assert_eq!(
                parser().parse(input, false),
                ChatAction::Unknown,
                "{input:?}"
            );
        }
    }

    #[test]
    fn command_names_are_case_insensitive() {
        for command in ["/SKILLS", "/SkIlLs"] {
            let Some((name, _)) = CommandParser::command_parts(command) else {
                panic!("expected a valid command name");
            };
            assert_eq!(
                KnownCommand::from_name(name.as_str()),
                Some(KnownCommand::Skills)
            );
        }
    }

    #[test]
    fn skills_alias_uses_the_same_command_path() {
        let Some((skills, _)) = CommandParser::command_parts("/skills") else {
            panic!("expected a valid command name");
        };
        let Some((skill, _)) = CommandParser::command_parts("/skill") else {
            panic!("expected a valid command name");
        };
        assert_eq!(
            KnownCommand::from_name(skills.as_str()),
            KnownCommand::from_name(skill.as_str())
        );
    }

    #[test]
    fn slash_prefixed_input_returns_unknown() {
        for input in [
            "/skill-other extra",
            "/",
            "/name_2",
            "/name__two",
            "/name--two",
            "/helps!",
        ] {
            assert_eq!(parser().parse(input, false), ChatAction::Unknown, "{input}");
        }
    }

    #[test]
    fn unknown_slash_input_with_text_returns_unknown() {
        assert_eq!(
            parser().parse("/nonexistent foo", false),
            ChatAction::Unknown
        );
    }

    #[test]
    fn existing_commands_with_arguments_remain_unknown() {
        for input in ["/help extra", "/new extra", "/agent extra"] {
            assert_eq!(parser().parse(input, false), ChatAction::Unknown, "{input}");
        }
    }

    #[test]
    fn unknown_command_returns_unknown() {
        assert_eq!(parser().parse("/nonexistent", false), ChatAction::Unknown);
    }

    #[test]
    fn leading_spaces_do_not_hide_command_intent() {
        assert_eq!(parser().parse("  /helps!", false), ChatAction::Unknown);
    }
}
