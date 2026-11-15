use std::{
    io::{self, Write},
    sync::{Mutex, OnceLock},
};

use crate::{
    entities::agent_event::{
        event::AgentEvent,
        status::{Notice, NoticeLevel},
    },
    infrastructure::{interaction::ChatIO, ui::gateway::EventSink},
};

static TUI_SINK: OnceLock<Mutex<Option<std::sync::Arc<dyn EventSink>>>> = OnceLock::new();

fn tui_sink() -> Option<std::sync::Arc<dyn EventSink>> {
    TUI_SINK
        .get()
        .and_then(|slot| slot.lock().ok())
        .and_then(|slot| slot.clone())
}

#[derive(Default)]
pub(crate) struct TerminalIO;

impl TerminalIO {
    /// Routes all terminal writes to the event sink while the alternate screen is active.
    pub(crate) fn install_tui_sink(sink: std::sync::Arc<dyn EventSink>) {
        let slot = TUI_SINK.get_or_init(|| Mutex::new(None));
        if let Ok(mut slot) = slot.lock() {
            *slot = Some(sink);
        }
    }

    /// Restores direct terminal writes after the alternate screen is left.
    pub(crate) fn uninstall_tui_sink() {
        let slot = TUI_SINK.get_or_init(|| Mutex::new(None));
        if let Ok(mut slot) = slot.lock() {
            *slot = None;
        }
    }

    fn tui_active() -> bool {
        tui_sink().is_some()
    }

    fn emit_tui(level: NoticeLevel, message: &str) -> bool {
        let Some(sink) = tui_sink() else {
            return false;
        };
        sink.emit(AgentEvent::Notice(Notice::new(level, message)));
        true
    }
}

impl ChatIO for TerminalIO {
    fn eprintln_red(&self, message: &str) {
        TerminalIO::eprintln_red(self, message);
    }
}

impl TerminalIO {
    pub fn print(&self, msg: &str) {
        if !Self::tui_active() {
            print!("{msg}")
        }
    }

    pub fn eprintln(&self, msg: &str) {
        if !Self::emit_tui(NoticeLevel::Info, msg) {
            eprintln!("{msg}");
        }
    }

    pub fn eprintln_gray(&self, msg: &str) {
        if Self::emit_tui(NoticeLevel::Info, msg) {
            return;
        }
        let style = console::Style::new().black().bright().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn eprintln_red(&self, msg: &str) {
        if Self::emit_tui(NoticeLevel::Error, msg) {
            return;
        }
        let style = console::Style::new().red().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn eprint_gray(&self, msg: &str) {
        if Self::tui_active() {
            return;
        }
        let style = console::Style::new().black().bright().for_stderr();
        eprint!("{}", style.apply_to(msg));
    }

    pub fn eprintln_orange(&self, msg: &str) {
        if Self::emit_tui(NoticeLevel::Warning, msg) {
            return;
        }
        let style = console::Style::new().yellow().bright().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn eprintln_blue(&self, msg: &str) {
        if Self::emit_tui(NoticeLevel::Info, msg) {
            return;
        }
        let style = console::Style::new().cyan().for_stderr();
        eprintln!("{}", style.apply_to(msg));
    }

    pub fn flush_stderr(&self) {
        let _ = io::stderr().flush();
    }

    pub fn flush_stdout(&self) {
        let _ = io::stdout().flush();
    }
}
