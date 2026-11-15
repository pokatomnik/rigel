use futures::future::BoxFuture;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::{
    entities::application::input_event::{InputEvent, KeyStroke, TerminalSize},
    infrastructure::ui::input::InputSource,
};

/// Crossterm-backed input stream owned by the UI loop.
pub(crate) struct CrosstermInput {
    receiver: tokio::sync::Mutex<UnboundedReceiver<InputEvent>>,
}

impl CrosstermInput {
    /// Starts one blocking reader thread and converts crossterm events.
    pub(crate) fn start() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || read_loop(sender));
        Self {
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
}

impl InputSource for CrosstermInput {
    fn next_event(&self) -> BoxFuture<'_, anyhow::Result<Option<InputEvent>>> {
        Box::pin(async move { Ok(self.receiver.lock().await.recv().await) })
    }
}

fn read_loop(sender: UnboundedSender<InputEvent>) {
    loop {
        match ratatui::crossterm::event::read() {
            Ok(event) => {
                if let Some(event) = map_event(event)
                    && sender.send(event).is_err()
                {
                    return;
                }
            }
            Err(_) => {
                let _ = sender.send(InputEvent::Closed);
                return;
            }
        }
    }
}

fn map_event(event: Event) -> Option<InputEvent> {
    match event {
        Event::Key(key) => map_key(key),
        Event::Paste(text) => Some(InputEvent::Paste(text)),
        Event::Resize(width, height) => Some(InputEvent::Resize(TerminalSize::new(width, height))),
        Event::FocusGained | Event::FocusLost | Event::Mouse(_) => None,
    }
}

fn map_key(key: KeyEvent) -> Option<InputEvent> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    match key.code {
        KeyCode::Enter => Some(InputEvent::Key(enter_chord(key.modifiers))),
        KeyCode::Tab | KeyCode::BackTab => Some(InputEvent::Key(KeyStroke::Tab)),
        KeyCode::Esc => Some(InputEvent::Key(KeyStroke::Escape)),
        KeyCode::Backspace => Some(InputEvent::Key(KeyStroke::Backspace)),
        KeyCode::Delete => Some(InputEvent::Key(KeyStroke::Delete)),
        KeyCode::Left => Some(InputEvent::Key(KeyStroke::Left)),
        KeyCode::Right => Some(InputEvent::Key(KeyStroke::Right)),
        KeyCode::Up => Some(InputEvent::Key(KeyStroke::Up)),
        KeyCode::Down => Some(InputEvent::Key(KeyStroke::Down)),
        KeyCode::Home => Some(InputEvent::Key(KeyStroke::Home)),
        KeyCode::End => Some(InputEvent::Key(KeyStroke::End)),
        KeyCode::PageUp => Some(InputEvent::Key(KeyStroke::PageUp)),
        KeyCode::PageDown => Some(InputEvent::Key(KeyStroke::PageDown)),
        KeyCode::Char(character) => map_character(character, key.modifiers),
        KeyCode::Insert
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_)
        | KeyCode::F(_) => None,
    }
}

fn map_character(character: char, modifiers: KeyModifiers) -> Option<InputEvent> {
    if modifiers.contains(KeyModifiers::CONTROL) {
        return Some(InputEvent::Key(KeyStroke::Ctrl(
            character.to_ascii_lowercase(),
        )));
    }
    if modifiers.contains(KeyModifiers::ALT) || modifiers.contains(KeyModifiers::SUPER) {
        return None;
    }
    Some(InputEvent::Text(character.to_string()))
}

fn enter_chord(modifiers: KeyModifiers) -> KeyStroke {
    if modifiers.contains(KeyModifiers::SUPER) {
        return KeyStroke::CmdEnter;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        return KeyStroke::CtrlEnter;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        return platform_newline();
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        return KeyStroke::ShiftEnter;
    }
    KeyStroke::Enter
}

#[cfg(target_os = "macos")]
const fn platform_newline() -> KeyStroke {
    KeyStroke::CmdEnter
}

#[cfg(not(target_os = "macos"))]
const fn platform_newline() -> KeyStroke {
    KeyStroke::CtrlEnter
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::{map_event, map_key};
    use crate::entities::application::input_event::{InputEvent, KeyStroke};

    #[test]
    fn plain_enter_is_the_submit_chord() {
        let event = map_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(event, Some(InputEvent::Key(KeyStroke::Enter)));
    }

    #[test]
    fn the_platform_newline_chord_is_selected_at_compile_time() {
        let modifiers = if cfg!(target_os = "macos") {
            KeyModifiers::SUPER
        } else {
            KeyModifiers::CONTROL
        };
        let event = map_key(KeyEvent::new(KeyCode::Enter, modifiers));

        assert_eq!(event, Some(InputEvent::Key(super::platform_newline())));
    }

    #[test]
    fn both_control_and_super_enter_are_newline_chords() {
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            Some(InputEvent::Key(KeyStroke::CtrlEnter))
        );
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SUPER)),
            Some(InputEvent::Key(KeyStroke::CmdEnter))
        );
    }

    #[test]
    fn alt_enter_uses_the_platform_newline_chord() {
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            Some(InputEvent::Key(super::platform_newline()))
        );
    }

    #[test]
    fn shift_enter_is_a_newline_chord() {
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(InputEvent::Key(KeyStroke::ShiftEnter))
        );
    }

    #[test]
    fn paste_is_not_a_key_chord() {
        let event = map_event(Event::Paste("/".to_string()));

        assert_eq!(event, Some(InputEvent::Paste("/".to_string())));
    }

    #[test]
    fn control_characters_become_lowercase_chords() {
        let event = map_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::CONTROL));

        assert_eq!(event, Some(InputEvent::Key(KeyStroke::Ctrl('k'))));
    }

    #[test]
    fn printable_characters_become_text_without_modifiers() {
        let event = map_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));

        assert_eq!(event, Some(InputEvent::Text("a".to_string())));
    }

    #[test]
    fn alt_and_super_characters_are_ignored() {
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SUPER)),
            None
        );
    }
}
