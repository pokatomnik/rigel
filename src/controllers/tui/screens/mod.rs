pub mod chat;
pub mod fallback;
pub mod header;
pub mod modal;
pub mod palette;
pub mod status;
pub mod transcript;

use crate::entities::{render_model::model::RenderModel, theme::palette::Palette};

/// Immutable model and theme supplied to every screen for one frame.
pub(crate) struct Presentation<'a> {
    /// Presentation snapshot for this frame.
    pub(crate) model: &'a RenderModel,
    /// Semantic palette used by every widget.
    pub(crate) palette: &'a Palette,
}

/// Returns the first visible row so the selected item stays inside the viewport.
pub(super) fn visible_offset(len: usize, selected: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        return 0;
    }
    let selected = selected.min(len - 1);
    selected
        .saturating_sub(visible / 2)
        .min(len.saturating_sub(visible))
}

#[cfg(test)]
mod tests {
    use super::visible_offset;

    #[test]
    fn selection_stays_visible_when_the_list_exceeds_the_viewport() {
        assert_eq!(visible_offset(17, 0, 10), 0);
        assert_eq!(visible_offset(17, 9, 10), 4);
        assert_eq!(visible_offset(17, 16, 10), 7);
        assert_eq!(visible_offset(3, 2, 10), 0);
        assert_eq!(visible_offset(0, 0, 10), 0);
    }
}
