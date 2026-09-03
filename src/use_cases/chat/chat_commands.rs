use rig::completion::CompletionModel;

use crate::shared::terminal::command_parser::CommandParserResult;

use super::chat::Chat;

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: crate::shared::history::history_persistence::HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<crate::shared::agent::agent::ConfiguredAgent<CM>> + 'static,
{
    pub(super) async fn next_command(&self) -> anyhow::Result<CommandParserResult> {
        let context_usage = *self.context_usage.lock().await;
        let input = self.terminal_io.readline(context_usage)?;
        Ok(self.command_parser.parse(input).await)
    }

    async fn handle_prompt(&self, prompt: String, echo: bool) -> anyhow::Result<bool> {
        self.run_prompt(prompt, echo).await?;
        Ok(true)
    }

    pub(super) async fn handle_new(&self) -> anyhow::Result<bool> {
        self.history_updated(
            crate::shared::history::chat_history::HistoryUpdate::Replace(Vec::new()),
        )
        .await?;
        self.reset_used_tokens().await;
        Ok(true)
    }

    async fn handle_agent_config(&self) -> anyhow::Result<bool> {
        self.handle_change_model().await?;
        Ok(true)
    }

    fn handle_unknown(&self) -> anyhow::Result<bool> {
        self.terminal_io
            .eprintln_orange("Unknown command. Type /help to get a list of available commands.");
        Ok(true)
    }

    pub(super) async fn handle_command(
        &self,
        command: CommandParserResult,
    ) -> anyhow::Result<bool> {
        match command {
            CommandParserResult::CommandExit => Ok(false),
            CommandParserResult::CommandContinue => Ok(true),
            CommandParserResult::Prompt(prompt, echo) => self.handle_prompt(prompt, echo).await,
            CommandParserResult::Goal(goal) => self.handle_goal(goal).await,
            CommandParserResult::New => self.handle_new().await,
            CommandParserResult::Compact => self.handle_compaction().await.map(|_| true),
            CommandParserResult::AgentConfig => self.handle_agent_config().await,
            CommandParserResult::Unknown => self.handle_unknown(),
        }
    }
}
