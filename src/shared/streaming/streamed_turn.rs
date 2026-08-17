use futures::StreamExt;
use std::sync::Arc;

use rig::{
    Agent, OneOrMany,
    agent::{MultiTurnStreamItem, PromptResponse, StreamingError},
    completion::CompletionModel,
    message::{Message, Reasoning, Text, ToolCall, ToolResult, ToolResultContent, UserContent},
    streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat},
};
use tokio::sync::Mutex;

use crate::shared::{
    history::{ChatHistory, HISTORY_SYNC_ERROR_PREFIX, HistoryPersistence, HistoryUpdate},
    recovery::recovery_context_from_streaming_error,
    recovery::{MAX_INVALID_TOOL_CALL_ATTEMPTS, tool_recovery_status},
    streaming::StreamOutputState,
    streaming::{ToolResultRecord, TurnJournal},
    string::string_ext::StringShort,
    terminal::TerminalIO,
};

pub(crate) struct StreamCompletion {
    output: String,
    output_state: StreamOutputState,
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
}

pub(crate) enum StreamRunOutcome {
    Completed(StreamCompletion),
    Failed(String),
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
}

impl<'a, CM, P> StreamedTurn<'a, CM, P>
where
    CM: CompletionModel + 'static,
    P: HistoryPersistence,
{
    pub(crate) fn new(
        agent: Arc<Mutex<Agent<CM>>>,
        terminal_io: &'a TerminalIO,
        history: &'a ChatHistory<P>,
    ) -> Self {
        Self {
            agent,
            terminal_io,
            history,
        }
    }

    pub(crate) async fn run_latest(&self, prompt: String) -> anyhow::Result<StreamRunOutcome> {
        let base = self.history.snapshot().await;
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
        let request = {
            let agent = self.agent.lock().await;
            agent
                .stream_chat(prompt, base.clone())
                .max_invalid_tool_call_retries(MAX_INVALID_TOOL_CALL_ATTEMPTS)
        };
        let mut stream = request.await;

        while let Some(item) = stream.next().await {
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
        progress: StreamProgress,
        base: Vec<Message>,
    ) -> anyhow::Result<StreamRunOutcome> {
        if let Some(error) = progress.stream_error {
            return self.finish_error(error).await;
        }
        let Some(response) = progress.final_response else {
            return Ok(StreamRunOutcome::Failed(
                "model stream ended without a final response".to_string(),
            ));
        };

        self.finish_response(response, progress.output_state, base)
            .await
    }

    async fn finish_error(&self, error: StreamingError) -> anyhow::Result<StreamRunOutcome> {
        let context = recovery_context_from_streaming_error(error);
        if context.message.contains(HISTORY_SYNC_ERROR_PREFIX) {
            anyhow::bail!("{}", context.message);
        }
        if let Some(messages) = context.chat_history {
            self.history
                .update(HistoryUpdate::Replace(messages))
                .await?;
        }
        Ok(StreamRunOutcome::Failed(context.message))
    }

    async fn finish_response(
        &self,
        response: PromptResponse,
        output_state: StreamOutputState,
        mut base: Vec<Message>,
    ) -> anyhow::Result<StreamRunOutcome> {
        let output = response.output().to_string();
        let Some(streamed) = response.messages else {
            return Ok(StreamRunOutcome::Failed(
                "model final response omitted canonical chat history".to_string(),
            ));
        };
        if !streamed.is_empty() {
            base.extend(streamed);
            self.history.update(HistoryUpdate::Replace(base)).await?;
        }

        Ok(StreamRunOutcome::Completed(StreamCompletion {
            output,
            output_state,
        }))
    }
}

/// Accumulates canonical state for an in-progress streamed turn.
struct StreamProgress {
    output_state: StreamOutputState,
    final_response: Option<PromptResponse>,
    stream_error: Option<StreamingError>,
    journal: TurnJournal,
}

impl StreamProgress {
    fn new(base: Vec<Message>, prompt: String) -> Self {
        Self {
            output_state: StreamOutputState::default(),
            final_response: None,
            stream_error: None,
            journal: TurnJournal::new(base, prompt),
        }
    }

    async fn accept<R, P>(
        &mut self,
        item: Result<MultiTurnStreamItem<R>, StreamingError>,
        terminal_io: &TerminalIO,
        history: &ChatHistory<P>,
    ) -> anyhow::Result<bool>
    where
        P: HistoryPersistence,
    {
        let item = match item {
            Ok(item) => item,
            Err(error) => {
                self.stream_error = Some(error);
                return Ok(false);
            }
        };
        self.handle_item(item, terminal_io, history).await?;
        Ok(true)
    }

    async fn handle_item<R, P>(
        &mut self,
        item: MultiTurnStreamItem<R>,
        terminal_io: &TerminalIO,
        history: &ChatHistory<P>,
    ) -> anyhow::Result<()>
    where
        P: HistoryPersistence,
    {
        match item {
            MultiTurnStreamItem::StreamAssistantItem(content) => {
                self.handle_assistant(content, terminal_io, history).await;
            }
            MultiTurnStreamItem::StreamUserItem(content) => {
                self.handle_user(content, terminal_io, history).await?;
            }
            MultiTurnStreamItem::CompletionCall(_) => {
                self.journal.rebase(history.snapshot().await);
            }
            MultiTurnStreamItem::ModelTurnRetried { .. } => self.output_state.retry_answer(),
            MultiTurnStreamItem::FinalResponse(response) => self.final_response = Some(response),
            _ => {}
        }
        Ok(())
    }

    async fn handle_assistant<R, P>(
        &mut self,
        content: StreamedAssistantContent<R>,
        terminal_io: &TerminalIO,
        history: &ChatHistory<P>,
    ) where
        P: HistoryPersistence,
    {
        match content {
            StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                print_reasoning(terminal_io, &mut self.output_state, reasoning.as_str());
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                print_reasoning_block(terminal_io, &mut self.output_state, &reasoning);
            }
            StreamedAssistantContent::Text(text) => {
                print_text(terminal_io, &mut self.output_state, &text);
            }
            StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            } => {
                print_tool_call(terminal_io, &tool_call);
                self.journal
                    .record_tool_call(history.snapshot().await, internal_call_id);
            }
            _ => {}
        }
    }

    async fn handle_user<P>(
        &mut self,
        content: StreamedUserContent,
        terminal_io: &TerminalIO,
        history: &ChatHistory<P>,
    ) -> anyhow::Result<()>
    where
        P: HistoryPersistence,
    {
        let StreamedUserContent::ToolResult {
            tool_result,
            internal_call_id,
        } = content;
        print_tool_result(terminal_io, &mut self.output_state, &tool_result);
        let record = self
            .journal
            .record_tool_result(tool_result, internal_call_id.as_str());
        self.persist_tool_result(record, history).await
    }

    async fn persist_tool_result<P>(
        &self,
        record: ToolResultRecord,
        history: &ChatHistory<P>,
    ) -> anyhow::Result<()>
    where
        P: HistoryPersistence,
    {
        let update = match record {
            ToolResultRecord::Updated(messages) => HistoryUpdate::Replace(messages),
            ToolResultRecord::Unmatched(result) => HistoryUpdate::Append(Message::User {
                content: OneOrMany::one(UserContent::ToolResult(result)),
            }),
            ToolResultRecord::Ignored => return Ok(()),
        };
        history.update(update).await
    }
}

fn print_reasoning(terminal_io: &TerminalIO, state: &mut StreamOutputState, reasoning: &str) {
    if reasoning.is_empty() {
        return;
    }
    if !reasoning.trim().is_empty() {
        state.set_received_reasoning(true);
        if !state.showing_reasoning() {
            terminal_io.eprintln_gray("[thinking]");
            state.set_showing_reasoning(true);
        }
    }
    terminal_io.eprint_gray(reasoning);
    terminal_io.flush_stderr();
}

fn print_reasoning_block(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    reasoning: &Reasoning,
) {
    print_reasoning(terminal_io, state, reasoning.display_text().as_str());
}

fn print_text(terminal_io: &TerminalIO, state: &mut StreamOutputState, text: &Text) {
    if text.text().is_empty() {
        return;
    }
    if !text.text().trim().is_empty() {
        state.set_received_answer(true);
        if state.showing_reasoning() {
            terminal_io.eprintln("\n[answer]");
            state.set_showing_reasoning(false);
        }
    }
    terminal_io.print(text.text());
    terminal_io.flush_stdout();
}

fn print_tool_call(terminal_io: &TerminalIO, tool_call: &ToolCall) {
    let args_str = tool_call.function.arguments.to_string().short(30);
    terminal_io.eprintln_orange(
        format!("\n[tool call: {}({})]", tool_call.function.name, args_str).as_str(),
    );
}

fn print_tool_result(
    terminal_io: &TerminalIO,
    state: &mut StreamOutputState,
    tool_result: &ToolResult,
) {
    state.record_tool_result(tool_recovery_status(tool_result));
    let output = tool_result
        .content
        .iter()
        .map(format_tool_result_content)
        .collect::<Vec<_>>()
        .join("\n");
    terminal_io.eprintln_blue(format!("[tool result: {}]", output.short(30)).as_str());
}

fn format_tool_result_content(content: &ToolResultContent) -> String {
    match content {
        ToolResultContent::Text(text) => text.text.clone(),
        ToolResultContent::Json { value } => value.to_string(),
        ToolResultContent::Image(_) => "<image>".to_string(),
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
        message::{AssistantContent, Message, UserContent},
        test_utils::{MockAddTool, MockCompletionModel, MockStreamEvent},
    };
    use tokio::sync::Mutex;

    use super::{StreamRunOutcome, StreamedTurn};
    use crate::{
        shared::terminal::TerminalIO,
        shared::{
            history::{ChatHistory, HistoryPersistence, HistorySyncHook},
            recovery::ToolRecoveryHook,
            response::InvalidResponseHook,
        },
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
        let history = recording_history(snapshots.clone());
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
        let history = recording_history(snapshots.clone());
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
        let history = Arc::new(ChatHistory::new(Vec::new(), persistence));
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
        let history = recording_history(snapshots.clone());
        let agent = AgentBuilder::new(invalid_tool_model())
            .add_hook(HistorySyncHook::new(history.clone()))
            .add_hook(ToolRecoveryHook)
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
    ) -> Arc<ChatHistory<RecordingPersistence>> {
        let persistence = RecordingPersistence { snapshots };
        Arc::new(ChatHistory::new(Vec::new(), persistence))
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
