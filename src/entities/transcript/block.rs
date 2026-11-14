use serde::{Deserialize, Serialize};

use super::entry::TranscriptEntryKind;

/// Runtime tool name whose call and result are rendered as one isolated subagent block.
pub(crate) const SUBAGENT_TOOL_NAME: &str = "spawn_subagent";

/// Collapsible presentation groups recognized by the transcript renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BlockKind {
    /// Tool call and its result.
    Tool,
    /// Assistant reasoning rendered separately from the answer.
    Reasoning,
    /// Isolated subagent prompt, activity, and report.
    Subagent,
    /// Compaction summary.
    Summary,
}

impl BlockKind {
    /// Returns the block kind for an entry, or `None` when the entry is plain text.
    pub(crate) fn from_entry(entry: &TranscriptEntryKind) -> Option<Self> {
        match entry {
            TranscriptEntryKind::ToolCall { tool_name, .. }
            | TranscriptEntryKind::ToolResult { tool_name, .. } => Some(Self::for_tool(tool_name)),
            TranscriptEntryKind::Assistant {
                reasoning: Some(_), ..
            } => Some(Self::Reasoning),
            TranscriptEntryKind::Summary { .. } => Some(Self::Summary),
            TranscriptEntryKind::User { .. } | TranscriptEntryKind::Boundary(_) => None,
            TranscriptEntryKind::Assistant {
                reasoning: None, ..
            } => None,
        }
    }

    /// Reports whether the block is collapsed when it first appears.
    pub(crate) const fn starts_collapsed(self) -> bool {
        true
    }

    fn for_tool(tool_name: &str) -> Self {
        if tool_name == SUBAGENT_TOOL_NAME {
            Self::Subagent
        } else {
            Self::Tool
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockKind, SUBAGENT_TOOL_NAME};
    use crate::entities::transcript::{boundary::CompactionBoundary, entry::TranscriptEntryKind};

    #[test]
    fn tool_calls_and_results_share_the_tool_block() {
        let call = TranscriptEntryKind::ToolCall {
            call_id: "call-1".to_string(),
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        };
        let result = TranscriptEntryKind::ToolResult {
            call_id: "call-1".to_string(),
            tool_name: "read_file".to_string(),
            output: "contents".to_string(),
            failed: false,
        };

        assert_eq!(BlockKind::from_entry(&call), Some(BlockKind::Tool));
        assert_eq!(BlockKind::from_entry(&result), Some(BlockKind::Tool));
    }

    #[test]
    fn subagent_tool_is_isolated_from_ordinary_tools() {
        let call = TranscriptEntryKind::ToolCall {
            call_id: "call-2".to_string(),
            tool_name: SUBAGENT_TOOL_NAME.to_string(),
            arguments: serde_json::json!({"task": "delegate"}),
        };
        let result = TranscriptEntryKind::ToolResult {
            call_id: "call-2".to_string(),
            tool_name: SUBAGENT_TOOL_NAME.to_string(),
            output: "report".to_string(),
            failed: false,
        };

        assert_eq!(BlockKind::from_entry(&call), Some(BlockKind::Subagent));
        assert_eq!(BlockKind::from_entry(&result), Some(BlockKind::Subagent));
    }

    #[test]
    fn plain_entries_have_no_collapsible_block() {
        let user = TranscriptEntryKind::User {
            text: "hello".to_string(),
        };
        let assistant = TranscriptEntryKind::Assistant {
            text: "hi".to_string(),
            reasoning: None,
        };
        let boundary = TranscriptEntryKind::Boundary(CompactionBoundary::new("chat.0.jsonl"));

        assert_eq!(BlockKind::from_entry(&user), None);
        assert_eq!(BlockKind::from_entry(&assistant), None);
        assert_eq!(BlockKind::from_entry(&boundary), None);
    }

    #[test]
    fn reasoning_and_summary_have_their_own_blocks() {
        let reasoning = TranscriptEntryKind::Assistant {
            text: "answer".to_string(),
            reasoning: Some("thought".to_string()),
        };
        let summary = TranscriptEntryKind::Summary {
            text: "summary".to_string(),
        };

        assert_eq!(
            BlockKind::from_entry(&reasoning),
            Some(BlockKind::Reasoning)
        );
        assert_eq!(BlockKind::from_entry(&summary), Some(BlockKind::Summary));
        assert!(BlockKind::Subagent.starts_collapsed());
    }
}
