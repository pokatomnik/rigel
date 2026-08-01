use std::sync::Arc;

use futures::StreamExt;
use rig::{
    Agent,
    agent::{MultiTurnStreamItem, PromptResponse, StreamingError},
    completion::{Chat as RigChat, CompletionModel},
    message::{Message, Reasoning, Text, ToolCall, ToolResult, ToolResultContent},
    streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat},
};
use tokio::sync::Mutex;

use crate::{
    prompts::recovery::{
        missing_answer_recovery_prompt, recovery_prompt, unresolved_tool_recovery_prompt,
    },
    shared::terminal_io::TerminalIO,
    use_cases::chat::{
        command_parser::{self, CommandParser},
        recovery_error::{
            format_recovery_stopped_notice, recovery_context_from_prompt_error,
            recovery_context_from_streaming_error,
        },
        stream_output_state::StreamOutputState,
        tool_recovery::tool_recovery_status,
    },
};

const MAX_RECOVERY_ATTEMPTS: usize = 3;
const MAX_TOOL_RECOVERY_ATTEMPTS: usize = 2;

/// An exhausted recovery run together with the latest conversation state it produced.
struct RecoveryFailure {
    error: String,
    messages: Vec<Message>,
}

/// Owns the interactive conversation, streams model output, and coordinates automatic recovery.
pub(crate) struct Chat<CM, F>
where
    CM: CompletionModel,
    F: AsyncFn(&[&Message]) + 'static,
{
    agent: Agent<CM>,
    terminal_io: Arc<TerminalIO>,
    messages: Arc<Mutex<Vec<Message>>>,
    on_messages_change: F,
    command_parser: CommandParser,
}

impl<CM, F> Chat<CM, F>
where
    CM: CompletionModel + 'static,
    F: AsyncFn(&[&Message]) + 'static,
{
    /// Creates a chat session from an agent, terminal adapter, existing history, and change hook.
    ///
    /// The supplied history becomes the initial model context. `on_messages_change` is called
    /// whenever this session commits a new version of that history.
    pub fn new(
        agent: Agent<CM>,
        terminal_io: Arc<TerminalIO>,
        messages: Vec<Message>,
        on_messages_change: F,
    ) -> Self {
        let messages = Arc::new(Mutex::new(messages));
        let command_parser = CommandParser::new(terminal_io.clone());
        Self {
            agent,
            terminal_io: terminal_io.clone(),
            messages,
            on_messages_change,
            command_parser,
        }
    }

    /// Retries a model run that ended with a streaming or runtime error.
    ///
    /// Each retry includes the latest error in a corrective prompt. When Rig returns canonical
    /// history with a failed attempt, that history replaces the local recovery copy so later
    /// retries and eventual user guidance retain all attempted tool calls and results.
    ///
    /// Returns the first successful response and its accumulated history, or `RecoveryFailure`
    /// after [`MAX_RECOVERY_ATTEMPTS`] attempts.
    async fn recover_response(
        &self,
        mut messages: Vec<Message>,
        initial_error: String,
    ) -> Result<(String, Vec<Message>), RecoveryFailure> {
        let mut error = initial_error;

        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let recovery_prompt = recovery_prompt(error.as_str(), attempt, MAX_RECOVERY_ATTEMPTS);

            match self.agent.chat(recovery_prompt, &mut messages).await {
                Ok(response) => return Ok((response, messages)),
                Err(recovery_error) => {
                    let context = recovery_context_from_prompt_error(recovery_error);
                    error = context.message;
                    if let Some(failure_messages) = context.chat_history {
                        messages = failure_messages;
                    }
                }
            }
        }

        Err(RecoveryFailure {
            error: format!(
                "model failed to recover after {MAX_RECOVERY_ATTEMPTS} attempts: {error}"
            ),
            messages,
        })
    }

    /// Requests a non-empty final answer after a stream produced reasoning but no answer text.
    ///
    /// Empty successful responses are treated as incomplete and retried. Prompt failures update
    /// the working history when Rig provides a canonical failure history. Exhausting the retry
    /// budget returns the latest history so it can still be saved for a user follow-up.
    async fn recover_missing_answer(
        &self,
        mut messages: Vec<Message>,
    ) -> Result<(String, Vec<Message>), RecoveryFailure> {
        let mut last_error = None;

        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let recovery_prompt = missing_answer_recovery_prompt(attempt, MAX_RECOVERY_ATTEMPTS);

            match self.agent.chat(recovery_prompt, &mut messages).await {
                Ok(response) if !response.trim().is_empty() => return Ok((response, messages)),
                Ok(_) => last_error = None,
                Err(error) => {
                    let context = recovery_context_from_prompt_error(error);
                    last_error = Some(context.message);
                    if let Some(failure_messages) = context.chat_history {
                        messages = failure_messages;
                    }
                }
            }
        }

        let error = if let Some(error) = last_error {
            format!(
                "model failed to produce an answer after {MAX_RECOVERY_ATTEMPTS} recovery \
                 attempts: {error}"
            )
        } else {
            format!("model produced no answer after {MAX_RECOVERY_ATTEMPTS} recovery attempts")
        };

        Err(RecoveryFailure { error, messages })
    }

    /// Asks the model to resolve a tool failure that was left without a corrective action.
    ///
    /// A recovery succeeds only when the model returns a non-empty final answer; further tool
    /// calls are handled internally by the agent run. The smaller tool-recovery budget prevents
    /// an unresolved tool loop from indefinitely delaying control from returning to the user.
    async fn recover_unresolved_tool(
        &self,
        mut messages: Vec<Message>,
    ) -> Result<(String, Vec<Message>), RecoveryFailure> {
        let mut last_error = None;

        for attempt in 1..=MAX_TOOL_RECOVERY_ATTEMPTS {
            let recovery_prompt =
                unresolved_tool_recovery_prompt(attempt, MAX_TOOL_RECOVERY_ATTEMPTS);

            match self.agent.chat(recovery_prompt, &mut messages).await {
                Ok(response) if !response.trim().is_empty() => return Ok((response, messages)),
                Ok(_) => last_error = None,
                Err(error) => {
                    let context = recovery_context_from_prompt_error(error);
                    last_error = Some(context.message);
                    if let Some(failure_messages) = context.chat_history {
                        messages = failure_messages;
                    }
                }
            }
        }

        let error = match last_error {
            Some(error) => format!(
                "model failed to correct a tool error after {MAX_TOOL_RECOVERY_ATTEMPTS} recovery \
                 attempts: {error}"
            ),
            None => format!(
                "model produced no answer after correcting a tool error in \
                 {MAX_TOOL_RECOVERY_ATTEMPTS} recovery attempts"
            ),
        };

        Err(RecoveryFailure { error, messages })
    }

    /// Records and prints non-empty reasoning text while maintaining reasoning display state.
    ///
    /// The first reasoning fragment opens a `[thinking]` section. All fragments are written to
    /// stderr in the subdued terminal style and flushed immediately for live streaming output.
    fn handle_reasoning_text(&self, reasoning: &str, state: &mut StreamOutputState) {
        if reasoning.trim().is_empty() {
            return;
        }

        state.set_received_reasoning(true);
        if !state.showing_reasoning() {
            self.terminal_io.eprintln_gray("[thinking]");
            state.set_showing_reasoning(true);
        }

        self.terminal_io.eprint_gray(reasoning);
        self.terminal_io.flush_stderr();
    }

    /// Adapts an incremental reasoning delta to the shared reasoning renderer.
    fn handle_reasoning_delta(&self, reasoning: String, state: &mut StreamOutputState) {
        self.handle_reasoning_text(reasoning.as_str(), state);
    }

    /// Extracts display text from a complete reasoning item and sends it to the shared renderer.
    fn handle_reasoning(&self, reasoning: Reasoning, state: &mut StreamOutputState) {
        self.handle_reasoning_text(reasoning.display_text().as_str(), state);
    }

    /// Records and prints a non-empty answer-text item from the assistant stream.
    ///
    /// If reasoning was being displayed, this closes the thinking section and opens an `[answer]`
    /// section before writing and immediately flushing the response text to stdout.
    fn handle_text(&self, text: Text, state: &mut StreamOutputState) {
        if text.text().trim().is_empty() {
            return;
        }

        state.set_received_answer(true);
        if state.showing_reasoning() {
            self.terminal_io.eprintln("\n[answer]");
            state.set_showing_reasoning(false);
        }

        self.terminal_io.print(text.text());
        self.terminal_io.flush_stdout();
    }

    /// Prints the name and serialized arguments of a tool call emitted by the model.
    ///
    /// Tool execution is owned by Rig; this method only exposes the attempted invocation to the
    /// user as terminal diagnostics.
    fn handle_tool_call(&self, tool_call: ToolCall) {
        self.terminal_io.eprintln(
            format!(
                "\n[tool call: {}({})]",
                tool_call.function.name, tool_call.function.arguments
            )
            .as_str(),
        );
    }

    /// Records a tool result for recovery decisions and prints its model-visible content.
    ///
    /// Recovery status markers update `StreamOutputState`. Text and JSON results are rendered as
    /// received, while image payloads are represented by a stable `<image>` placeholder.
    fn handle_tool_result(&self, tool_result: ToolResult, state: &mut StreamOutputState) {
        state.record_tool_result(tool_recovery_status(&tool_result));
        let output = tool_result
            .content
            .iter()
            .map(|content| match content {
                ToolResultContent::Text(text) => text.text.clone(),
                ToolResultContent::Json { value } => value.to_string(),
                ToolResultContent::Image(_) => "<image>".to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");

        self.terminal_io
            .eprintln(format!("[tool result: {output}]").as_str());
    }

    /// Stores the canonical messages delivered with the stream's final response.
    ///
    /// The caller commits these messages only after it has confirmed that no additional answer or
    /// tool recovery is required for the completed stream.
    fn handle_final_response(
        &self,
        response: PromptResponse,
        streamed_messages: &mut Option<Vec<Message>>,
    ) {
        *streamed_messages = response.messages;
    }

    /// Routes one agent-stream item to the matching terminal or state handler.
    ///
    /// Reasoning, answer text, tool calls, tool results, and final history are handled explicitly.
    /// Telemetry and lifecycle variants that do not affect the CLI presentation are ignored.
    fn handle_stream_chunk<R>(
        &self,
        item: MultiTurnStreamItem<R>,
        output_state: &mut StreamOutputState,
        streamed_messages: &mut Option<Vec<Message>>,
    ) {
        match item {
            MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ReasoningDelta { reasoning, .. },
            ) => self.handle_reasoning_delta(reasoning, output_state),
            MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Reasoning(
                reasoning,
            )) => self.handle_reasoning(reasoning, output_state),
            MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text)) => {
                self.handle_text(text, output_state)
            }
            MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ToolCall {
                tool_call,
                ..
            }) => self.handle_tool_call(tool_call),
            MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                tool_result,
                ..
            }) => self.handle_tool_result(tool_result, output_state),
            MultiTurnStreamItem::FinalResponse(response) => {
                self.handle_final_response(response, streamed_messages)
            }
            _ => {}
        }
    }

    /// Prints a final answer produced by a non-streaming automatic recovery attempt.
    ///
    /// Recovery calls do not pass through the normal streamed text handler, so this method emits
    /// the answer section and flushes stdout explicitly.
    fn handle_recovered_response(&self, response: &str) {
        self.terminal_io.eprintln("\n[answer]");
        self.terminal_io.print(response);
        self.terminal_io.flush_stdout();
    }

    /// Notifies the configured history observer about the current committed messages.
    ///
    /// Messages are borrowed rather than cloned; the observer is expected to persist them during
    /// the callback; the await point lets the observer finish its I/O before the session continues.
    async fn notify_messages_changed(&self, messages: &[Message]) {
        let messages = messages.iter().collect::<Vec<_>>();
        (self.on_messages_change)(&messages).await;
    }

    /// Atomically replaces the session history and notifies the history observer.
    ///
    /// This is used for recovery outcomes because Rig may return a canonical history that differs
    /// from the history available before the failed run.
    async fn replace_messages(&self, new_messages: Vec<Message>) {
        let mut messages = self.messages.lock().await;
        *messages = new_messages;
        self.notify_messages_changed(&messages).await;
    }

    /// Commits either outcome of an automatic recovery attempt.
    ///
    /// Successful recovery prints the answer and stores its history. Exhausted recovery still
    /// stores the latest history, then returns the diagnostic error to the interactive loop so it
    /// can inform the user without losing the failed tool context.
    async fn handle_recovery_result(
        &self,
        result: Result<(String, Vec<Message>), RecoveryFailure>,
    ) -> anyhow::Result<()> {
        match result {
            Ok((response, messages)) => {
                self.handle_recovered_response(response.as_str());
                self.replace_messages(messages).await;
                Ok(())
            }
            Err(failure) => {
                self.replace_messages(failure.messages).await;
                anyhow::bail!(failure.error)
            }
        }
    }

    /// Builds recovery context after the agent stream itself returns an error.
    ///
    /// Canonical history embedded in the streaming error is preferred. If none is available, the
    /// previous session history plus the current user message becomes the recovery context.
    async fn recover_after_stream_error(
        &self,
        user_message: &str,
        error: StreamingError,
    ) -> anyhow::Result<()> {
        let context = recovery_context_from_streaming_error(error);
        let recovery_messages = if let Some(failure_messages) = context.chat_history {
            failure_messages
        } else {
            let mut messages = self.messages.lock().await.clone();
            messages.push(Message::user(user_message));
            messages
        };

        let result = self
            .recover_response(recovery_messages, context.message)
            .await;
        self.handle_recovery_result(result).await
    }

    /// Recovers after a completed stream contained reasoning but no final answer.
    ///
    /// Canonical streamed messages are appended to the session history when available. Otherwise,
    /// the original user message is appended so the recovery prompt still has the request context.
    async fn recover_after_missing_answer(
        &self,
        user_message: String,
        streamed_messages: &mut Option<Vec<Message>>,
    ) -> anyhow::Result<()> {
        let mut recovery_messages = self.messages.lock().await.clone();
        if let Some(messages) = streamed_messages.take() {
            recovery_messages.extend(messages);
        } else {
            recovery_messages.push(Message::user(user_message));
        }

        let result = self.recover_missing_answer(recovery_messages).await;
        self.handle_recovery_result(result).await
    }

    /// Recovers after a stream ended with an unresolved tool error and no final answer.
    ///
    /// The method prepares the most complete available history, delegates the corrective run to
    /// `recover_unresolved_tool`, and commits either its successful or exhausted result.
    async fn recover_after_unresolved_tool(
        &self,
        user_message: String,
        streamed_messages: &mut Option<Vec<Message>>,
    ) -> anyhow::Result<()> {
        let mut recovery_messages = self.messages.lock().await.clone();
        if let Some(messages) = streamed_messages.take() {
            recovery_messages.extend(messages);
        } else {
            recovery_messages.push(Message::user(user_message));
        }

        let result = self.recover_unresolved_tool(recovery_messages).await;
        self.handle_recovery_result(result).await
    }

    /// Appends canonical messages from a successful stream to the committed session history.
    ///
    /// Empty message collections are ignored. A non-empty append triggers the history observer so
    /// persisted chat state stays synchronized with the in-memory conversation.
    async fn append_streamed_messages(&self, messages: Vec<Message>) {
        if messages.is_empty() {
            return;
        }

        let mut current_messages = self.messages.lock().await;
        current_messages.extend(messages);
        self.notify_messages_changed(&current_messages).await;
    }

    /// Runs the interactive prompt loop until the user enters `/exit` or terminal input fails.
    ///
    /// Each user message starts a streamed agent run. Stream items are displayed as they arrive,
    /// then the completed turn is either committed or sent through the appropriate recovery path.
    /// Exhausted model recovery is reported to the terminal and control returns to `readline` so
    /// the user can guide the model; it does not terminate the application.
    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            let messages = self.messages.lock().await.clone();

            let mut user_message = self.terminal_io.readline()?;
            let command = self.command_parser.parse(user_message).await;
            match command {
                command_parser::CommandParserResult::CommandExit => break,
                command_parser::CommandParserResult::CommandContinue => continue,
                command_parser::CommandParserResult::Prompt(prompt, echo) => {
                    if echo {
                        self.terminal_io.print(format!("{prompt}\n").as_str());
                    }
                    user_message = prompt;
                }
                command_parser::CommandParserResult::New => {
                    self.replace_messages(Vec::new()).await;
                    continue;
                }
            }

            let mut stream = self.agent.stream_chat(user_message.clone(), messages).await;
            let mut output_state = StreamOutputState::default();
            let mut streamed_messages = None;
            let mut stream_error = None;

            while let Some(item) = stream.next().await {
                let item = match item {
                    Ok(item) => item,
                    Err(error) => {
                        stream_error = Some(error);
                        break;
                    }
                };

                self.handle_stream_chunk(item, &mut output_state, &mut streamed_messages);
            }

            let turn_result = if let Some(error) = stream_error {
                self.recover_after_stream_error(user_message.as_str(), error)
                    .await
            } else if output_state.requires_tool_recovery() {
                self.recover_after_unresolved_tool(user_message, &mut streamed_messages)
                    .await
            } else if output_state.requires_answer_recovery() {
                self.recover_after_missing_answer(user_message, &mut streamed_messages)
                    .await
            } else if let Some(messages) = streamed_messages {
                self.append_streamed_messages(messages).await;
                Ok(())
            } else {
                Ok(())
            };

            if let Err(error) = turn_result {
                self.terminal_io.eprintln(
                    format!("\n{}", format_recovery_stopped_notice(&error.to_string())).as_str(),
                );
            }

            // add newline
            self.terminal_io.eprintln("");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::StreamOutputState;
    use crate::use_cases::chat::tool_recovery::ToolRecoveryStatus;

    #[test]
    fn reasoning_without_answer_requires_recovery() {
        let state = StreamOutputState::new(false, true, false);

        assert!(state.requires_answer_recovery());
    }

    #[test]
    fn reasoning_with_answer_does_not_require_recovery() {
        let state = StreamOutputState::new(false, true, true);

        assert!(!state.requires_answer_recovery());
    }

    #[test]
    fn missing_reasoning_does_not_require_recovery() {
        assert!(!StreamOutputState::default().requires_answer_recovery());
    }

    #[test]
    fn tool_error_requires_a_follow_up_action() {
        let mut state = StreamOutputState::default();

        state.record_tool_result(ToolRecoveryStatus::Error);

        assert!(state.requires_tool_recovery());
        assert!(!state.requires_answer_recovery());
    }

    #[test]
    fn final_answer_after_tool_error_completes_recovery() {
        let mut state = StreamOutputState::default();
        state.record_tool_result(ToolRecoveryStatus::Error);

        state.set_received_answer(true);

        assert!(!state.requires_tool_recovery());
        assert!(!state.requires_answer_recovery());
    }

    #[test]
    fn successful_correction_requires_a_final_answer() {
        let mut state = StreamOutputState::default();
        state.record_tool_result(ToolRecoveryStatus::Error);

        state.record_tool_result(ToolRecoveryStatus::Recovered);

        assert!(!state.requires_tool_recovery());
        assert!(state.requires_answer_recovery());
    }

    #[test]
    fn tool_result_without_answer_requires_answer_recovery() {
        let mut state = StreamOutputState::default();

        state.record_tool_result(ToolRecoveryStatus::None);

        assert!(state.requires_answer_recovery());
    }
}
