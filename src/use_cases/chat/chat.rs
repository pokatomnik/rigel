use std::sync::Arc;

use rig::{
    Agent,
    completion::{Chat as RigChat, CompletionModel},
    message::Message,
};
use tokio::sync::Mutex;

use crate::{
    shared::terminal_io::TerminalIO,
    use_cases::chat::{
        command_parser::{CommandParser, CommandParserResult},
        history_sync::{ChatHistory, HistoryPersistence, HistoryUpdate},
        recovery_error::format_recovery_stopped_notice,
        streamed_turn::{StreamRunOutcome, StreamedTurn},
        turn_recovery::{RecoveryRequest, TurnRecoverer, TurnStatus},
    },
};

/// Orchestrates commands and conversation turns between the user and the model.
pub(crate) struct Chat<CM, P, GM>
where
    CM: CompletionModel,
    P: HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<Agent<CM>> + 'static,
{
    agent: Arc<Mutex<Agent<CM>>>,
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<P>>,
    command_parser: CommandParser,
    change_model: GM,
}

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<Agent<CM>> + 'static,
{
    pub(crate) fn from_history(
        agent: Agent<CM>,
        terminal_io: Arc<TerminalIO>,
        history: Arc<ChatHistory<P>>,
        change_model: GM,
    ) -> Self {
        let command_parser = CommandParser::new(terminal_io.clone());
        Self {
            agent: Arc::new(Mutex::new(agent)),
            terminal_io,
            history,
            command_parser,
            change_model,
        }
    }

    fn streamed_turn(&self) -> StreamedTurn<'_, CM, P> {
        StreamedTurn::new(
            self.agent.clone(),
            self.terminal_io.as_ref(),
            self.history.as_ref(),
        )
    }

    async fn run_prompt(&self, prompt: String, echo: bool) -> anyhow::Result<()> {
        self.echo_prompt(prompt.as_str(), echo);
        let base = self.history.snapshot().await;
        let streamed_turn = self.streamed_turn();
        let outcome = streamed_turn.run(prompt, base).await?;
        let status = self.complete_user_turn(streamed_turn, outcome).await?;
        self.report_turn_status(status);
        self.terminal_io.eprintln("");
        Ok(())
    }

    async fn complete_user_turn(
        &self,
        streamed_turn: StreamedTurn<'_, CM, P>,
        outcome: StreamRunOutcome,
    ) -> anyhow::Result<TurnStatus> {
        let request = match outcome {
            StreamRunOutcome::Failed(error) => RecoveryRequest::StreamFailure(error),
            StreamRunOutcome::Completed(completion) if completion.requires_tool_recovery() => {
                RecoveryRequest::UnresolvedTool
            }
            StreamRunOutcome::Completed(completion) if completion.requires_answer_recovery() => {
                RecoveryRequest::MissingAnswer
            }
            StreamRunOutcome::Completed(_) => return Ok(TurnStatus::Complete),
        };

        TurnRecoverer::new(streamed_turn).recover(request).await
    }

    fn echo_prompt(&self, prompt: &str, echo: bool) {
        if echo {
            self.terminal_io.print(format!("{prompt}\n").as_str());
        }
    }

    fn report_turn_status(&self, status: TurnStatus) {
        if let TurnStatus::RecoveryStopped(error) = status {
            let notice = format_recovery_stopped_notice(error.as_str());
            self.terminal_io.eprintln(format!("\n{notice}").as_str());
        }
    }

    async fn history_updated(&self, update: HistoryUpdate) -> anyhow::Result<()> {
        self.history.update(update).await
    }

    async fn compact_context(&self, summarization: String) -> anyhow::Result<bool> {
        let mut messages = self.history.snapshot().await;
        let summarized = {
            let agent = self.agent.lock().await;
            agent.chat(summarization, &mut messages).await
        };
        let Ok(summarized) = summarized else {
            self.terminal_io.eprintln_gray("Compacting failed.");
            return Ok(false);
        };

        self.history_updated(HistoryUpdate::Replace(compacted_history(summarized)))
            .await?;
        Ok(true)
    }

    async fn handle_compaction(&self, summarization: String) -> anyhow::Result<()> {
        self.terminal_io.eprintln_gray("Compacting started...");
        if self.compact_context(summarization).await? {
            self.terminal_io.eprintln_gray("Compacting done.");
        }
        Ok(())
    }

    async fn next_command(&self) -> anyhow::Result<CommandParserResult> {
        let input = self.terminal_io.readline()?;
        Ok(self.command_parser.parse(input).await)
    }

    async fn handle_change_model(&self) -> anyhow::Result<()> {
        let new_agent = (self.change_model)().await?;
        let mut guard = self.agent.lock().await;
        *guard = new_agent;
        Ok(())
    }

    /// Runs the interactive loop until `/exit` or terminal input failure.
    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            match self.next_command().await? {
                CommandParserResult::CommandExit => break,
                CommandParserResult::CommandContinue => {}
                CommandParserResult::Prompt(prompt, echo) => {
                    self.run_prompt(prompt, echo).await?;
                }
                CommandParserResult::New => {
                    self.history_updated(HistoryUpdate::Replace(Vec::new()))
                        .await?;
                }
                CommandParserResult::Compact(summarization) => {
                    self.handle_compaction(summarization).await?;
                }
                CommandParserResult::ModelChange => {
                    self.handle_change_model().await?;
                }
            }
        }
        Ok(())
    }
}

fn compacted_history(summarized: String) -> Vec<Message> {
    vec![Message::assistant(summarized)]
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use anyhow::{Result, ensure};
    use rig::{
        Agent, AgentBuilder,
        message::Message,
        test_utils::{MockCompletionModel, MockTurn},
    };

    use super::{Chat, compacted_history};
    use crate::{
        prompts::summarization::summarization,
        shared::terminal_io::TerminalIO,
        use_cases::chat::history_sync::{ChatHistory, HistoryPersistence},
    };

    #[derive(Clone, Copy)]
    struct NoopPersistence;

    impl HistoryPersistence for NoopPersistence {
        fn save<'a>(
            &'a self,
            _messages: &'a [Message],
        ) -> futures::future::BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn compacted_history_keeps_summary_as_single_assistant_message() {
        let history = compacted_history("compressed summary".to_string());
        assert_eq!(history, vec![Message::assistant("compressed summary")]);
    }

    #[tokio::test]
    async fn compact_context_replaces_history_with_model_summary() -> Result<()> {
        let agent = AgentBuilder::new(MockCompletionModel::text("compressed summary")).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("first turn")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(agent, Arc::new(TerminalIO), history.clone(), async || {
            Ok(AgentBuilder::new(MockCompletionModel::text("replacement")).build())
        });

        ensure!(chat.compact_context(summarization().to_string()).await?);
        ensure!(
            history.snapshot().await == vec![Message::assistant("compressed summary")],
            "summary was not committed"
        );
        Ok(())
    }

    #[tokio::test]
    async fn compact_context_sends_complete_history_before_prompt() -> Result<()> {
        let model = MockCompletionModel::text("compressed summary");
        let source = vec![Message::user("first"), Message::assistant("second")];
        let history = Arc::new(ChatHistory::new(source.clone(), NoopPersistence));
        let agent = AgentBuilder::new(model.clone()).build();
        let chat = Chat::from_history(agent, Arc::new(TerminalIO), history, async || {
            Ok(AgentBuilder::new(MockCompletionModel::text("replacement")).build())
        });

        ensure!(chat.compact_context(summarization().to_string()).await?);
        let requests = model.requests();
        let [request] = requests.as_slice() else {
            anyhow::bail!("compaction must make exactly one model request");
        };
        let sent = request.chat_history.iter().cloned().collect::<Vec<_>>();
        let mut expected = source;
        expected.push(Message::user(summarization()));
        ensure!(sent == expected, "compaction request omitted chat history");
        Ok(())
    }

    #[tokio::test]
    async fn compact_context_keeps_history_when_model_run_fails() -> Result<()> {
        let model = MockCompletionModel::new([MockTurn::error("boom")]);
        let agent = AgentBuilder::new(model).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("keep me")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(agent, Arc::new(TerminalIO), history.clone(), async || {
            Ok(AgentBuilder::new(MockCompletionModel::text("replacement")).build())
        });

        ensure!(!chat.compact_context(summarization().to_string()).await?);
        ensure!(
            history.snapshot().await == vec![Message::user("keep me")],
            "failed compaction changed history"
        );
        Ok(())
    }

    #[tokio::test]
    async fn changing_model_replaces_agent_used_for_compaction() -> Result<()> {
        let initial = AgentBuilder::new(MockCompletionModel::text("old summary")).build();
        let replacement = AgentBuilder::new(MockCompletionModel::text("new summary")).build();
        let changed = Arc::new(AtomicBool::new(false));
        let changed_by_callback = changed.clone();
        let replacement_by_callback = replacement.clone();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("before model change")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(
            initial,
            Arc::new(TerminalIO),
            history.clone(),
            async move || {
                changed_by_callback.store(true, Ordering::SeqCst);
                Ok(replacement_by_callback.clone())
            },
        );

        chat.handle_change_model().await?;
        ensure!(changed.load(Ordering::SeqCst));
        ensure!(chat.compact_context(summarization().to_string()).await?);
        ensure!(
            history.snapshot().await == vec![Message::assistant("new summary")],
            "compaction used the old agent after model change"
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_model_change_preserves_current_agent() -> Result<()> {
        let initial = AgentBuilder::new(MockCompletionModel::text("current summary")).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("before failed model change")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(
            initial,
            Arc::new(TerminalIO),
            history.clone(),
            async || -> anyhow::Result<Agent<MockCompletionModel>> {
                anyhow::bail!("model selection failed")
            },
        );

        ensure!(chat.handle_change_model().await.is_err());
        ensure!(chat.compact_context(summarization().to_string()).await?);
        ensure!(
            history.snapshot().await == vec![Message::assistant("current summary")],
            "failed model change replaced the current agent"
        );
        Ok(())
    }
}
