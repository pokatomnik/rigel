use rig::{
    OneOrMany,
    agent::{CompletionCall, MultiTurnStreamItem, PromptResponse, StreamingError},
    completion::Usage,
    message::{Message, UserContent},
    streaming::{StreamedAssistantContent, StreamedUserContent},
};

use crate::{
    controllers::model_runtime::{
        history::{
            chat_history::{ChatHistory, HistoryUpdate},
            history_persistence::HistoryPersistence,
        },
        streaming::{
            message_output::{
                print_reasoning, print_reasoning_block, print_text, print_tool_call,
                print_tool_result,
            },
            stream_output_state::StreamOutputState,
            turn_journal::{ToolResultRecord, TurnJournal},
        },
    },
    controllers::terminal::terminal_io::TerminalIO,
    entities::conversation::context_usage::ContextUsage,
};

/// Owns stream events and journal state until a streamed turn is finalized.
pub(super) struct StreamProgress {
    output_state: StreamOutputState,
    final_response: Option<PromptResponse>,
    stream_error: Option<StreamingError>,
    last_usage: Option<Usage>,
    journal: TurnJournal,
}

impl StreamProgress {
    /// Starts an event accumulator for the current prompt and provider history.
    pub(super) fn new(base: Vec<Message>, prompt: String) -> Self {
        Self {
            output_state: StreamOutputState::default(),
            final_response: None,
            stream_error: None,
            last_usage: None,
            journal: TurnJournal::new(base, prompt),
        }
    }

    /// Returns context usage from the latest provider usage event and configured limit.
    pub(super) fn context_usage(&self, max_context_tokens: Option<u64>) -> ContextUsage {
        ContextUsage::from_total_tokens(
            self.last_usage
                .map(|usage| usage.total_tokens)
                .filter(|tokens| *tokens != 0),
            max_context_tokens,
        )
    }

    /// Takes a provider stream error, if one ended the event loop.
    pub(super) fn take_stream_error(&mut self) -> Option<StreamingError> {
        self.stream_error.take()
    }

    /// Takes the final provider response after stream completion.
    pub(super) fn take_final_response(&mut self) -> Option<PromptResponse> {
        self.final_response.take()
    }

    /// Moves accumulated display state and usage into the completed response.
    pub(super) fn into_completion_state(
        self,
        max_context_tokens: Option<u64>,
    ) -> (StreamOutputState, ContextUsage) {
        let context_usage = self.context_usage(max_context_tokens);
        (self.output_state, context_usage)
    }

    /// Applies one provider event, returning false only when the stream should stop.
    pub(super) async fn accept<R, P>(
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
                self.handle_assistant(content, terminal_io, history).await?;
            }
            MultiTurnStreamItem::StreamUserItem(content) => {
                self.handle_user(content, terminal_io, history).await?;
            }
            MultiTurnStreamItem::CompletionCall(CompletionCall { usage, .. }) => {
                self.last_usage = Some(usage);
                self.journal.rebase(history.snapshot_result().await?);
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
    ) -> anyhow::Result<()>
    where
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
                    .record_tool_call(history.snapshot_result().await?, internal_call_id);
            }
            _ => {}
        }
        Ok(())
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
