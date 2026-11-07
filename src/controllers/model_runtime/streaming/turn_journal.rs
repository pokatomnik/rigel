use rig::{
    OneOrMany,
    message::{Message, ToolResult, UserContent},
};

/// Builds canonical tool-result snapshots for one streamed turn.
pub(crate) struct TurnJournal {
    messages: Vec<Message>,
    calls: Vec<RecordedCall>,
    result_message_index: Option<usize>,
}

struct RecordedCall {
    internal_call_id: String,
    result_recorded: bool,
}

pub(crate) enum ToolResultRecord {
    Updated(Vec<Message>),
    Unmatched(ToolResult),
    Ignored,
}

impl TurnJournal {
    pub(crate) fn new(mut base: Vec<Message>, prompt: String) -> Self {
        base.push(Message::user(prompt));
        Self {
            messages: base,
            calls: Vec::new(),
            result_message_index: None,
        }
    }

    pub(crate) fn rebase(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        self.calls.clear();
        self.result_message_index = None;
    }

    pub(crate) fn record_tool_call(&mut self, messages: Vec<Message>, internal_call_id: String) {
        if self.calls.is_empty() || self.calls.iter().all(|call| call.result_recorded) {
            self.rebase(messages);
        }

        if self
            .calls
            .iter()
            .any(|call| call.internal_call_id == internal_call_id)
        {
            return;
        }

        self.calls.push(RecordedCall {
            internal_call_id,
            result_recorded: false,
        });
    }

    pub(crate) fn record_tool_result(
        &mut self,
        tool_result: ToolResult,
        internal_call_id: &str,
    ) -> ToolResultRecord {
        let Some(call) = self
            .calls
            .iter_mut()
            .find(|call| call.internal_call_id == internal_call_id)
        else {
            return ToolResultRecord::Unmatched(tool_result);
        };
        if call.result_recorded {
            return ToolResultRecord::Ignored;
        }
        call.result_recorded = true;

        if let Some(index) = self.result_message_index
            && let Some(Message::User { content }) = self.messages.get_mut(index)
        {
            content.push(UserContent::ToolResult(tool_result));
            return ToolResultRecord::Updated(self.messages.clone());
        }

        self.start_result_message(tool_result)
    }

    fn start_result_message(&mut self, tool_result: ToolResult) -> ToolResultRecord {
        let index = self.messages.len();
        self.messages.push(Message::User {
            content: OneOrMany::one(UserContent::ToolResult(tool_result)),
        });
        self.result_message_index = Some(index);
        ToolResultRecord::Updated(self.messages.clone())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, ensure};
    use rig::{
        OneOrMany,
        message::{
            AssistantContent, Message, ToolCall, ToolFunction, ToolResult, ToolResultContent,
            UserContent,
        },
    };

    use super::{ToolResultRecord, TurnJournal};

    #[test]
    fn two_results_share_one_user_message() -> Result<()> {
        let canonical = canonical_tool_history(&["call-1", "call-2"]);
        let mut journal = TurnJournal::new(Vec::new(), "prompt".to_string());
        journal.record_tool_call(canonical.clone(), "internal-1".to_string());
        journal.record_tool_call(canonical, "internal-2".to_string());

        updated(journal.record_tool_result(tool_result("call-1", "first"), "internal-1"))?;
        let snapshot =
            updated(journal.record_tool_result(tool_result("call-2", "second"), "internal-2"))?;

        ensure!(
            result_message_lengths(&snapshot) == [2],
            "tool results were not kept in one complete message"
        );
        Ok(())
    }

    #[test]
    fn unmatched_and_duplicate_results_are_classified() -> Result<()> {
        let canonical = canonical_tool_history(&["call-1"]);
        let mut journal = TurnJournal::new(Vec::new(), "prompt".to_string());
        journal.record_tool_call(canonical, "internal-1".to_string());

        let unmatched = journal.record_tool_result(tool_result("other", "ignored"), "unknown");
        ensure!(matches!(unmatched, ToolResultRecord::Unmatched(_)));
        let matched = journal.record_tool_result(tool_result("call-1", "done"), "internal-1");
        ensure!(matches!(matched, ToolResultRecord::Updated(_)));
        let duplicate =
            journal.record_tool_result(tool_result("call-1", "duplicate"), "internal-1");
        ensure!(matches!(duplicate, ToolResultRecord::Ignored));
        Ok(())
    }

    #[test]
    fn next_tool_batch_rebases_onto_new_canonical_history() -> Result<()> {
        let first = canonical_tool_history(&["call-1"]);
        let mut journal = TurnJournal::new(Vec::new(), "prompt".to_string());
        journal.record_tool_call(first, "internal-1".to_string());
        updated(journal.record_tool_result(tool_result("call-1", "done"), "internal-1"))?;

        let mut second = canonical_tool_history(&["call-2"]);
        second.insert(0, Message::user("preserved canonical retry"));
        journal.record_tool_call(second.clone(), "internal-2".to_string());
        let snapshot =
            updated(journal.record_tool_result(tool_result("call-2", "done"), "internal-2"))?;

        ensure!(
            snapshot.starts_with(second.as_slice()),
            "canonical rebase was lost"
        );
        Ok(())
    }

    fn updated(record: ToolResultRecord) -> Result<Vec<Message>> {
        match record {
            ToolResultRecord::Updated(messages) => Ok(messages),
            _ => anyhow::bail!("tool result did not update the journal"),
        }
    }

    fn result_message_lengths(messages: &[Message]) -> Vec<usize> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::User { content }
                    if content
                        .iter()
                        .any(|item| matches!(item, UserContent::ToolResult(_))) =>
                {
                    Some(content.len())
                }
                _ => None,
            })
            .collect()
    }

    fn canonical_tool_history(ids: &[&str]) -> Vec<Message> {
        let calls = ids.iter().map(|id| {
            AssistantContent::ToolCall(ToolCall::new(
                (*id).to_string(),
                ToolFunction::new("tool".to_string(), serde_json::json!({})),
            ))
        });
        let Some(content) = OneOrMany::from_iter_optional(calls) else {
            return vec![Message::user("prompt")];
        };
        vec![
            Message::user("prompt"),
            Message::Assistant { id: None, content },
        ]
    }

    fn tool_result(id: &str, output: &str) -> ToolResult {
        ToolResult {
            id: id.to_string(),
            call_id: None,
            content: OneOrMany::one(ToolResultContent::text(output)),
        }
    }
}
