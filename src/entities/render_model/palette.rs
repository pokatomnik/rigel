use crate::entities::palette::{item::PaletteItem, level::PaletteLevel};

/// Command palette overlay with its current level and selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaletteView {
    /// Level currently rendered.
    pub(crate) level: PaletteLevel,
    /// Items available on this level.
    pub(crate) items: Vec<PaletteItem>,
    /// Zero-based index of the highlighted item.
    pub(crate) selected: usize,
    /// Optional status text shown under the list.
    pub(crate) status: Option<String>,
}

impl PaletteView {
    /// Creates a palette view from the items of one level.
    pub(crate) fn new(level: PaletteLevel, items: Vec<PaletteItem>) -> Self {
        Self {
            level,
            items,
            selected: 0,
            status: None,
        }
    }

    /// Attaches a status line without changing the selection.
    pub(crate) fn with_status(mut self, status: Option<String>) -> Self {
        self.status = status;
        self
    }

    /// Sets the highlighted item, clamped to the available list.
    pub(crate) fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected.min(self.items.len().saturating_sub(1));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::PaletteView;
    use crate::entities::palette::{
        item::{PaletteAction, PaletteItem},
        level::PaletteLevel,
    };

    #[test]
    fn palette_starts_on_the_first_item_and_keeps_its_level() {
        let view = PaletteView::new(
            PaletteLevel::Models,
            vec![PaletteItem::new("back", "← Back", PaletteAction::Back)],
        );

        assert_eq!(view.level, PaletteLevel::Models);
        assert_eq!(view.selected, 0);
        assert_eq!(view.items.len(), 1);
    }
}
