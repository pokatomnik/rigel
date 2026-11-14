const OVERFLOW_MARKERS: [&str; 8] = [
    "context length",
    "context window",
    "context_length_exceeded",
    "maximum context",
    "too many tokens",
    "prompt is too long",
    "input is too long",
    "reduce the length",
];

/// Reports whether a provider error describes a context window overflow.
///
/// The detector is intentionally conservative: it only returns true for messages that name
/// the context limit, so unrelated provider failures keep their normal recovery path.
pub(crate) fn is_context_overflow(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    OVERFLOW_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::is_context_overflow;

    #[test]
    fn recognized_overflow_messages_are_detected() {
        for message in [
            "This model's maximum context length is 8192 tokens",
            "context_length_exceeded",
            "The prompt is too long for the context window",
            "Too many tokens in the request",
        ] {
            assert!(is_context_overflow(message), "{message}");
        }
    }

    #[test]
    fn unrelated_failures_keep_their_normal_path() {
        for message in [
            "connection refused",
            "invalid api key",
            "tool call failed",
            "rate limit exceeded",
        ] {
            assert!(!is_context_overflow(message), "{message}");
        }
    }
}
