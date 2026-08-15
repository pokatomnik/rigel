use std::{fmt, path::PathBuf, sync::Arc};

use crate::{prompts::summarization::summarization, shared::terminal_io::TerminalIO};

const SKILLS_DIRECTORY: &str = ".agents/skills";
const SKILL_MANIFEST: &str = "SKILL.md";

struct SkillOption {
    name: String,
    manifest_path: PathBuf,
}

impl fmt::Display for SkillOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.name)
    }
}

#[derive(Debug)]
pub(crate) enum CommandParserResult {
    /// Ask to exit the chat.
    CommandExit,

    /// Continue requesting a command / prompt from the user.
    CommandContinue,

    /// A prompt was parsed from the user.
    Prompt(String, bool),

    /// Forget everything and start from the beginning.
    New,

    /// Context compact required
    Compact(String),

    /// Model change request
    AgentConfig,

    /// Unknown command — user typed `/something` but no matching command exists.
    Unknown,
}

pub(crate) struct CommandParser {
    terminal_io: Arc<TerminalIO>,
}

impl CommandParser {
    pub fn handle_help(&self) -> CommandParserResult {
        self.terminal_io.eprintln("Available commands:");
        self.terminal_io.eprintln("/exit - Exit the chat");
        self.terminal_io
            .eprintln("/skill - Get a skill's instructions");
        self.terminal_io.eprintln("/help - Show this help message");
        self.terminal_io
            .eprintln("/new - forget everything and start from the beginning");
        self.terminal_io
            .eprintln("/editor - Open default editor to type your prompt");
        self.terminal_io
            .eprintln("/compact - compact dialog context");
        self.terminal_io
            .eprintln("/agent - change agent preferences");

        CommandParserResult::CommandContinue
    }

    /// Discovers current directory skills, lets the user select one, and returns its instructions.
    ///
    /// Only direct subdirectories of `.agents/skills` containing a regular `SKILL.md` file are
    /// offered. When no skills are found or anything fails, `No skills found` is printed and the
    /// chat waits for the next command.
    pub async fn get_skill(&self) -> CommandParserResult {
        match self.select_skill().await {
            Some(prompt) => CommandParserResult::Prompt(prompt, false),
            None => {
                self.terminal_io.eprintln("No skills found");
                CommandParserResult::CommandContinue
            }
        }
    }

    /// Returns the manifest contents of a user-selected skill, or `None` when no skill can be
    /// discovered, selected, or read.
    async fn select_skill(&self) -> Option<String> {
        let current_dir = std::env::current_dir().ok()?;
        let skills_dir = current_dir.join(SKILLS_DIRECTORY);
        let mut entries = tokio::fs::read_dir(&skills_dir).await.ok()?;

        let mut skills = Vec::new();
        loop {
            let Some(entry) = entries.next_entry().await.ok()? else {
                break;
            };
            if !entry.file_type().await.ok()?.is_dir() {
                continue;
            }

            let manifest_path = entry.path().join(SKILL_MANIFEST);
            match tokio::fs::metadata(&manifest_path).await {
                Ok(metadata) if metadata.is_file() => skills.push(SkillOption {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    manifest_path,
                }),
                _ => {}
            }
        }

        skills.sort_by(|left, right| left.name.cmp(&right.name));
        if skills.is_empty() {
            return None;
        }

        let selected = self
            .terminal_io
            .fuzzy_select("Select skill", &skills)
            .ok()?;
        tokio::fs::read_to_string(&selected.manifest_path)
            .await
            .ok()
    }

    pub fn new(terminal_io: Arc<TerminalIO>) -> Self {
        Self { terminal_io }
    }

    pub fn handle_editor(&self) -> CommandParserResult {
        let result = self.terminal_io.editor().unwrap_or_default();
        CommandParserResult::Prompt(result, true)
    }

    pub async fn parse(&self, raw_input: String) -> CommandParserResult {
        match raw_input {
            _ if raw_input.starts_with("/exit") => CommandParserResult::CommandExit,
            _ if raw_input.starts_with("/skill") => self.get_skill().await,
            _ if raw_input.starts_with("/help") => self.handle_help(),
            _ if raw_input.starts_with("/editor") => self.handle_editor(),
            _ if raw_input.starts_with("/new") => CommandParserResult::New,
            _ if raw_input.starts_with("/compact") => {
                CommandParserResult::Compact(summarization().to_string())
            }
            _ if raw_input.starts_with("/agent") => CommandParserResult::AgentConfig,
            _ if raw_input.starts_with("/") => CommandParserResult::Unknown,
            _ => CommandParserResult::Prompt(raw_input, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CommandParser, CommandParserResult};
    use crate::{prompts::summarization::summarization, shared::terminal_io::TerminalIO};

    fn parser() -> CommandParser {
        CommandParser::new(Arc::new(TerminalIO))
    }

    #[tokio::test]
    async fn compact_command_carries_the_summarization_prompt() {
        let result = parser().parse("/compact".to_string()).await;

        let CommandParserResult::Compact(prompt) = result else {
            panic!("expected a compact command result");
        };
        assert_eq!(prompt, summarization());
    }

    #[tokio::test]
    async fn unknown_input_stays_a_prompt_without_echo() {
        let result = parser().parse("hello".to_string()).await;

        assert!(matches!(result, CommandParserResult::Prompt(_, false)));
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
    async fn unknown_command_returns_unknown_instead_of_leaking_as_prompt() {
        let result = parser().parse("/nonexistent".to_string()).await;

        assert!(matches!(result, CommandParserResult::Unknown));
    }

    #[tokio::test]
    async fn unknown_command_multiple_patterns_returns_unknown() {
        let result = parser().parse("/nonexistent foo".to_string()).await;

        assert!(matches!(result, CommandParserResult::Unknown));
    }

    #[tokio::test]
    async fn bare_slash_returns_unknown() {
        let result = parser().parse("/".to_string()).await;

        assert!(matches!(result, CommandParserResult::Unknown));
    }

    #[tokio::test]
    async fn plain_non_slash_input_returns_prompt() {
        let result = parser().parse("write hello.rs".to_string()).await;

        match result {
            CommandParserResult::Prompt(text, echo) => {
                assert_eq!(text, "write hello.rs");
                assert!(!echo);
            }
            other => panic!("expected a Prompt variant, got: {other:#?}"),
        }
    }
}
