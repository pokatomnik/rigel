use std::sync::Arc;

use rig::{Agent, completion::CompletionModel};
use tokio::sync::Mutex;

use crate::{
    entities::context_usage::ContextUsage,
    shared::{
        agent::dependencies::ConfiguredAgent,
        goal::goal_state::GoalState,
        history::{chat_history::ChatHistory, history_persistence::HistoryPersistence},
        terminal::{command_parser::CommandParser, terminal_io::TerminalIO},
    },
};

/// Orchestrates commands and conversation turns between the user and the model.
pub(crate) struct Chat<CM, P, GM>
where
    CM: CompletionModel,
    P: HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<ConfiguredAgent<CM>> + 'static,
{
    pub(super) agent: Arc<Mutex<Agent<CM>>>,
    pub(super) terminal_io: Arc<TerminalIO>,
    pub(super) history: Arc<ChatHistory<P>>,
    pub(super) command_parser: CommandParser,
    pub(super) change_model: GM,
    pub(super) goal_state: Option<Arc<GoalState>>,
    pub(super) context_usage: Mutex<ContextUsage>,
}

impl<CM, P, GM> Chat<CM, P, GM>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
    GM: AsyncFn() -> anyhow::Result<ConfiguredAgent<CM>> + 'static,
{
    pub(crate) fn from_history(
        configured_agent: ConfiguredAgent<CM>,
        terminal_io: Arc<TerminalIO>,
        history: Arc<ChatHistory<P>>,
        change_model: GM,
    ) -> Self {
        let command_parser = CommandParser::new(terminal_io.clone());
        Self {
            agent: Arc::new(Mutex::new(configured_agent.agent)),
            terminal_io,
            history,
            command_parser,
            change_model,
            goal_state: None,
            context_usage: Mutex::new(ContextUsage::new(None, configured_agent.max_context_tokens)),
        }
    }

    /// Runs the interactive loop until `/exit` or terminal input failure.
    pub async fn run(&self) -> anyhow::Result<()> {
        self.display_history().await;
        loop {
            if !self.handle_command(self.next_command().await?).await? {
                return Ok(());
            }
        }
    }
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
        completion::{
            CompletionError, CompletionModel as CompletionModelTrait, CompletionRequest,
            CompletionResponse,
        },
        message::{Message, ToolChoice, UserContent},
        streaming::StreamingCompletionResponse,
        test_utils::{MockCompletionModel, MockResponse, MockStreamEvent, MockTurn},
    };

    use super::Chat;
    use crate::{
        entities::context_usage::ContextUsage,
        prompts::summarization::summarization,
        shared::{
            agent::dependencies::ConfiguredAgent,
            goal::{goal_completion_hook::GoalCompletionHook, goal_state::GoalState},
            history::{
                chat_history::ChatHistory, history_persistence::HistoryPersistence,
                history_sync::HistorySyncHook,
            },
            recovery::turn_recovery::TurnStatus,
            terminal::terminal_io::TerminalIO,
        },
        tools::tool_mark_goal_complete::MarkGoalComplete,
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

    #[tokio::test]
    async fn compact_context_replaces_history_with_model_summary() -> Result<()> {
        let agent = AgentBuilder::new(MockCompletionModel::text("compressed summary")).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("first turn")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );

        chat.update_context_usage(ContextUsage::new(Some(800), Some(1_000)))
            .await;
        ensure!(chat.compact_context().await?);
        ensure!(
            history.snapshot().await == vec![Message::assistant("compressed summary")],
            "summary was not committed"
        );
        assert_eq!(
            *chat.context_usage.lock().await,
            ContextUsage::new(None, Some(1_000))
        );
        Ok(())
    }

    #[tokio::test]
    async fn compact_context_sends_complete_history_before_prompt() -> Result<()> {
        let model = MockCompletionModel::text("compressed summary");
        let source = vec![Message::user("first"), Message::assistant("second")];
        let history = Arc::new(ChatHistory::new(source.clone(), NoopPersistence));
        let agent = AgentBuilder::new(model.clone()).build();
        let chat = Chat::from_history(
            configured_agent(agent),
            Arc::new(TerminalIO),
            history,
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );

        ensure!(chat.compact_context().await?);
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
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("keep me")],
            NoopPersistence,
        ));
        let agent = AgentBuilder::new(model)
            .add_hook(HistorySyncHook::new(history.clone()))
            .build();
        let chat = Chat::from_history(
            configured_agent(agent),
            Arc::new(TerminalIO),
            history.clone(),
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );

        ensure!(!chat.compact_context().await?);
        ensure!(
            history.snapshot().await == vec![Message::user("keep me")],
            "failed compaction changed history"
        );
        Ok(())
    }

    #[tokio::test]
    async fn automatic_compaction_replaces_history_and_resets_used_tokens() -> Result<()> {
        let agent = AgentBuilder::new(MockCompletionModel::text("automatic summary")).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("long conversation")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );
        let status = TurnStatus::Complete {
            output: "answer".to_string(),
            context_usage: ContextUsage::new(Some(800), Some(1_000)),
        };

        chat.compact_after_turn(&status).await?;

        assert_eq!(
            history.snapshot().await,
            vec![Message::assistant("automatic summary")]
        );
        assert_eq!(
            *chat.context_usage.lock().await,
            ContextUsage::new(None, Some(1_000))
        );
        Ok(())
    }

    #[tokio::test]
    async fn completed_prompt_triggers_one_automatic_compaction_without_continuation() -> Result<()>
    {
        let stream_model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::text("answer"),
            MockStreamEvent::final_response_with_total_tokens(800),
        ]]);
        let summary_model = MockCompletionModel::text("automatic summary");
        let model = DualMockModel {
            stream_model: stream_model.clone(),
            completion_model: summary_model.clone(),
        };
        let agent = AgentBuilder::new(model).build();
        let history = Arc::new(ChatHistory::new(Vec::new(), NoopPersistence));
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(dual_text_model("replacement")).build(),
                ))
            },
        );

        chat.run_prompt("question".to_string(), false).await?;

        assert_eq!(
            history.snapshot().await,
            vec![Message::assistant("automatic summary")]
        );
        assert_eq!(stream_model.request_count(), 1);
        assert_eq!(summary_model.request_count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn unknown_context_usage_does_not_start_automatic_compaction() -> Result<()> {
        let model = MockCompletionModel::text("must not be used");
        let agent = AgentBuilder::new(model.clone()).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("keep this")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );
        let status = TurnStatus::Complete {
            output: "answer".to_string(),
            context_usage: ContextUsage::new(None, Some(1_000)),
        };

        chat.compact_after_turn(&status).await?;

        assert_eq!(model.request_count(), 0);
        assert_eq!(history.snapshot().await, vec![Message::user("keep this")]);
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
            configured_agent_with_context(initial, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            async move || {
                changed_by_callback.store(true, Ordering::SeqCst);
                Ok(configured_agent_with_context(
                    replacement_by_callback.clone(),
                    Some(2_000),
                ))
            },
        );

        chat.handle_change_model().await?;
        ensure!(changed.load(Ordering::SeqCst));
        assert_eq!(
            *chat.context_usage.lock().await,
            ContextUsage::new(None, Some(2_000))
        );
        ensure!(chat.compact_context().await?);
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
            configured_agent(initial),
            Arc::new(TerminalIO),
            history.clone(),
            async || -> anyhow::Result<ConfiguredAgent<MockCompletionModel>> {
                anyhow::bail!("model selection failed")
            },
        );

        ensure!(chat.handle_change_model().await.is_err());
        ensure!(chat.compact_context().await?);
        ensure!(
            history.snapshot().await == vec![Message::assistant("current summary")],
            "failed model change replaced the current agent"
        );
        Ok(())
    }

    #[tokio::test]
    async fn new_conversation_resets_used_tokens_but_keeps_model_limit() -> Result<()> {
        let agent = AgentBuilder::new(MockCompletionModel::text("summary")).build();
        let history = Arc::new(ChatHistory::new(
            vec![Message::user("old")],
            NoopPersistence,
        ));
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );
        chat.update_context_usage(ContextUsage::new(Some(800), Some(1_000)))
            .await;

        chat.handle_new().await?;

        assert_eq!(history.snapshot().await, Vec::<Message>::new());
        assert_eq!(
            *chat.context_usage.lock().await,
            ContextUsage::new(None, Some(1_000))
        );
        Ok(())
    }

    #[tokio::test]
    async fn pursue_goal_follows_up_and_allows_the_next_goal() -> Result<()> {
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::text("progress"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::tool_call(
                    "complete-a",
                    "mark_goal_complete",
                    serde_json::json!({"report": "goal A report"}),
                ),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::text("final A"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::tool_call(
                    "complete-b",
                    "mark_goal_complete",
                    serde_json::json!({"report": "goal B report"}),
                ),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::text("final B"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
        ]);
        let state = Arc::new(GoalState::new());
        let history = Arc::new(ChatHistory::new(Vec::new(), NoopPersistence));
        let agent = AgentBuilder::new(model.clone())
            .tool(MarkGoalComplete::new(state.clone()))
            .add_hook(GoalCompletionHook::new(state.clone()))
            .add_hook(HistorySyncHook::new(history.clone()))
            .default_max_turns(12)
            .build();
        let chat = Chat::from_history(
            configured_agent(agent),
            Arc::new(TerminalIO),
            history,
            async || {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        )
        .with_goal_state(state.clone());

        chat.pursue_goal("goal A".to_string()).await?;
        assert!(!state.is_active());
        assert!(!state.has_pending_report());
        assert_eq!(model.request_count(), 3);
        assert!(request_contains_user_text(
            &model.requests()[1],
            "Original goal:\ngoal A"
        ));
        assert_eq!(model.requests()[2].tool_choice, Some(ToolChoice::None));

        chat.pursue_goal("goal B".to_string()).await?;
        assert!(!state.is_active());
        assert_eq!(model.request_count(), 5);
        assert!(request_contains_user_text(&model.requests()[3], "goal B"));
        Ok(())
    }

    fn request_contains_user_text(
        request: &rig::completion::CompletionRequest,
        expected: &str,
    ) -> bool {
        request.chat_history.iter().any(|message| {
            matches!(
                message,
                Message::User { content }
                    if content.iter().any(|content| matches!(
                        content,
                        UserContent::Text(text) if text.text().contains(expected)
                    ))
            )
        })
    }

    fn configured_agent<M: CompletionModelTrait>(agent: Agent<M>) -> ConfiguredAgent<M> {
        ConfiguredAgent::from_agent(agent)
    }

    fn configured_agent_with_context<M: CompletionModelTrait>(
        agent: Agent<M>,
        max_context_tokens: Option<u64>,
    ) -> ConfiguredAgent<M> {
        ConfiguredAgent {
            agent,
            max_context_tokens,
        }
    }

    #[derive(Clone)]
    struct DualMockModel {
        stream_model: MockCompletionModel,
        completion_model: MockCompletionModel,
    }

    impl CompletionModelTrait for DualMockModel {
        type Response = MockResponse;
        type StreamingResponse = MockResponse;
        type Client = ();

        fn make(_: &Self::Client, _: impl Into<String>) -> Self {
            Self {
                stream_model: MockCompletionModel::default(),
                completion_model: MockCompletionModel::default(),
            }
        }

        async fn completion(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            self.completion_model.completion(request).await
        }

        async fn stream(
            &self,
            request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            self.stream_model.stream(request).await
        }
    }

    fn dual_text_model(text: &str) -> DualMockModel {
        DualMockModel {
            stream_model: MockCompletionModel::from_stream_turns([vec![
                MockStreamEvent::text(text),
                MockStreamEvent::final_response_with_total_tokens(1),
            ]]),
            completion_model: MockCompletionModel::text(text),
        }
    }
}
