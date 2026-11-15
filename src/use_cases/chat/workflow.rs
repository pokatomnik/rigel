use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(test)]
use std::future::Future;

use tokio::sync::{Mutex, RwLock};

#[cfg(test)]
use crate::controllers::{
    model_runtime::history::{chat_history::ChatHistory, history_persistence::HistoryPersistence},
    terminal::terminal_io::TerminalIO,
};

#[cfg(test)]
use crate::prompts::provider::StaticPromptProvider;

use crate::entities::agent_command::queue::PromptQueue;

use crate::{
    entities::{conversation::context_usage::ContextUsage, goal::goal_state::GoalState},
    infrastructure::{
        conversation::ConversationHistory,
        model::{ModelSession, ModelSessionFactory},
        prompts::PromptProvider,
        skills::repository::SkillRepository,
        ui::{
            cancellation::RunCancellation,
            gateway::{AgentCommandSource, EventSink},
            interaction::InteractionBroker,
        },
    },
};

/// Orchestrates interactive chat policy through infrastructure traits only.
pub(crate) struct Chat {
    pub(super) session: Arc<Mutex<Box<dyn ModelSession>>>,
    pub(super) history: Arc<dyn ConversationHistory>,
    pub(super) model_factory: RwLock<Option<Arc<dyn ModelSessionFactory>>>,
    pub(super) goal_state: Option<Arc<GoalState>>,
    pub(super) skill_repository: Option<Arc<dyn SkillRepository>>,
    pub(super) prompt_provider: Option<Arc<dyn PromptProvider>>,
    pub(super) context_usage: Mutex<ContextUsage>,
    pub(super) command_source: Option<Arc<dyn AgentCommandSource>>,
    pub(super) events: Option<Arc<dyn EventSink>>,
    pub(super) interaction_broker: Option<Arc<dyn InteractionBroker>>,
    pub(super) cancellation: Option<Arc<dyn RunCancellation>>,
    pub(super) queue: Mutex<PromptQueue>,
    pub(super) next_entry_id: AtomicU64,
    pub(super) stop_requested: AtomicBool,
    pub(super) exit_requested: AtomicBool,
    pub(super) selected_model:
        RwLock<Option<crate::entities::selected_model::selection::SelectedModel>>,
}

impl Chat {
    /// Creates a chat from one built session and domain history.
    pub(crate) fn from_session(
        session: Box<dyn ModelSession>,
        history: Arc<dyn ConversationHistory>,
    ) -> Self {
        let max_context_tokens = session.max_context_tokens();
        let selected_model = session.selected_model();
        Self {
            session: Arc::new(Mutex::new(session)),
            history,
            model_factory: RwLock::new(None),
            goal_state: None,
            skill_repository: None,
            prompt_provider: None,
            context_usage: Mutex::new(ContextUsage::new(None, max_context_tokens)),
            command_source: None,
            events: None,
            interaction_broker: None,
            cancellation: None,
            queue: Mutex::new(PromptQueue::new()),
            next_entry_id: AtomicU64::new(1),
            stop_requested: AtomicBool::new(false),
            exit_requested: AtomicBool::new(false),
            selected_model: RwLock::new(selected_model),
        }
    }

    /// Attaches the factory used only after a complete replacement session is built.
    pub(crate) async fn with_model_session_factory(
        self,
        factory: Arc<dyn ModelSessionFactory>,
    ) -> Self {
        *self.model_factory.write().await = Some(factory);
        self
    }

    /// Attaches the repository used when the UI selects a skill in the palette.
    pub(crate) fn with_skill_repository(mut self, repository: Arc<dyn SkillRepository>) -> Self {
        self.skill_repository = Some(repository);
        self
    }

    /// Attaches prompts used for goal continuation and compaction requests.
    pub(crate) fn with_prompt_provider(mut self, provider: Arc<dyn PromptProvider>) -> Self {
        self.prompt_provider = Some(provider);
        self
    }

    /// Attaches shared goal state used by the goal workflow and completion hook.
    pub(crate) fn with_goal_state(mut self, goal_state: Arc<GoalState>) -> Self {
        self.goal_state = Some(goal_state);
        self
    }

    /// Attaches the command stream produced by the terminal UI.
    pub(crate) fn with_command_source(mut self, source: Arc<dyn AgentCommandSource>) -> Self {
        self.command_source = Some(source);
        self
    }

    /// Attaches the event boundary used for all UI-visible facts.
    pub(crate) fn with_event_sink(mut self, events: Arc<dyn EventSink>) -> Self {
        self.events = Some(events);
        self
    }

    /// Attaches the broker that resolves permission and question responses.
    pub(crate) fn with_interaction_broker(mut self, broker: Arc<dyn InteractionBroker>) -> Self {
        self.interaction_broker = Some(broker);
        self
    }

    /// Attaches the soft-stop flag shared with the model session.
    pub(crate) fn with_cancellation(mut self, cancellation: Arc<dyn RunCancellation>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Sets the first identifier allocated for live text entries.
    pub(crate) fn with_next_entry_id(mut self, next_entry_id: u64) -> Self {
        self.next_entry_id = AtomicU64::new(next_entry_id.max(1));
        self
    }

    /// Allocates the next live transcript entry identity.
    pub(super) fn allocate_entry_id(&self) -> crate::entities::transcript::entry::EntryId {
        crate::entities::transcript::entry::EntryId::new(
            self.next_entry_id.fetch_add(1, Ordering::SeqCst),
        )
    }

    /// Runs the event-driven interactive loop until shutdown.
    pub async fn run(&self) -> anyhow::Result<()> {
        self.emit_initial_model().await;
        let result = self.run_commands().await;
        self.emit(crate::entities::agent_event::event::AgentEvent::ShutdownComplete);
        self.emit(
            crate::entities::agent_event::event::AgentEvent::ActivityChanged(
                crate::entities::agent_event::status::AgentActivity::Idle,
            ),
        );
        result
    }

    async fn emit_initial_model(&self) {
        if let Some(model) = self.selected_model.read().await.clone() {
            self.emit(crate::entities::agent_event::event::AgentEvent::ModelChanged(model));
        }
    }

    /// Emits one UI event when the application boundary is configured.
    pub(super) fn emit(&self, event: crate::entities::agent_event::event::AgentEvent) {
        if let Some(events) = self.events.as_ref() {
            events.emit(event);
        }
    }

    /// Waits for the next UI command, or `None` when the UI closed.
    pub(super) async fn next_agent_command(
        &self,
    ) -> Option<crate::entities::agent_command::command::AgentCommand> {
        match self.command_source.as_ref() {
            Some(source) => source.next_command().await,
            None => None,
        }
    }
}

#[cfg(test)]
impl Chat {
    fn from_history<CM, P, GM, Fut>(
        configured_agent: crate::controllers::model_runtime::agent::dependencies::ConfiguredAgent<
            CM,
        >,
        terminal_io: Arc<TerminalIO>,
        history: Arc<ChatHistory<P>>,
        change_model: GM,
    ) -> Self
    where
        CM: rig::completion::CompletionModel + 'static,
        P: HistoryPersistence,
        GM: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<
                Output = anyhow::Result<
                    crate::controllers::model_runtime::agent::dependencies::ConfiguredAgent<CM>,
                >,
            > + Send
            + 'static,
    {
        let session =
            crate::controllers::model_runtime::session::chat::RigModelSession::from_configured(
                configured_agent,
                terminal_io.clone(),
                history.clone(),
            )
            .with_test_capabilities();
        let factory = TestModelSessionFactory {
            terminal_io: terminal_io.clone(),
            history: history.clone(),
            change_model,
        };
        let chat = Self::from_session(Box::new(session), history)
            .with_prompt_provider(Arc::new(StaticPromptProvider));
        futures::executor::block_on(chat.with_model_session_factory(Arc::new(factory)))
    }
}

#[cfg(test)]
struct TestModelSessionFactory<CM, P, GM, Fut>
where
    CM: rig::completion::CompletionModel,
    P: HistoryPersistence,
    GM: Fn() -> Fut,
    Fut: Future<
        Output = anyhow::Result<
            crate::controllers::model_runtime::agent::dependencies::ConfiguredAgent<CM>,
        >,
    >,
{
    terminal_io: Arc<TerminalIO>,
    history: Arc<ChatHistory<P>>,
    change_model: GM,
}

#[cfg(test)]
impl<CM, P, GM, Fut> ModelSessionFactory for TestModelSessionFactory<CM, P, GM, Fut>
where
    CM: rig::completion::CompletionModel + 'static,
    P: HistoryPersistence,
    GM: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<
            Output = anyhow::Result<
                crate::controllers::model_runtime::agent::dependencies::ConfiguredAgent<CM>,
            >,
        > + Send
        + 'static,
{
    fn create(
        &self,
        _kind: crate::entities::model::model_session_kind::ModelSessionKind,
        _selected_model: Option<crate::entities::selected_model::selection::SelectedModel>,
    ) -> futures::future::BoxFuture<'_, anyhow::Result<Box<dyn ModelSession>>> {
        Box::pin(async move {
            let configured = (self.change_model)().await?;
            let session =
                crate::controllers::model_runtime::session::chat::RigModelSession::from_configured(
                    configured,
                    self.terminal_io.clone(),
                    self.history.clone(),
                )
                .with_test_capabilities();
            Ok(Box::new(session) as Box<dyn ModelSession>)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
        controllers::model_runtime::agent::dependencies::ConfiguredAgent,
        controllers::model_runtime::goal_completion_hook::GoalCompletionHook,
        controllers::model_runtime::history::{
            chat_history::ChatHistory, history_persistence::NonPersistentHistory,
            message_codec::from_provider,
        },
        controllers::model_runtime::history_sync::HistorySyncHook,
        controllers::terminal::terminal_io::TerminalIO,
        entities::conversation::context_usage::ContextUsage,
        entities::conversation::conversation_message::ConversationMessage,
        entities::conversation::turn::TurnStatus,
        entities::goal::goal_state::{GoalReport, GoalState, GoalStateError},
        infrastructure::conversation::ConversationPersistence,
        infrastructure::tools::goal::GoalCompleter,
        infrastructure::ui::gateway::AgentCommandSource,
        prompts::summarization::summarization,
        tools::tool_mark_goal_complete::mark_goal_complete::MarkGoalComplete,
        use_cases::chat::history::conversation_history::ConversationHistory as DomainConversationHistory,
    };

    struct PendingCommands;

    impl AgentCommandSource for PendingCommands {
        fn next_command(
            &self,
        ) -> futures::future::BoxFuture<
            '_,
            Option<crate::entities::agent_command::command::AgentCommand>,
        > {
            Box::pin(std::future::pending())
        }
    }

    struct TestGoalCompleter {
        state: Arc<GoalState>,
    }

    impl GoalCompleter for TestGoalCompleter {
        fn complete(
            &self,
            report: String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<GoalReport, GoalStateError>> + Send + '_>,
        > {
            Box::pin(async move { self.state.complete(report) })
        }

        fn has_pending_report(&self) -> bool {
            self.state.has_pending_report()
        }
    }

    #[derive(Clone, Copy)]
    struct NoopPersistence;

    impl ConversationPersistence for NoopPersistence {
        fn save(
            &self,
            _messages: &[ConversationMessage],
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }
    }

    struct ResetPersistence {
        resets: Arc<AtomicUsize>,
    }

    impl ConversationPersistence for ResetPersistence {
        fn save(
            &self,
            _messages: &[ConversationMessage],
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }

        fn reset(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + '_>>
        {
            self.resets.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn new_conversation_resets_every_durable_segment() -> Result<()> {
        let resets = Arc::new(AtomicUsize::new(0));
        let domain = Arc::new(DomainConversationHistory::new(
            vec![ConversationMessage::User("old".to_string())],
            ResetPersistence {
                resets: resets.clone(),
            },
        ));
        let history = Arc::new(ChatHistory::<NonPersistentHistory>::new(domain.clone()));
        let chat = Chat::from_history(
            configured_agent(AgentBuilder::new(MockCompletionModel::text("unused")).build()),
            Arc::new(TerminalIO),
            history,
            || async {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );

        chat.handle_new().await?;

        ensure!(resets.load(Ordering::SeqCst) == 1);
        ensure!(domain.snapshot_messages().await.is_empty());
        Ok(())
    }

    fn test_history(messages: Vec<Message>) -> Arc<ChatHistory<NonPersistentHistory>> {
        let domain_messages = from_provider(&messages).unwrap_or_default();
        let domain = Arc::new(DomainConversationHistory::new(
            domain_messages,
            NoopPersistence,
        ));
        Arc::new(ChatHistory::new(domain))
    }

    #[tokio::test]
    async fn compact_context_replaces_history_with_model_summary() -> Result<()> {
        let agent = AgentBuilder::new(MockCompletionModel::text("compressed summary")).build();
        let history = test_history(vec![Message::user("first turn")]);
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            || async {
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
        let history = test_history(source.clone());
        let agent = AgentBuilder::new(model.clone()).build();
        let chat = Chat::from_history(
            configured_agent(agent),
            Arc::new(TerminalIO),
            history,
            || async {
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
        let history = test_history(vec![Message::user("keep me")]);
        let agent = AgentBuilder::new(model)
            .add_hook(HistorySyncHook::new(history.clone()))
            .build();
        let chat = Chat::from_history(
            configured_agent(agent),
            Arc::new(TerminalIO),
            history.clone(),
            || async {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        );

        let error = chat.compact_context().await.err();
        ensure!(
            error.is_some_and(|error| error.to_string().contains("boom")),
            "compaction failure was not propagated"
        );
        ensure!(
            history.snapshot().await == vec![Message::user("keep me")],
            "failed compaction changed history"
        );
        Ok(())
    }

    #[tokio::test]
    async fn automatic_compaction_replaces_history_and_resets_used_tokens() -> Result<()> {
        let agent = AgentBuilder::new(MockCompletionModel::text("automatic summary")).build();
        let history = test_history(vec![Message::user("long conversation")]);
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            || async {
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
        let history = test_history(Vec::new());
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            || async {
                Ok(configured_agent(
                    AgentBuilder::new(dual_text_model("replacement")).build(),
                ))
            },
        );

        chat.run_prompt("question".to_string()).await?;

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
        let history = test_history(vec![Message::user("keep this")]);
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            || async {
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
        let history = test_history(vec![Message::user("before model change")]);
        let chat = Chat::from_history(
            configured_agent_with_context(initial, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            move || {
                let changed = changed_by_callback.clone();
                let replacement = replacement_by_callback.clone();
                async move {
                    changed.store(true, Ordering::SeqCst);
                    Ok(configured_agent_with_context(replacement, Some(2_000)))
                }
            },
        );

        chat.handle_change_model(None).await?;
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
        let history = test_history(vec![Message::user("before failed model change")]);
        let chat = Chat::from_history(
            configured_agent(initial),
            Arc::new(TerminalIO),
            history.clone(),
            || async { anyhow::bail!("model selection failed") },
        );

        ensure!(chat.handle_change_model(None).await.is_err());
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
        let history = test_history(vec![Message::user("old")]);
        let chat = Chat::from_history(
            configured_agent_with_context(agent, Some(1_000)),
            Arc::new(TerminalIO),
            history.clone(),
            || async {
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
        let goal_completer = Arc::new(TestGoalCompleter {
            state: state.clone(),
        });
        let history = test_history(Vec::new());
        let agent = AgentBuilder::new(model.clone())
            .tool(MarkGoalComplete::new(goal_completer.clone()))
            .add_hook(GoalCompletionHook::new(goal_completer))
            .add_hook(HistorySyncHook::new(history.clone()))
            .default_max_turns(12)
            .build();
        let chat = Chat::from_history(
            configured_agent(agent),
            Arc::new(TerminalIO),
            history,
            || async {
                Ok(configured_agent(
                    AgentBuilder::new(MockCompletionModel::text("replacement")).build(),
                ))
            },
        )
        .with_goal_state(state.clone())
        .with_command_source(Arc::new(PendingCommands));

        chat.run_goal("goal A".to_string()).await?;
        assert!(!state.is_active());
        assert!(!state.has_pending_report());
        assert_eq!(model.request_count(), 3);
        assert!(request_contains_user_text(
            &model.requests()[1],
            "Original goal:\ngoal A"
        ));
        assert_eq!(model.requests()[2].tool_choice, Some(ToolChoice::None));

        chat.run_goal("goal B".to_string()).await?;
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
            summarizer: None,
            selected_model: None,
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
