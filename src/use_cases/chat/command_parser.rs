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
    CommandExit,
    CommandContinue,
    Prompt(String),
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

        CommandParserResult::CommandContinue
    }

    /// Discovers workspace skills, lets the user select one, and returns its instructions.
    ///
    /// Only direct subdirectories of `.agents/skills` containing a regular `SKILL.md` file are
    /// offered. Any discovery, selection, or read error is printed to the terminal and leaves the
    /// chat waiting for the next command.
    pub async fn get_skill(&self) -> CommandParserResult {
        let current_dir = match std::env::current_dir() {
            Ok(current_dir) => current_dir,
            Err(error) => {
                self.terminal_io.eprintln(
                    format!("Failed to locate the current workspace directory: {error}").as_str(),
                );
                return CommandParserResult::CommandContinue;
            }
        };

        let skills_dir = current_dir.join(SKILLS_DIRECTORY);
        let mut entries = match tokio::fs::read_dir(&skills_dir).await {
            Ok(entries) => entries,
            Err(error) => {
                self.terminal_io.eprintln(
                    format!("Failed to read skills directory `{SKILLS_DIRECTORY}`: {error}")
                        .as_str(),
                );
                return CommandParserResult::CommandContinue;
            }
        };

        let mut skills = Vec::new();
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    self.terminal_io.eprintln(
                        format!("Failed to read an entry in `{SKILLS_DIRECTORY}`: {error}")
                            .as_str(),
                    );
                    return CommandParserResult::CommandContinue;
                }
            };

            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(error) => {
                    self.terminal_io.eprintln(
                        format!("Failed to inspect `{}`: {error}", entry.path().display()).as_str(),
                    );
                    return CommandParserResult::CommandContinue;
                }
            };
            if !file_type.is_dir() {
                continue;
            }

            let manifest_path = entry.path().join(SKILL_MANIFEST);
            match tokio::fs::metadata(&manifest_path).await {
                Ok(metadata) if metadata.is_file() => skills.push(SkillOption {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    manifest_path,
                }),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    self.terminal_io.eprintln(
                        format!("Failed to inspect `{}`: {error}", manifest_path.display())
                            .as_str(),
                    );
                    return CommandParserResult::CommandContinue;
                }
            }
        }

        skills.sort_by(|left, right| left.name.cmp(&right.name));
        if skills.is_empty() {
            self.terminal_io.eprintln(
                format!(
                    "No skills containing `{SKILL_MANIFEST}` were found in \
                     `{SKILLS_DIRECTORY}`."
                )
                .as_str(),
            );
            return CommandParserResult::CommandContinue;
        }

        let selected = match self.terminal_io.fuzzy_select("Select skill", &skills) {
            Ok(selected) => selected,
            Err(error) => {
                self.terminal_io
                    .eprintln(format!("Failed to select a skill: {error}").as_str());
                return CommandParserResult::CommandContinue;
            }
        };

        match tokio::fs::read_to_string(&selected.manifest_path).await {
            Ok(prompt) => CommandParserResult::Prompt(prompt),
            Err(error) => {
                self.terminal_io.eprintln(
                    format!(
                        "Failed to read `{}` for skill `{}`: {error}",
                        selected.manifest_path.display(),
                        selected.name
                    )
                    .as_str(),
                );
                CommandParserResult::CommandContinue
            }
        }
    }

    pub fn new(terminal_io: Arc<TerminalIO>) -> Self {
        Self { terminal_io }
    }

    pub async fn parse(&self, raw_input: String) -> CommandParserResult {
        match raw_input.as_str() {
            "/exit" => CommandParserResult::CommandExit,
            "/skill" => self.get_skill().await,
            "/help" => self.handle_help(),
            _ => CommandParserResult::Prompt(raw_input),
        }
    }
}
