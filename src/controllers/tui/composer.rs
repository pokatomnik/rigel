use ratatui_textarea::{CursorMove, TextArea};

use crate::entities::application::input_event::{InputEvent, KeyStroke};

/// Multiline draft editor owned by the terminal boundary.
pub(crate) struct Composer {
    textarea: TextArea<'static>,
}

fn placeholder() -> String {
    "Enter sends · Alt+Enter: newline".to_string()
}

impl Composer {
    /// Creates an empty no-wrap editor.
    pub(crate) fn new() -> Self {
        let mut textarea = TextArea::new(vec![String::new()]);
        textarea.set_placeholder_text(placeholder());
        Self { textarea }
    }

    /// Applies one terminal event that the workflow forwarded to the editor.
    pub(crate) fn apply(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Text(text) => {
                self.textarea.insert_str(text);
            }
            InputEvent::Paste(text) => {
                self.textarea.insert_str(text);
            }
            InputEvent::Key(key) => self.apply_key(*key),
            InputEvent::Resize(_) | InputEvent::Closed => {}
        }
    }

    /// Returns the complete draft with its original line breaks.
    pub(crate) fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Reports whether the draft has no characters.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.textarea.is_empty()
    }

    /// Removes every character after a successful submission.
    pub(crate) fn clear(&mut self) {
        self.textarea = TextArea::new(vec![String::new()]);
        self.textarea.set_placeholder_text(placeholder());
    }

    /// Returns the number of authored lines, including the current empty line.
    pub(crate) fn line_count(&self) -> usize {
        self.textarea.lines().len().max(1)
    }

    /// Returns the mutable widget for renderer styling.
    pub(crate) fn textarea_mut(&mut self) -> &mut TextArea<'static> {
        &mut self.textarea
    }

    /// Returns the widget for drawing.
    pub(crate) fn textarea(&self) -> &TextArea<'static> {
        &self.textarea
    }

    /// Returns the cursor position inside the composer, relative to the frame.
    pub(crate) fn cursor_position(&self, area: ratatui::layout::Rect) -> Option<(u16, u16)> {
        let inner_width = area.width.saturating_sub(2);
        let inner_height = area.height.saturating_sub(2);
        let cursor = self.textarea.cursor();
        let row = u16::try_from(cursor.0).ok()?;
        let line = self.textarea.lines().get(cursor.0)?;
        let column = u16::try_from(ratatui::text::Line::from(line.as_str()).width()).ok()?;
        if row >= inner_height || column >= inner_width {
            return None;
        }
        Some((area.x + 1 + column, area.y + 1 + row))
    }

    fn apply_key(&mut self, key: KeyStroke) {
        match key {
            KeyStroke::CtrlEnter | KeyStroke::CmdEnter | KeyStroke::ShiftEnter => {
                self.textarea.insert_newline();
            }
            KeyStroke::Tab => {
                self.textarea.insert_tab();
            }
            KeyStroke::Backspace => {
                self.textarea.delete_char();
            }
            KeyStroke::Delete => {
                self.textarea.delete_next_char();
            }
            KeyStroke::Left => self.textarea.move_cursor(CursorMove::Back),
            KeyStroke::Right => self.textarea.move_cursor(CursorMove::Forward),
            KeyStroke::Up => self.textarea.move_cursor(CursorMove::Up),
            KeyStroke::Down => self.textarea.move_cursor(CursorMove::Down),
            KeyStroke::Home => self.textarea.move_cursor(CursorMove::Head),
            KeyStroke::End => self.textarea.move_cursor(CursorMove::End),
            KeyStroke::PageUp => self.textarea.move_cursor(CursorMove::Top),
            KeyStroke::PageDown => self.textarea.move_cursor(CursorMove::Bottom),
            KeyStroke::Enter | KeyStroke::Escape | KeyStroke::Ctrl(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Composer;
    use crate::entities::application::input_event::{InputEvent, KeyStroke};

    #[test]
    fn newline_chords_add_a_line_and_the_draft_keeps_it() {
        for chord in [
            KeyStroke::CtrlEnter,
            KeyStroke::CmdEnter,
            KeyStroke::ShiftEnter,
        ] {
            let mut composer = Composer::new();
            composer.apply(&InputEvent::Text("first".to_string()));
            composer.apply(&InputEvent::Key(chord));
            composer.apply(&InputEvent::Text("second".to_string()));

            assert_eq!(composer.text(), "first\nsecond");
            assert_eq!(composer.line_count(), 2);
        }
    }

    #[test]
    fn plain_enter_is_not_inserted_by_the_editor() {
        let mut composer = Composer::new();
        composer.apply(&InputEvent::Text("first".to_string()));
        composer.apply(&InputEvent::Key(KeyStroke::Enter));

        assert_eq!(composer.text(), "first");
    }

    #[test]
    fn backspace_and_delete_edit_at_the_cursor() {
        let mut composer = Composer::new();
        composer.apply(&InputEvent::Text("ab".to_string()));
        composer.apply(&InputEvent::Key(KeyStroke::Left));
        composer.apply(&InputEvent::Key(KeyStroke::Backspace));

        assert_eq!(composer.text(), "b");
        composer.apply(&InputEvent::Key(KeyStroke::Delete));
        assert!(composer.is_empty());
    }

    #[test]
    fn pasted_text_is_inserted_verbatim() {
        let mut composer = Composer::new();
        composer.apply(&InputEvent::Paste("/note".to_string()));

        assert_eq!(composer.text(), "/note");
    }

    #[test]
    fn clear_removes_all_lines() {
        let mut composer = Composer::new();
        composer.apply(&InputEvent::Text("text".to_string()));
        composer.clear();

        assert!(composer.is_empty());
        assert_eq!(composer.line_count(), 1);
    }
}
