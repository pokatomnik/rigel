use crate::entities::application::input_event::TerminalSize;

/// Minimum terminal width that can display the normal UI.
pub(crate) const MIN_WIDTH: u16 = 60;
/// Minimum terminal height that can display the normal UI.
pub(crate) const MIN_HEIGHT: u16 = 20;

/// Terminal size together with the small-screen fallback decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayoutView {
    /// Current terminal size.
    pub(crate) size: TerminalSize,
    /// Whether the normal UI must be replaced by the English warning.
    pub(crate) fallback: bool,
}

impl LayoutView {
    /// Creates a layout decision from the current terminal size.
    pub(crate) fn new(size: TerminalSize) -> Self {
        Self {
            size,
            fallback: size.width < MIN_WIDTH || size.height < MIN_HEIGHT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutView, MIN_HEIGHT, MIN_WIDTH};
    use crate::entities::application::input_event::TerminalSize;

    #[test]
    fn minimum_size_boundaries_enable_and_disable_the_fallback() {
        assert!(!LayoutView::new(TerminalSize::new(MIN_WIDTH, MIN_HEIGHT)).fallback);
        assert!(LayoutView::new(TerminalSize::new(MIN_WIDTH - 1, MIN_HEIGHT)).fallback);
        assert!(LayoutView::new(TerminalSize::new(MIN_WIDTH, MIN_HEIGHT - 1)).fallback);
    }
}
