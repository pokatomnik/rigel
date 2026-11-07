use futures::StreamExt;
use std::sync::Arc;

use rig::{
    Agent,
    agent::{MultiTurnStreamItem, PromptResponse, StreamingError},
    completion::CompletionModel,
    message::Message,
    streaming::StreamingChat,
    tool::ToolContext,
};
use tokio::sync::Mutex;

use crate::{
    controllers::model_runtime::history_sync::HISTORY_SYNC_ERROR_PREFIX,
    controllers::model_runtime::{
        history::{
            chat_history::{ChatHistory, HistoryUpdate},
            history_persistence::HistoryPersistence,
        },
        recovery::{
            recovery_error::recovery_context_from_streaming_error,
            tool_recovery::MAX_INVALID_TOOL_CALL_ATTEMPTS,
        },
        streaming::{progress::StreamProgress, stream_output_state::StreamOutputState},
    },
    controllers::terminal::terminal_io::TerminalIO,
    entities::{
        conversation::context_usage::ContextUsage, selected_model::selection::SelectedModel,
    },
    shared::terminal::loader::SpinnerGuard,
};

pub(crate) struct StreamCompletion {
    output: String,
    output_state: StreamOutputState,
    context_usage: ContextUsage,
}

impl StreamCompletion {
    pub(crate) fn output(&self) -> &str {
        self.output.as_str()
    }

    pub(crate) fn has_output(&self) -> bool {
        !self.output.trim().is_empty()
    }

    pub(crate) fn requires_tool_recovery(&self) -> bool {
        self.output_state.requires_tool_recovery()
    }

    pub(crate) fn requires_answer_recovery(&self) -> bool {
        self.output_state.requires_answer_recovery()
    }

    pub(crate) fn context_usage(&self) -> ContextUsage {
        self.context_usage
    }
}

pub(crate) enum StreamRunOutcome {
    Completed(StreamCompletion),
    Failed(StreamFailure),
}

pub(crate) struct StreamFailure {
    error: String,
    context_usage: ContextUsage,
}

impl StreamFailure {
    pub(crate) fn error(&self) -> &str {
        self.error.as_str()
    }

    pub(crate) fn context_usage(&self) -> ContextUsage {
        self.context_usage
    }
}

pub(crate) struct StreamedTurnContext<'a, P>
where
    P: HistoryPersistence,
{
    history: &'a ChatHistory<P>,
    context_usage: ContextUsage,
    selected_model: Option<SelectedModel>,
}

impl<'a, P> StreamedTurnContext<'a, P>
where
    P: HistoryPersistence,
{
    pub(crate) fn new(history: &'a ChatHistory<P>, context_usage: ContextUsage) -> Self {
        Self {
            history,
            context_usage,
            selected_model: None,
        }
    }

    /// Adds the selected model metadata for tools that create delegated sessions.
    pub(crate) fn with_selected_model(mut self, model: SelectedModel) -> Self {
        self.selected_model = Some(model);
        self
    }
}

/// Executes one streamed model turn and commits its durable boundaries.
pub(crate) struct StreamedTurn<'a, CM, P>
where
    CM: CompletionModel,
    P: HistoryPersistence,
{
    agent: Arc<Mutex<Agent<CM>>>,
    terminal_io: &'a TerminalIO,
    history: &'a ChatHistory<P>,
    max_context_tokens: Option<u64>,
    context_usage: ContextUsage,
    selected_model: Option<SelectedModel>,
}

impl<'a, CM, P> StreamedTurn<'a, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    #[cfg(test)]
    pub(crate) fn new(
        agent: Arc<Mutex<Agent<CM>>>,
        terminal_io: &'a TerminalIO,
        history: &'a ChatHistory<P>,
    ) -> Self {
        Self::with_context(
            agent,
            terminal_io,
            StreamedTurnContext::new(history, ContextUsage::new(None, None)),
        )
    }

    pub(crate) fn with_context(
        agent: Arc<Mutex<Agent<CM>>>,
        terminal_io: &'a TerminalIO,
        context: StreamedTurnContext<'a, P>,
    ) -> Self {
        Self {
            agent,
            terminal_io,
            history: context.history,
            max_context_tokens: context.context_usage.max_context_tokens(),
            context_usage: context.context_usage,
            selected_model: context.selected_model,
        }
    }

    fn tool_context(&self) -> ToolContext {
        let mut context = ToolContext::new();
        context.insert(self.context_usage);
        if let Some(model) = self.selected_model.as_ref() {
            context.insert(model.clone());
        }
        context
    }

    pub(crate) async fn run_latest(&self, prompt: String) -> anyhow::Result<StreamRunOutcome> {
        let base = self.history.snapshot_result().await?;
        self.run(prompt, base).await
    }

    pub(crate) async fn run(
        &self,
        prompt: String,
        base: Vec<Message>,
    ) -> anyhow::Result<StreamRunOutcome> {
        self.history
            .update(HistoryUpdate::Append(Message::user(prompt.clone())))
            .await?;
        let mut progress = StreamProgress::new(base.clone(), prompt.clone());
        let mut loader = SpinnerGuard::start();
        let request = {
            let agent = self.agent.lock().await;
            agent
                .stream_chat(prompt, base.clone())
                .tool_context(self.tool_context())
                .max_invalid_tool_call_retries(MAX_INVALID_TOOL_CALL_ATTEMPTS)
        };
        let mut stream = request.await;

        while let Some(item) = stream.next().await {
            if matches!(
                &item,
                Ok(MultiTurnStreamItem::StreamAssistantItem(_))
                    | Ok(MultiTurnStreamItem::StreamUserItem(_))
            ) {
                loader.hide();
            }
            if !progress
                .accept(item, self.terminal_io, self.history)
                .await?
            {
                break;
            }
        }

        self.finish(progress, base).await
    }

    async fn finish(
        &self,
        mut progress: StreamProgress,
        base: Vec<Message>,
    ) -> anyhow::Result<StreamRunOutcome> {
        if let Some(error) = progress.take_stream_error() {
            return self
                .finish_error(error, progress.context_usage(self.max_context_tokens))
                .await;
        }
        let Some(response) = progress.take_final_response() else {
            return Ok(StreamRunOutcome::Failed(StreamFailure {
                error: "model stream ended without a final response".to_string(),
                context_usage: progress.context_usage(self.max_context_tokens),
            }));
        };

        self.finish_response(response, progress, base).await
    }

    async fn finish_error(
        &self,
        error: StreamingError,
        context_usage: ContextUsage,
    ) -> anyhow::Result<StreamRunOutcome> {
        let context = recovery_context_from_streaming_error(error);
        if context.message.contains(HISTORY_SYNC_ERROR_PREFIX) {
            anyhow::bail!("{}", context.message);
        }
        if let Some(messages) = context.chat_history {
            self.history
                .update(HistoryUpdate::Replace(messages))
                .await?;
        }
        Ok(StreamRunOutcome::Failed(StreamFailure {
            error: context.message,
            context_usage,
        }))
    }

    async fn finish_response(
        &self,
        response: PromptResponse,
        progress: StreamProgress,
        mut base: Vec<Message>,
    ) -> anyhow::Result<StreamRunOutcome> {
        let (output_state, context_usage) = progress.into_completion_state(self.max_context_tokens);
        let output = response.output().to_string();
        let Some(streamed) = response.messages else {
            return Ok(StreamRunOutcome::Failed(StreamFailure {
                error: "model final response omitted canonical chat history".to_string(),
                context_usage,
            }));
        };
        if !streamed.is_empty() {
            base.extend(streamed);
            self.history.update(HistoryUpdate::Replace(base)).await?;
        }

        Ok(StreamRunOutcome::Completed(StreamCompletion {
            output,
            output_state,
            context_usage,
        }))
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
        AgentBuilder,
        agent::{AgentHook, HookContext, ModelTurnAction, ModelTurnFinished},
        completion::Usage,
        message::{AssistantContent, Message, UserContent},
        test_utils::{MockAddTool, MockCompletionModel, MockStreamEvent},
        tool::{Tool, ToolContext, ToolExecutionError},
    };
    use tokio::sync::Mutex;

    use super::{StreamRunOutcome, StreamedTurn, StreamedTurnContext};
    use crate::{
        controllers::model_runtime::history_sync::HistorySyncHook,
        controllers::model_runtime::{
            history::{chat_history::ChatHistory, history_persistence::HistoryPersistence},
            recovery::tool_recovery::ToolRecoveryHook,
            response::invalid_response::InvalidResponseHook,
        },
        controllers::terminal::terminal_io::TerminalIO,
        entities::{
            conversation::context_usage::ContextUsage, selected_model::selection::SelectedModel,
        },
        use_cases::chat::recovery::policy::DefaultToolRecoveryPolicyFactory,
    };

    #[derive(Clone)]
    struct RecordingPersistence {
        snapshots: Arc<Mutex<Vec<Vec<Message>>>>,
    }

    impl HistoryPersistence for RecordingPersistence {
        fn save<'a>(
            &'a self,
            messages: &'a [Message],
        ) -> futures::future::BoxFuture<'a, anyhow::Result<()>> {
            Box::pin(async move {
                self.snapshots.lock().await.push(messages.to_vec());
                Ok(())
            })
        }
    }

    #[derive(Clone)]
    struct ContextProbe {
        seen: Arc<Mutex<Option<ContextUsage>>>,
        model_seen: Arc<Mutex<Option<SelectedModel>>>,
    }

    impl Tool for ContextProbe {
        const NAME: &'static str = "context_probe";
        type Args = serde_json::Value;
        type Output = String;
        type Error = ToolExecutionError;

        fn description(&self) -> String {
            "Records the context usage supplied to tool execution".to_string()
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn call(
            &self,
            context: &mut ToolContext,
            _args: Self::Args,
        ) -> Result<Self::Output, Self::Error> {
            *self.seen.lock().await = context.get::<ContextUsage>().copied();
            *self.model_seen.lock().await = context.get::<SelectedModel>().cloned();
            Ok("observed".to_string())
        }
    }

    #[derive(Clone)]
    struct RetryOnceHook {
        retried: Arc<AtomicBool>,
    }

    impl AgentHook for RetryOnceHook {
        async fn on_model_turn_finished(
            &self,
            _ctx: &HookContext,
            _event: ModelTurnFinished<'_>,
        ) -> ModelTurnAction {
            if self.retried.swap(true, Ordering::SeqCst) {
                ModelTurnAction::continue_run()
            } else {
                ModelTurnAction::retry_with_feedback("correct the response")
            }
        }
    }

    #[tokio::test]
    async fn completed_response_is_persisted_before_stream_finishes() -> Result<()> {
        let model = MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::text("answer"),
            MockStreamEvent::final_response_with_total_tokens(1),
        ]]);
        let snapshots = recorded_snapshots();
        let history = recording_history(snapshots.clone())?;
        let agent = AgentBuilder::new(model)
            .add_hook(HistorySyncHook::new(history.clone()))
            .build();
        let turn = StreamedTurn::new(Arc::new(Mutex::new(agent)), &TerminalIO, &history);

        let outcome = turn.run("prompt".to_string(), Vec::new()).await?;
        ensure!(matches!(outcome, StreamRunOutcome::Completed(_)));
        ensure!(snapshots.lock().await.len() == 2);
        Ok(())
    }

    #[tokio::test]
    async fn streamed_turn_provides_current_context_usage_to_tools() -> Result<()> {
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::tool_call("call-1", "context_probe", serde_json::json!({})),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::text("done"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
        ]);
        let seen = Arc::new(Mutex::new(None));
        let model_seen = Arc::new(Mutex::new(None));
        let selected_model = SelectedModel::new("parent-model".to_string(), Some(1_000));
        let usage = ContextUsage::new(Some(800), Some(1_000));
        let agent = AgentBuilder::new(model)
            .tool(ContextProbe {
                seen: seen.clone(),
                model_seen: model_seen.clone(),
            })
            .default_max_turns(3)
            .build();
        let history = ChatHistory::from_provider(
            Vec::new(),
            crate::controllers::model_runtime::history::history_persistence::NonPersistentHistory,
        )?;
        let turn = StreamedTurn::with_context(
            Arc::new(Mutex::new(agent)),
            &TerminalIO,
            StreamedTurnContext::new(&history, usage).with_selected_model(selected_model.clone()),
        );

        let outcome = turn.run("prompt".to_string(), Vec::new()).await?;

        ensure!(matches!(outcome, StreamRunOutcome::Completed(_)));
        ensure!(*seen.lock().await == Some(usage));
        ensure!(*model_seen.lock().await == Some(selected_model));
        Ok(())
    }

    #[tokio::test]
    async fn completion_uses_the_last_provider_call_usage() -> Result<()> {
        let mut first_usage = Usage::new();
        first_usage.total_tokens = 300;
        let mut last_usage = Usage::new();
        last_usage.total_tokens = 800;
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::tool_call("call-1", "add", serde_json::json!({"x": 1, "y": 2})),
                MockStreamEvent::final_response(first_usage),
            ],
            vec![
                MockStreamEvent::text("answer"),
                MockStreamEvent::final_response(last_usage),
            ],
        ]);
        let history = ChatHistory::from_provider(
            Vec::new(),
            crate::controllers::model_runtime::history::history_persistence::NonPersistentHistory,
        )?;
        let agent = AgentBuilder::new(model)
            .tool(MockAddTool)
            .default_max_turns(3)
            .build();
        let turn = StreamedTurn::with_context(
            Arc::new(Mutex::new(agent)),
            &TerminalIO,
            StreamedTurnContext::new(&history, ContextUsage::new(None, Some(1_000))),
        );

        let outcome = turn.run("prompt".to_string(), Vec::new()).await?;
        let StreamRunOutcome::Completed(completion) = outcome else {
            let StreamRunOutcome::Failed(failure) = outcome else {
                anyhow::bail!("expected a completed stream")
            };
            anyhow::bail!("expected a completed stream: {}", failure.error())
        };
        ensure!(
            completion.context_usage() == ContextUsage::new(Some(800), Some(1_000)),
            "aggregated or first-call usage was used"
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_response_is_retried_without_entering_history() -> Result<()> {
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::text("bad <|tool_call|> response"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![
                MockStreamEvent::text("accepted"),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
        ]);
        let probe = model.clone();
        let snapshots = recorded_snapshots();
        let history = recording_history(snapshots.clone())?;
        let agent = AgentBuilder::new(model)
            .add_hook(InvalidResponseHook::new(Arc::new(TerminalIO)))
            .add_hook(HistorySyncHook::new(history.clone()))
            .default_max_turns(3)
            .build();

        let turn = StreamedTurn::new(Arc::new(Mutex::new(agent)), &TerminalIO, &history);
        ensure!(matches!(
            turn.run("prompt".to_string(), Vec::new()).await?,
            StreamRunOutcome::Completed(_)
        ));
        ensure!(probe.request_count() == 2);
        ensure!(
            !snapshots
                .lock()
                .await
                .iter()
                .any(|messages| has_assistant_text(messages, "bad <|tool_call|> response"))
        );
        ensure!(has_assistant_text(&history.snapshot().await, "accepted"));
        Ok(())
    }

    #[tokio::test]
    async fn internal_feedback_retry_is_persisted_before_next_model_call() -> Result<()> {
        let model = MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::text_start(Some(serde_json::json!({ "rejected": true }))),
                MockStreamEvent::text(""),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![MockStreamEvent::error("retry transport failed")],
        ]);
        let snapshots = Arc::new(Mutex::new(Vec::<Vec<Message>>::new()));
        let persistence = RecordingPersistence {
            snapshots: snapshots.clone(),
        };
        let history = Arc::new(ChatHistory::from_provider(Vec::new(), persistence)?);
        let agent = AgentBuilder::new(model)
            .add_hook(HistorySyncHook::new(history.clone()))
            .add_hook(RetryOnceHook {
                retried: Arc::new(AtomicBool::new(false)),
            })
            .default_max_turns(3)
            .build();

        assert_failed(
            StreamedTurn::new(Arc::new(Mutex::new(agent)), &TerminalIO, &history),
            &history,
        )
        .await?;
        let stored = history.snapshot().await;
        ensure!(has_rejected_assistant_metadata(&stored));
        ensure!(has_user_text(&stored, "correct the response"));
        ensure!(snapshots.lock().await.len() == 3);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_tool_retry_is_persisted_before_next_model_call() -> Result<()> {
        let snapshots = recorded_snapshots();
        let history = recording_history(snapshots.clone())?;
        let agent = AgentBuilder::new(invalid_tool_model())
            .add_hook(HistorySyncHook::new(history.clone()))
            .add_hook(ToolRecoveryHook::new(Arc::new(
                DefaultToolRecoveryPolicyFactory::new(),
            )))
            .tool(MockAddTool)
            .default_max_turns(3)
            .build();

        assert_failed(
            StreamedTurn::new(Arc::new(Mutex::new(agent)), &TerminalIO, &history),
            &history,
        )
        .await?;
        let stored = history.snapshot().await;
        ensure!(has_tool_call(&stored, "default_api"));
        ensure!(has_tool_result(&stored, "invalid-call"));
        let observed = snapshots.lock().await;
        ensure!(observed.len() == 3);
        ensure!(tool_boundaries_are_separate(observed.as_slice()));
        Ok(())
    }

    fn has_assistant_text(messages: &[Message], expected: &str) -> bool {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::Assistant { content, .. }
                    if content.iter().any(|item| matches!(
                        item,
                        AssistantContent::Text(text) if text.text == expected
                    ))
            )
        })
    }

    fn invalid_tool_model() -> MockCompletionModel {
        MockCompletionModel::from_stream_turns([
            vec![
                MockStreamEvent::tool_call(
                    "invalid-call",
                    "default_api",
                    serde_json::json!({ "x": 1, "y": 2 }),
                ),
                MockStreamEvent::final_response_with_total_tokens(1),
            ],
            vec![MockStreamEvent::error("retry transport failed")],
        ])
    }

    fn recorded_snapshots() -> Arc<Mutex<Vec<Vec<Message>>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn recording_history(
        snapshots: Arc<Mutex<Vec<Vec<Message>>>>,
    ) -> Result<Arc<ChatHistory<RecordingPersistence>>> {
        let persistence = RecordingPersistence { snapshots };
        Ok(Arc::new(ChatHistory::from_provider(
            Vec::new(),
            persistence,
        )?))
    }

    fn tool_boundaries_are_separate(snapshots: &[Vec<Message>]) -> bool {
        let (Some(call_snapshot), Some(result_snapshot)) = (snapshots.get(1), snapshots.get(2))
        else {
            return false;
        };
        has_tool_call(call_snapshot, "default_api")
            && !has_tool_result(call_snapshot, "invalid-call")
            && has_tool_result(result_snapshot, "invalid-call")
    }

    async fn assert_failed<CM, P>(
        turn: StreamedTurn<'_, CM, P>,
        history: &ChatHistory<P>,
    ) -> Result<()>
    where
        CM: rig::completion::CompletionModel + 'static,
        P: HistoryPersistence,
    {
        let outcome = turn.run("prompt".to_string(), Vec::new()).await?;
        ensure!(matches!(outcome, StreamRunOutcome::Failed(_)));
        ensure!(!history.snapshot().await.is_empty());
        Ok(())
    }

    fn has_rejected_assistant_metadata(messages: &[Message]) -> bool {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::Assistant { content, .. }
                    if content.iter().any(|item| matches!(
                        item,
                        AssistantContent::Text(text)
                            if text.additional_params.as_ref().is_some_and(|params| {
                                params.get("rejected").and_then(|value| value.as_bool()) == Some(true)
                            })
                    ))
            )
        })
    }

    fn has_user_text(messages: &[Message], expected: &str) -> bool {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::User { content }
                    if content.iter().any(|item| matches!(
                        item,
                        UserContent::Text(text) if text.text() == expected
                    ))
            )
        })
    }

    fn has_tool_call(messages: &[Message], expected_name: &str) -> bool {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::Assistant { content, .. }
                    if content.iter().any(|item| matches!(
                        item,
                        AssistantContent::ToolCall(call)
                            if call.function.name == expected_name
                    ))
            )
        })
    }

    fn has_tool_result(messages: &[Message], expected_id: &str) -> bool {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::User { content }
                    if content.iter().any(|item| matches!(
                        item,
                        UserContent::ToolResult(result) if result.id == expected_id
                    ))
            )
        })
    }
}
