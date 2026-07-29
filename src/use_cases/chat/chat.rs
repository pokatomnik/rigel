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
    prompts::recovery::recovery_prompt,
    shared::terminal_io::{ReadlineResult, TerminalIO},
};

const EXIT: &str = "/exit";
const MAX_RECOVERY_ATTEMPTS: usize = 3;

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

    fn handle_reasoning_text(&self, reasoning: &str, showing_reasoning: &mut bool) {
        if !*showing_reasoning {
            self.terminal_io.eprintln_gray("[thinking]");
            *showing_reasoning = true;
        }

        self.terminal_io.eprint_gray(reasoning);
        self.terminal_io.flush_stderr();
    }

    fn handle_reasoning_delta(&self, reasoning: String, showing_reasoning: &mut bool) {
        self.handle_reasoning_text(reasoning.as_str(), showing_reasoning);
    }

    fn handle_reasoning(&self, reasoning: Reasoning, showing_reasoning: &mut bool) {
        self.handle_reasoning_text(reasoning.display_text().as_str(), showing_reasoning);
    }

    fn handle_text(&self, text: Text, showing_reasoning: &mut bool) {
        if *showing_reasoning {
            self.terminal_io.eprintln("\n[answer]");
            *showing_reasoning = false;
        }

        self.terminal_io.print(text.text());
        self.terminal_io.flush_stdout();
    }

    fn handle_tool_call(&self, tool_call: ToolCall) {
        self.terminal_io.eprintln(
            format!(
                "\n[tool call: {}({})]",
                tool_call.function.name, tool_call.function.arguments
            )
            .as_str(),
        );
    }

    fn handle_tool_result(&self, tool_result: ToolResult) {
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

    fn handle_final_response(
        &self,
        response: PromptResponse,
        streamed_messages: &mut Option<Vec<Message>>,
    ) {
        *streamed_messages = response.messages;
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            let messages = self.messages.lock().await.clone();

            let ReadlineResult::Line(user_message) = self.terminal_io.readline()? else {
                return Ok(());
            };

            if user_message == EXIT {
                break;
            }

            let mut stream = self.agent.stream_chat(user_message.clone(), messages).await;
            let mut showing_reasoning = false;
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

                match item {
                    MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::ReasoningDelta { reasoning, .. },
                    ) => self.handle_reasoning_delta(reasoning, &mut showing_reasoning),
                    MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::Reasoning(reasoning),
                    ) => self.handle_reasoning(reasoning, &mut showing_reasoning),
                    MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                        text,
                    )) => self.handle_text(text, &mut showing_reasoning),
                    MultiTurnStreamItem::StreamAssistantItem(
                        StreamedAssistantContent::ToolCall { tool_call, .. },
                    ) => self.handle_tool_call(tool_call),
                    MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                        tool_result,
                        ..
                    }) => self.handle_tool_result(tool_result),
                    MultiTurnStreamItem::FinalResponse(response) => {
                        self.handle_final_response(response, &mut streamed_messages)
                    }
                    _ => {}
                }
            }

            if let Some(error) = stream_error {
                let (response, recovered_messages) = self
                    .recover_response(
                        user_message.as_str(),
                        self.messages.lock().await.clone(),
                        error,
                    )
                    .await?;

                self.terminal_io.eprintln("\n[answer]");
                self.terminal_io.print(response.as_str());
                self.terminal_io.flush_stdout();
                *self.messages.lock().await = recovered_messages;
            } else if let Some(messages) = streamed_messages {
                self.messages.lock().await.extend(messages);
            }

            // add newline
            self.terminal_io.eprintln("");
        }

        Ok(())
    }
}
