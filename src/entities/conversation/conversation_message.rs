/// Provider-neutral message in a conversation history.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConversationMessage {
    /// Exact provider message JSON retained as one conversation boundary.
    ///
    /// The raw value preserves ordered multi-part content, assistant IDs, tool call IDs,
    /// JSON/image payloads, and provider-specific metadata while keeping `entities` free of Rig
    /// types. Runtime adapters decode it only at the provider boundary.
    Provider {
        /// Serialized provider message, including its role and complete content array.
        raw: serde_json::Value,
    },
    /// Text authored by the system or a runtime preamble.
    System(String),
    /// Text authored by the user.
    User(String),
    /// Text authored by the assistant.
    Assistant(String),
}
