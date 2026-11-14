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
