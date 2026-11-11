use crate::entities::{
    application::{chat_action::ChatAction, input_outcome::InputOutcome},
    conversation::turn::HistoryUpdate,
};

use super::workflow::Chat;

impl Chat {
    pub(super) async fn next_command(&self) -> anyhow::Result<ChatAction> {
        let context_usage = *self.context_usage.lock().await;
        match self.io.readline(context_usage)? {
            InputOutcome::Text(input) => {
                let parser = self
                    .command_parser
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("chat command parser was not configured"))?;
                Ok(parser.parse(input.as_str(), false))
            }
            InputOutcome::Eof | InputOutcome::Cancelled => Ok(ChatAction::Exit),
        }
    }

    async fn handle_prompt(&self, prompt: String, echo: bool) -> anyhow::Result<bool> {
        self.run_prompt(prompt, echo).await?;
        Ok(true)
    }

    pub(super) async fn handle_new(&self) -> anyhow::Result<bool> {
        self.history_updated(HistoryUpdate::Replace(Vec::new()))
            .await?;
        self.reset_used_tokens().await;
        Ok(true)
    }

    async fn handle_agent_config(&self) -> anyhow::Result<bool> {
        self.handle_change_model().await?;
        Ok(true)
    }

    fn handle_unknown(&self) -> anyhow::Result<bool> {
        self.io
            .eprintln_orange("Unknown command. Type /help to get a list of available commands.");
        Ok(true)
    }

    async fn handle_editor(&self) -> anyhow::Result<bool> {
        let input = self.io.editor()?;
        let parser = self
            .command_parser
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat command parser was not configured"))?;
        let action = parser.parse(input.as_str(), true);
        if !matches!(action, ChatAction::Prompt { .. }) {
            self.io.print(format!("{input}\n").as_str());
        }
        self.handle_editor_action(action).await
    }

    fn handle_help(&self) -> anyhow::Result<bool> {
        for line in [
            "Available commands:",
            "/exit - Exit the chat",
            "/skills - Get a skill's instructions (/skill is an alias)",
            "/help - Show this help message",
            "/new - forget everything and start from the beginning",
            "/editor - Open default editor to type your prompt",
            "/compact - compact dialog context",
            "/agent - change agent preferences",
            "/goal <text> - pursue a goal until it is completed",
        ] {
            self.io.eprintln(line);
        }
        Ok(true)
    }

    async fn handle_skills(&self) -> anyhow::Result<bool> {
        let repository = self
            .skill_repository
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("skill repository was not configured"))?;
        let skills = repository.discover().await?;
        if skills.is_empty() {
            self.io.eprintln("No skills found");
            return Ok(true);
        }
        let Some(index) = self.io.select_skill(skills.as_slice())? else {
            self.io.eprintln("No skills found");
            return Ok(true);
        };
        let Some(selected) = skills.get(index) else {
            self.io.eprintln("No skills found");
            return Ok(true);
        };
        let instructions = repository.instructions(selected).await?;
        self.handle_prompt(instructions, false).await
    }

    async fn handle_editor_action(&self, command: ChatAction) -> anyhow::Result<bool> {
        match command {
            ChatAction::Exit => Ok(false),
            ChatAction::Continue => Ok(true),
            ChatAction::Prompt { text, echo } => self.handle_prompt(text, echo).await,
            ChatAction::Goal(goal) => self.handle_goal(goal).await,
            ChatAction::New => self.handle_new().await,
            ChatAction::Compact => self.handle_compaction().await.map(|_| true),
            ChatAction::ChangeModel => self.handle_agent_config().await,
            ChatAction::Help => self.handle_help(),
            ChatAction::Editor | ChatAction::Skills | ChatAction::Unknown => Ok(true),
        }
    }

    pub(super) async fn handle_command(&self, command: ChatAction) -> anyhow::Result<bool> {
        match command {
            ChatAction::Exit => Ok(false),
            ChatAction::Continue => Ok(true),
            ChatAction::Prompt { text, echo } => self.handle_prompt(text, echo).await,
            ChatAction::Goal(goal) => self.handle_goal(goal).await,
            ChatAction::New => self.handle_new().await,
            ChatAction::Compact => self.handle_compaction().await.map(|_| true),
            ChatAction::ChangeModel => self.handle_agent_config().await,
            ChatAction::Help => self.handle_help(),
            ChatAction::Editor => self.handle_editor().await,
            ChatAction::Skills => self.handle_skills().await,
            ChatAction::Unknown => self.handle_unknown(),
        }
    }
}
