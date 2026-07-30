use std::sync::Arc;

use futures::StreamExt;
use rig::{
    Agent,
    agent::{MultiTurnStreamItem, PromptResponse},
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
        stream_output_state::StreamOutputState, tool_recovery::tool_recovery_status,
    },
};

const EXIT: &str = "/exit";
const MAX_RECOVERY_ATTEMPTS: usize = 3;
const MAX_TOOL_RECOVERY_ATTEMPTS: usize = 2;

pub(crate) struct Chat<CM>
where
    CM: CompletionModel,
{
    agent: Agent<CM>,
    terminal_io: Arc<TerminalIO>,
    messages: Arc<Mutex<Vec<Message>>>,
}

impl<CM> Chat<CM>
where
    CM: CompletionModel + 'static,
{
    pub fn new(agent: Agent<CM>, terminal_io: Arc<TerminalIO>, messages: Vec<Message>) -> Self {
        let messages = Arc::new(Mutex::new(messages));
        Self {
            agent,
            terminal_io,
            messages,
        }
    }

    async fn recover_response(
        &self,
        user_message: &str,
        mut messages: Vec<Message>,
        initial_error: String,
    ) -> anyhow::Result<(String, Vec<Message>)> {
        messages.push(Message::user(user_message));
        let mut error = initial_error;

        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let recovery_prompt = recovery_prompt(error.as_str(), attempt, MAX_RECOVERY_ATTEMPTS);

            match self.agent.chat(recovery_prompt, &mut messages).await {
                Ok(response) => return Ok((response, messages)),
                Err(recovery_error) => error = recovery_error.to_string(),
            }
        }

        anyhow::bail!("model failed to recover after {MAX_RECOVERY_ATTEMPTS} attempts: {error}")
    }

    async fn recover_missing_answer(
        &self,
        mut messages: Vec<Message>,
    ) -> anyhow::Result<(String, Vec<Message>)> {
        let mut last_error = None;

        for attempt in 1..=MAX_RECOVERY_ATTEMPTS {
            let recovery_prompt = missing_answer_recovery_prompt(attempt, MAX_RECOVERY_ATTEMPTS);

            match self.agent.chat(recovery_prompt, &mut messages).await {
                Ok(response) if !response.trim().is_empty() => return Ok((response, messages)),
                Ok(_) => last_error = None,
                Err(error) => last_error = Some(error.to_string()),
            }
        }

        if let Some(error) = last_error {
            anyhow::bail!(
                "model failed to produce an answer after {MAX_RECOVERY_ATTEMPTS} recovery \
                 attempts: {error}"
            );
        }

        anyhow::bail!("model produced no answer after {MAX_RECOVERY_ATTEMPTS} recovery attempts")
    }

    async fn recover_unresolved_tool(
        &self,
        mut messages: Vec<Message>,
    ) -> anyhow::Result<(String, Vec<Message>)> {
        let mut last_error = None;

        for attempt in 1..=MAX_TOOL_RECOVERY_ATTEMPTS {
            let recovery_prompt =
                unresolved_tool_recovery_prompt(attempt, MAX_TOOL_RECOVERY_ATTEMPTS);

            match self.agent.chat(recovery_prompt, &mut messages).await {
                Ok(response) if !response.trim().is_empty() => return Ok((response, messages)),
                Ok(_) => last_error = None,
                Err(error) => last_error = Some(error.to_string()),
            }
        }

        if let Some(error) = last_error {
            anyhow::bail!(
                "model failed to correct a tool error after {MAX_TOOL_RECOVERY_ATTEMPTS} recovery \
                 attempts: {error}"
            );
        }

        anyhow::bail!(
            "model produced no answer after correcting a tool error in \
             {MAX_TOOL_RECOVERY_ATTEMPTS} recovery attempts"
        )
    }

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

    /// Handles an incremental reasoning-text chunk from the assistant stream.
    fn handle_reasoning_delta(&self, reasoning: String, state: &mut StreamOutputState) {
        self.handle_reasoning_text(reasoning.as_str(), state);
    }

    /// Handles a complete reasoning item from the assistant stream.
    fn handle_reasoning(&self, reasoning: Reasoning, state: &mut StreamOutputState) {
        self.handle_reasoning_text(reasoning.display_text().as_str(), state);
    }

    /// Handles an answer-text item from the assistant stream.
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

    /// Handles a tool-call item emitted by the assistant stream.
    fn handle_tool_call(&self, tool_call: ToolCall) {
        self.terminal_io.eprintln(
            format!(
                "\n[tool call: {}({})]",
                tool_call.function.name, tool_call.function.arguments
            )
            .as_str(),
        );
    }

    /// Handles a tool-result item emitted by the user stream.
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

    /// Handles the final-response item that contains the streamed conversation messages.
    fn handle_final_response(
        &self,
        response: PromptResponse,
        streamed_messages: &mut Option<Vec<Message>>,
    ) {
        *streamed_messages = response.messages;
    }

    /// Handles a chunk of response from LLM-provided stream
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

    fn handle_recovered_response(&self, response: &str) {
        self.terminal_io.eprintln("\n[answer]");
        self.terminal_io.print(response);
        self.terminal_io.flush_stdout();
    }

    async fn recover_after_stream_error(
        &self,
        user_message: &str,
        error: String,
    ) -> anyhow::Result<()> {
        let (response, recovered_messages) = self
            .recover_response(user_message, self.messages.lock().await.clone(), error)
            .await?;

        self.handle_recovered_response(response.as_str());
        *self.messages.lock().await = recovered_messages;

        Ok(())
    }

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

        let (response, recovered_messages) = self.recover_missing_answer(recovery_messages).await?;

        self.handle_recovered_response(response.as_str());
        *self.messages.lock().await = recovered_messages;

        Ok(())
    }

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

        let (response, recovered_messages) =
            self.recover_unresolved_tool(recovery_messages).await?;

        self.handle_recovered_response(response.as_str());
        *self.messages.lock().await = recovered_messages;

        Ok(())
    }

    async fn append_streamed_messages(&self, messages: Vec<Message>) {
        self.messages.lock().await.extend(messages);
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            let messages = self.messages.lock().await.clone();

            let user_message = self.terminal_io.readline()?;

            if user_message == EXIT {
                break;
            }

            let mut stream = self.agent.stream_chat(user_message.clone(), messages).await;
            let mut output_state = StreamOutputState::default();
            let mut streamed_messages = None;
            let mut stream_error = None;

            while let Some(item) = stream.next().await {
                let item = match item {
                    Ok(item) => item,
                    Err(error) => {
                        stream_error = Some(error.to_string());
                        break;
                    }
                };

                self.handle_stream_chunk(item, &mut output_state, &mut streamed_messages);
            }

            if let Some(error) = stream_error {
                self.recover_after_stream_error(user_message.as_str(), error)
                    .await?;
            } else if output_state.requires_tool_recovery() {
                self.recover_after_unresolved_tool(user_message, &mut streamed_messages)
                    .await?;
            } else if output_state.requires_answer_recovery() {
                self.recover_after_missing_answer(user_message, &mut streamed_messages)
                    .await?;
            } else if let Some(messages) = streamed_messages {
                self.append_streamed_messages(messages).await;
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
