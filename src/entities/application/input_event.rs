use serde::{Deserialize, Serialize};

/// Terminal-independent size reported by a resize event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerminalSize {
    /// Number of text columns.
    pub(crate) width: u16,
    /// Number of text rows.
    pub(crate) height: u16,
}

impl TerminalSize {
    /// Creates a size from the columns and rows reported by the terminal.
    pub(crate) const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

/// Terminal-independent key identity interpreted by the UI workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum KeyStroke {
    /// Line break in the composer.
    Enter,
    /// Focus or activation movement.
    Tab,
    /// Soft stop outside modals; safe refusal inside modals.
    Escape,
    /// Remove the previous character.
    Backspace,
    /// Remove the next character.
    Delete,
    /// Move the cursor one column left.
    Left,
    /// Move the cursor one column right.
    Right,
    /// Move the cursor one line up.
    Up,
    /// Move the cursor one line down.
    Down,
    /// Move to the beginning of the line.
    Home,
    /// Move to the end of the line.
    End,
    /// Scroll one page up.
    PageUp,
    /// Scroll one page down.
    PageDown,
    /// Submit on Windows and Linux.
    CtrlEnter,
    /// Submit on macOS.
    CmdEnter,
    /// A control chord identified by its lowercase character.
    Ctrl(char),
}

/// Provider-neutral user input delivered to the UI workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum InputEvent {
    /// Printable text inserted into the composer, including pasted blocks.
    Text(String),
    /// A key press interpreted by the UI workflow.
    Key(KeyStroke),
    /// The terminal reported a new size.
    Resize(TerminalSize),
    /// The terminal input stream ended.
    Closed,
}

#[cfg(test)]
mod tests {
    use super::{InputEvent, KeyStroke, TerminalSize};

    #[test]
    fn resize_carries_both_dimensions() {
        let event = InputEvent::Resize(TerminalSize::new(60, 20));

        assert_eq!(event, InputEvent::Resize(TerminalSize::new(60, 20)));
    }

    #[test]
    fn key_events_round_trip_through_json() -> anyhow::Result<()> {
        for event in [
            InputEvent::Key(KeyStroke::CtrlEnter),
            InputEvent::Key(KeyStroke::CmdEnter),
            InputEvent::Key(KeyStroke::Ctrl('k')),
            InputEvent::Key(KeyStroke::Escape),
            InputEvent::Text("hello".to_string()),
            InputEvent::Closed,
        ] {
            let json = serde_json::to_value(&event)?;
            let restored: InputEvent = serde_json::from_value(json)?;
            assert_eq!(restored, event);
        }
        Ok(())
    }
}
