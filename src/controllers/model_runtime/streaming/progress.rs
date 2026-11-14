use std::{collections::HashMap, sync::Arc};

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
        recovery::tool_recovery::{ToolRecoveryStatus, tool_recovery_status},
        streaming::{
            message_output::{StreamDisplay, format_tool_result_full, stream_tool_call},
            stream_output_state::StreamOutputState,
            turn_journal::{ToolResultRecord, TurnJournal},
        },
    },
    controllers::terminal::terminal_io::TerminalIO,
    entities::{
        agent_event::event::AgentEvent,
        conversation::context_usage::ContextUsage,
        transcript::entry::{EntryId, TranscriptEntry, TranscriptEntryKind},
    },
    infrastructure::ui::gateway::EventSink,
};

/// Owns stream events and journal state until a streamed turn is finalized.
pub(super) struct StreamProgress {
    output_state: StreamOutputState,
    final_response: Option<PromptResponse>,
    stream_error: Option<StreamingError>,
    last_usage: Option<Usage>,
    journal: TurnJournal,
    events: Option<Arc<dyn EventSink>>,
    turn_id: u64,
    tool_names: HashMap<String, String>,
}

impl StreamProgress {
    /// Starts an event accumulator for the current prompt and provider history.
    pub(super) fn new(
        base: Vec<Message>,
        prompt: String,
        events: Option<Arc<dyn EventSink>>,
        turn_id: u64,
    ) -> Self {
        Self {
            output_state: StreamOutputState::default(),
            final_response: None,
            stream_error: None,
            last_usage: None,
            journal: TurnJournal::new(base, prompt),
            events,
            turn_id,
            tool_names: HashMap::new(),
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
                let live = self.events.is_some();
                StreamDisplay::new(terminal_io, &mut self.output_state)
                    .reasoning(reasoning.as_str(), live);
                self.emit_answer_delta(reasoning.as_str(), true);
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                let live = self.events.is_some();
                let text = reasoning.display_text();
                StreamDisplay::new(terminal_io, &mut self.output_state)
                    .reasoning(text.as_str(), live);
                self.emit_answer_delta(text.as_str(), true);
            }
            StreamedAssistantContent::Text(text) => {
                let live = self.events.is_some();
                StreamDisplay::new(terminal_io, &mut self.output_state).text(&text, live);
                self.emit_answer_delta(text.text(), false);
            }
            StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            } => {
                stream_tool_call(terminal_io, &tool_call, self.events.is_some());
                self.record_tool_call(&tool_call);
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
        let live = self.events.is_some();
        StreamDisplay::new(terminal_io, &mut self.output_state).tool_result(&tool_result, live);
        self.record_tool_result(&tool_result);
        let record = self
            .journal
            .record_tool_result(tool_result, internal_call_id.as_str());
        self.persist_tool_result(record, history).await
    }

    fn emit_answer_delta(&self, text: &str, reasoning: bool) {
        if text.is_empty() {
            return;
        }
        let Some(events) = self.events.as_ref() else {
            return;
        };
        let event = if reasoning {
            AgentEvent::ReasoningDelta {
                turn_id: self.turn_id,
                text: text.to_string(),
            }
        } else {
            AgentEvent::AnswerDelta {
                turn_id: self.turn_id,
                text: text.to_string(),
            }
        };
        events.emit(event);
    }

    fn record_tool_call(&mut self, tool_call: &rig::message::ToolCall) {
        let call_id = tool_call
            .call_id
            .clone()
            .unwrap_or_else(|| tool_call.id.clone());
        self.tool_names
            .insert(call_id.clone(), tool_call.function.name.clone());
        let Some(events) = self.events.as_ref() else {
            return;
        };
        events.emit(AgentEvent::TranscriptEntryAdded(TranscriptEntry::new(
            EntryId::from_call_id(call_id.as_str()),
            TranscriptEntryKind::ToolCall {
                call_id,
                tool_name: tool_call.function.name.clone(),
                arguments: tool_call.function.arguments.clone(),
            },
        )));
    }

    fn record_tool_result(&mut self, tool_result: &rig::message::ToolResult) {
        let call_id = tool_result
            .call_id
            .clone()
            .unwrap_or_else(|| tool_result.id.clone());
        let tool_name = self
            .tool_names
            .get(call_id.as_str())
            .cloned()
            .unwrap_or_else(|| "tool".to_string());
        let failed = matches!(tool_recovery_status(tool_result), ToolRecoveryStatus::Error);
        let Some(events) = self.events.as_ref() else {
            return;
        };
        events.emit(AgentEvent::TranscriptEntryReplaced(TranscriptEntry::new(
            EntryId::from_call_id(call_id.as_str()),
            TranscriptEntryKind::ToolResult {
                call_id,
                tool_name,
                output: format_tool_result_full(tool_result),
                failed,
            },
        )));
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
