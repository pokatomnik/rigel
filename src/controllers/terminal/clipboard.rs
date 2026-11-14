use std::io::Write;

use base64::{Engine, engine::general_purpose::STANDARD};

use crate::infrastructure::ui::clipboard::Clipboard;

/// Largest payload accepted by common OSC 52 terminal implementations.
const MAX_PAYLOAD_BYTES: usize = 48_000;

/// Copies text through the terminal's OSC 52 escape sequence.
///
/// OSC 52 works over SSH and inside tmux without an extra clipboard crate. Terminals that
/// disable it cannot acknowledge the write, so oversized payloads are rejected explicitly
/// instead of being truncated.
pub(crate) struct Osc52Clipboard {
    max_bytes: usize,
}

impl Osc52Clipboard {
    /// Creates a clipboard limited to the portable OSC 52 payload size.
    pub(crate) fn new() -> Self {
        Self {
            max_bytes: MAX_PAYLOAD_BYTES,
        }
    }

    fn escape_sequence(&self, text: &str) -> anyhow::Result<String> {
        let bytes = text.len();
        anyhow::ensure!(
            bytes <= self.max_bytes,
            "payload is {bytes} bytes; the terminal clipboard accepts at most {} bytes",
            self.max_bytes
        );
        Ok(format!("\x1b]52;c;{}\x07", STANDARD.encode(text)))
    }
}

impl Clipboard for Osc52Clipboard {
    fn copy(&self, text: &str) -> anyhow::Result<()> {
        let sequence = self.escape_sequence(text)?;
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(sequence.as_bytes())?;
        stdout.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Osc52Clipboard;

    #[test]
    fn payload_is_base64_encoded_inside_an_osc52_sequence() -> anyhow::Result<()> {
        let clipboard = Osc52Clipboard::new();
        let sequence = clipboard.escape_sequence("hello")?;

        assert_eq!(sequence, "\x1b]52;c;aGVsbG8=\x07");
        Ok(())
    }

    #[test]
    fn oversized_payload_is_rejected_without_truncation() {
        let clipboard = Osc52Clipboard { max_bytes: 4 };
        let error = clipboard.escape_sequence("hello").err();

        assert!(error.is_some_and(|error| error.to_string().contains("at most 4 bytes")));
    }
}
