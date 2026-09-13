/// Provider-neutral message in a conversation history.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConversationMessage {
    /// Text authored by the system or a runtime preamble.
    System(String),
    /// Text authored by the user.
    User(String),
    /// Text authored by the assistant.
    Assistant(String),
    /// Assistant text with opaque provider metadata retained at the runtime boundary.
    AssistantWithMetadata {
        /// Text content shown to the user.
        text: String,
        /// Provider-neutral opaque metadata that must survive retries.
        metadata: serde_json::Value,
    },
    /// A complete assistant tool call boundary.
    AssistantToolCall {
        /// Provider call identifier.
        id: String,
        /// Runtime tool name.
        name: String,
        /// JSON arguments encoded as text.
        arguments: String,
    },
    /// A tool result returned to the model.
    ToolResult {
        /// Provider call identifier.
        id: String,
        /// Runtime result content.
        content: String,
    },
}
