#[cfg(test)]
use super::rgb::contrast_ratio;
use super::{rgb::Rgb, token::ColorToken};

/// Immutable semantic palette owned by one UI theme.
///
/// Every color is explicit so the rendered frame never inherits a terminal default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Palette {
    background: Rgb,
    surface: Rgb,
    selection: Rgb,
    text: Rgb,
    muted: Rgb,
    accent: Rgb,
    success: Rgb,
    warning: Rgb,
    error: Rgb,
    border: Rgb,
}

impl Palette {
    /// Creates the fixed Rigel Night palette accepted for this migration.
    pub(crate) fn rigel_night() -> Self {
        Self {
            background: Rgb::new(0x10, 0x15, 0x1C),
            surface: Rgb::new(0x18, 0x21, 0x2B),
            selection: Rgb::new(0x27, 0x34, 0x45),
            text: Rgb::new(0xE6, 0xED, 0xF3),
            muted: Rgb::new(0xA1, 0xB0, 0xC1),
            accent: Rgb::new(0x7C, 0xC7, 0xFF),
            success: Rgb::new(0x85, 0xD6, 0xA5),
            warning: Rgb::new(0xE7, 0xC0, 0x78),
            error: Rgb::new(0xF2, 0x8B, 0x82),
            border: Rgb::new(0x70, 0x81, 0x97),
        }
    }

    /// Returns the explicit color assigned to a semantic token.
    pub(crate) fn color(&self, token: ColorToken) -> Rgb {
        match token {
            ColorToken::Background => self.background,
            ColorToken::Surface => self.surface,
            ColorToken::Selection => self.selection,
            ColorToken::Text => self.text,
            ColorToken::Muted => self.muted,
            ColorToken::Accent => self.accent,
            ColorToken::Success => self.success,
            ColorToken::Warning => self.warning,
            ColorToken::Error => self.error,
            ColorToken::Border => self.border,
        }
    }

    /// Returns the WCAG contrast ratio between two semantic tokens.
    #[cfg(test)]
    pub(crate) fn contrast(&self, foreground: ColorToken, background: ColorToken) -> f64 {
        contrast_ratio(self.color(foreground), self.color(background))
    }
}

#[cfg(test)]
mod tests {
    use super::Palette;
    use crate::entities::theme::token::ColorToken;

    fn documented_colors() -> [(ColorToken, &'static str); 10] {
        [
            (ColorToken::Background, "#10151C"),
            (ColorToken::Surface, "#18212B"),
            (ColorToken::Selection, "#273445"),
            (ColorToken::Text, "#E6EDF3"),
            (ColorToken::Muted, "#A1B0C1"),
            (ColorToken::Accent, "#7CC7FF"),
            (ColorToken::Success, "#85D6A5"),
            (ColorToken::Warning, "#E7C078"),
            (ColorToken::Error, "#F28B82"),
            (ColorToken::Border, "#708197"),
        ]
    }

    #[test]
    fn rigel_night_matches_the_documented_palette() {
        let palette = Palette::rigel_night();

        for (token, expected) in documented_colors() {
            assert_eq!(palette.color(token).to_hex(), expected, "{token:?}");
        }
    }

    #[test]
    fn readable_tokens_keep_sufficient_contrast() {
        let palette = Palette::rigel_night();
        let readable = [
            ColorToken::Text,
            ColorToken::Muted,
            ColorToken::Accent,
            ColorToken::Success,
            ColorToken::Warning,
            ColorToken::Error,
        ];

        for token in readable {
            let ratio = palette.contrast(token, ColorToken::Background);
            assert!(ratio >= 4.5, "{token:?} contrast is only {ratio:.2}");
        }
    }

    #[test]
    fn border_is_readable_as_a_structural_line() {
        let palette = Palette::rigel_night();
        let ratio = palette.contrast(ColorToken::Border, ColorToken::Background);

        assert!(ratio >= 3.0, "border contrast is only {ratio:.2}");
    }

    #[test]
    fn surfaces_are_visually_distinct_from_the_background() {
        let palette = Palette::rigel_night();

        assert_ne!(
            palette.color(ColorToken::Surface),
            palette.color(ColorToken::Background)
        );
        assert_ne!(
            palette.color(ColorToken::Selection),
            palette.color(ColorToken::Surface)
        );
    }
}
