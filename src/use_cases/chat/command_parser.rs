use std::{fmt, path::PathBuf, sync::Arc};

use crate::shared::terminal_io::TerminalIO;

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

pub(crate) enum CommandParserResult {
    /// Ask to exit the chat.
    CommandExit,

    /// Continue requesting a command / prompt from the user.
    CommandContinue,

    /// A prompt was parsed from the user.
    Prompt(String, bool),

    /// Forget everything and start from the beginning.
    New,
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

        CommandParserResult::CommandContinue
    }

    /// Discovers workspace skills, lets the user select one, and returns its instructions.
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
        match raw_input.as_str() {
            "/exit" => CommandParserResult::CommandExit,
            "/skill" => self.get_skill().await,
            "/help" => self.handle_help(),
            "/editor" => self.handle_editor(),
            "/new" => CommandParserResult::New,
            _ => CommandParserResult::Prompt(raw_input, false),
        }
    }
}
