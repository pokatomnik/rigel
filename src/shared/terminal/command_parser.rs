use std::sync::Arc;

use crate::shared::{skills::catalog::SkillCatalog, terminal::terminal_io::TerminalIO};

#[derive(Debug)]
pub(crate) enum CommandParserResult {
    /// Ask to exit the chat.
    CommandExit,

    /// Continue requesting a command / prompt from the user.
    CommandContinue,

    /// Stop because the terminal input reached EOF.
    InputEof,

    /// Stop because the user cancelled terminal input.
    InputCancelled,

    /// A prompt was parsed from the user.
    Prompt(String, bool),

    /// Forget everything and start from the beginning.
    New,

    /// Context compact required
    Compact,

    /// Model change request
    AgentConfig,

    /// Start pursuing a goal.
    Goal(String),

    /// Unknown command — user typed `/something` but no matching command exists.
    Unknown,
}

pub(crate) struct CommandParser {
    terminal_io: Arc<TerminalIO>,
    skill_catalog: SkillCatalog,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

impl CommandParser {
    pub fn handle_help(&self) -> CommandParserResult {
        self.terminal_io.eprintln("Available commands:");
        self.terminal_io.eprintln("/exit - Exit the chat");
        self.terminal_io
            .eprintln("/skills - Get a skill's instructions (/skill is an alias)");
        self.terminal_io.eprintln("/help - Show this help message");
        self.terminal_io
            .eprintln("/new - forget everything and start from the beginning");
        self.terminal_io
            .eprintln("/editor - Open default editor to type your prompt");
        self.terminal_io
            .eprintln("/compact - compact dialog context");
        self.terminal_io
            .eprintln("/agent - change agent preferences");
        self.terminal_io
            .eprintln("/goal <text> - pursue a goal until it is completed");

        CommandParserResult::CommandContinue
    }

    /// Discovers skills, lets the user select one, and returns its instructions.
    pub async fn get_skill(&self) -> CommandParserResult {
        let skills = self.skill_catalog.discover().await;
        if skills.is_empty() {
            self.terminal_io.eprintln("No skills found");
            return CommandParserResult::CommandContinue;
        }
        let Some(selected) = self
            .terminal_io
            .fuzzy_select("Select skill", skills.as_slice())
            .ok()
        else {
            self.terminal_io.eprintln("No skills found");
            return CommandParserResult::CommandContinue;
        };
        match selected.instructions().await {
            Ok(prompt) => CommandParserResult::Prompt(prompt, false),
            Err(_) => {
                self.terminal_io.eprintln("No skills found");
                CommandParserResult::CommandContinue
            }
        }
    }

    pub fn new(terminal_io: Arc<TerminalIO>) -> Self {
        Self {
            terminal_io,
            skill_catalog: SkillCatalog::from_environment(),
        }
    }

    pub fn handle_editor(&self) -> CommandParserResult {
        let result = self.terminal_io.editor().unwrap_or_default();
        CommandParserResult::Prompt(result, true)
    }

    pub async fn parse(&self, raw_input: String) -> CommandParserResult {
        if raw_input.trim().is_empty() {
            return CommandParserResult::CommandContinue;
        }
        let command_input = raw_input.trim_start();
        let Some((command_name, arguments)) = Self::command_parts(command_input) else {
            return CommandParserResult::Prompt(raw_input, false);
        };
        let command = KnownCommand::from_name(command_name.as_str());
        match command {
            Some(KnownCommand::Goal) => self.handle_goal(arguments),
            Some(_) if arguments.is_some() => CommandParserResult::Unknown,
            _ => self.handle_command(command).await,
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

    fn handle_goal(&self, arguments: Option<&str>) -> CommandParserResult {
        let Some(goal) = arguments.filter(|goal| !goal.trim().is_empty()) else {
            self.terminal_io
                .eprintln("Usage: /goal <text> (the goal text is required)");
            return CommandParserResult::CommandContinue;
        };
        CommandParserResult::Goal(goal.to_string())
    }

    async fn handle_command(&self, command: Option<KnownCommand>) -> CommandParserResult {
        match command {
            Some(KnownCommand::Exit) => CommandParserResult::CommandExit,
            Some(KnownCommand::Skills) => self.get_skill().await,
            Some(KnownCommand::Help) => self.handle_help(),
            Some(KnownCommand::Editor) => self.handle_editor(),
            Some(KnownCommand::New) => CommandParserResult::New,
            Some(KnownCommand::Compact) => CommandParserResult::Compact,
            Some(KnownCommand::Agent) => CommandParserResult::AgentConfig,
            Some(KnownCommand::Goal) => self.handle_goal(None),
            None => CommandParserResult::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CommandParser, CommandParserResult};
    use crate::shared::terminal::terminal_io::TerminalIO;

    fn parser() -> CommandParser {
        CommandParser::new(Arc::new(TerminalIO))
    }

    #[tokio::test]
    async fn compact_command_is_recognized() {
        let result = parser().parse("/compact".to_string()).await;

        assert!(matches!(result, CommandParserResult::Compact));
    }

    #[tokio::test]
    async fn plain_text_stays_a_prompt_without_echo() {
        let result = parser().parse("hello".to_string()).await;

        assert!(matches!(result, CommandParserResult::Prompt(_, false)));
    }

    #[tokio::test]
    async fn blank_input_stays_in_the_loop_without_becoming_a_prompt() {
        for input in ["", " ", "\n", "\t\n"] {
            let result = parser().parse(input.to_string()).await;

            assert!(
                matches!(result, CommandParserResult::CommandContinue),
                "{input:?}"
            );
        }
    }

    #[tokio::test]
    async fn exit_command_stops_the_chat() {
        let result = parser().parse("/exit".to_string()).await;

        assert!(matches!(result, CommandParserResult::CommandExit));
    }

    #[tokio::test]
    async fn new_command_clears_the_history() {
        let result = parser().parse("/new".to_string()).await;

        assert!(matches!(result, CommandParserResult::New));
    }

    #[tokio::test]
    async fn model_command_requests_model_change() {
        let result = parser().parse("/agent".to_string()).await;

        assert!(matches!(result, CommandParserResult::AgentConfig));
    }

    #[tokio::test]
    async fn goal_command_keeps_the_text_after_its_name() {
        let result = parser()
            .parse("/goal inspect  the repository carefully".to_string())
            .await;

        assert!(matches!(
            result,
            CommandParserResult::Goal(goal) if goal == "inspect  the repository carefully"
        ));
    }

    #[tokio::test]
    async fn goal_command_accepts_leading_whitespace_and_case_variants() {
        for input in [
            " /GOAL inspect the repository",
            "\t/GoAl inspect the repository",
        ] {
            let result = parser().parse(input.to_string()).await;

            assert!(
                matches!(result, CommandParserResult::Goal(goal) if goal == "inspect the repository")
            );
        }
    }

    #[tokio::test]
    async fn empty_goal_stays_in_the_command_loop() {
        for input in ["/goal", "/goal   ", "/goal\t"] {
            let result = parser().parse(input.to_string()).await;

            assert!(matches!(result, CommandParserResult::CommandContinue));
        }
    }

    #[tokio::test]
    async fn help_command_accepts_leading_whitespace() {
        for input in ["/help", "\t/help", " /help", "\t /help"] {
            let result = parser().parse(input.to_string()).await;

            assert!(
                matches!(result, CommandParserResult::CommandContinue),
                "{input:?}"
            );
        }
    }

    #[tokio::test]
    async fn input_starting_with_exclamation_before_slash_stays_a_prompt() {
        let result = parser().parse("\t!/help".to_string()).await;

        assert!(matches!(result, CommandParserResult::Prompt(_, false)));
    }

    #[tokio::test]
    async fn unknown_command_accepts_leading_space_but_is_not_a_prompt() {
        for input in ["/unknown", " /unknown"] {
            let result = parser().parse(input.to_string()).await;

            assert!(matches!(result, CommandParserResult::Unknown), "{input:?}");
        }
    }

    #[test]
    fn command_names_are_case_insensitive() {
        for command in ["/SKILLS", "/SkIlLs"] {
            let Some((name, _)) = CommandParser::command_parts(command) else {
                panic!("expected a valid command name");
            };

            assert_eq!(
                super::KnownCommand::from_name(name.as_str()),
                Some(super::KnownCommand::Skills)
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
            super::KnownCommand::from_name(skills.as_str()),
            super::KnownCommand::from_name(skill.as_str())
        );
        assert_eq!(
            super::KnownCommand::from_name(skills.as_str()),
            Some(super::KnownCommand::Skills)
        );
    }

    #[tokio::test]
    async fn slash_prefixed_input_returns_unknown() {
        for input in [
            "/skill-other extra",
            "/",
            "/name_2",
            "/name__two",
            "/name--two",
            "/helps!",
        ] {
            let result = parser().parse(input.to_string()).await;

            assert!(matches!(result, CommandParserResult::Unknown), "{input}");
        }
    }

    #[tokio::test]
    async fn unknown_command_returns_unknown() {
        let result = parser().parse("/nonexistent".to_string()).await;

        assert!(matches!(result, CommandParserResult::Unknown));
    }

    #[tokio::test]
    async fn unknown_slash_input_with_text_returns_unknown() {
        let result = parser().parse("/nonexistent foo".to_string()).await;

        assert!(matches!(result, CommandParserResult::Unknown));
    }

    #[tokio::test]
    async fn existing_commands_with_arguments_remain_unknown() {
        for input in ["/help extra", "/new extra", "/agent extra"] {
            let result = parser().parse(input.to_string()).await;

            assert!(matches!(result, CommandParserResult::Unknown), "{input}");
        }
    }

    #[tokio::test]
    async fn leading_spaces_do_not_hide_command_intent() {
        let result = parser().parse("  /helps!".to_string()).await;

        assert!(matches!(result, CommandParserResult::Unknown));
    }
}
