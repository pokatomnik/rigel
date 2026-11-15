use ratatui::style::Color;

use crate::entities::theme::{palette::Palette, rgb::Rgb, token::ColorToken};

/// Converts one explicit RGB value into a Ratatui color without terminal defaults.
pub(crate) fn color(rgb: Rgb) -> Color {
    Color::Rgb(rgb.red, rgb.green, rgb.blue)
}

/// Resolves a semantic token through the palette for a Ratatui widget.
pub(crate) fn token(palette: &Palette, token: ColorToken) -> Color {
    color(palette.color(token))
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::{color, token};
    use crate::entities::theme::{palette::Palette, rgb::Rgb, token::ColorToken};

    #[test]
    fn rgb_conversion_is_explicit_and_lossless() {
        assert_eq!(
            color(Rgb::new(0x10, 0x15, 0x1C)),
            Color::Rgb(0x10, 0x15, 0x1C)
        );
    }

    #[test]
    fn semantic_tokens_resolve_through_the_palette() {
        assert_eq!(
            token(&Palette::rigel_night(), ColorToken::Accent),
            Color::Rgb(0x7C, 0xC7, 0xFF)
        );
    }
}
