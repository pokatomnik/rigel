use serde::{Deserialize, Serialize};

const HEX_DIGITS_PER_CHANNEL: usize = 2;
const HEX_LENGTH: usize = 6;

/// An explicit 24-bit RGB color that never depends on terminal defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Rgb {
    /// Red channel in the inclusive range `0..=255`.
    pub(crate) red: u8,
    /// Green channel in the inclusive range `0..=255`.
    pub(crate) green: u8,
    /// Blue channel in the inclusive range `0..=255`.
    pub(crate) blue: u8,
}

impl Rgb {
    /// Creates a color from explicit channels without consulting the terminal.
    pub(crate) const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// Parses `#RRGGBB` or `RRGGBB` and rejects anything that is not six hex digits.
    pub(crate) fn from_hex(value: &str) -> anyhow::Result<Self> {
        let digits = value.strip_prefix('#').unwrap_or(value);
        if digits.len() != HEX_LENGTH || !digits.is_ascii() {
            anyhow::bail!("expected a six-digit ASCII hex color, got `{value}`");
        }
        Ok(Self::new(
            parse_channel(digits, 0)?,
            parse_channel(digits, 2)?,
            parse_channel(digits, 4)?,
        ))
    }

    /// Returns the color as an uppercase `#RRGGBB` string for diagnostics and tests.
    pub(crate) fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.red, self.green, self.blue)
    }

    /// Returns the WCAG 2.1 relative luminance in the inclusive range `0.0..=1.0`.
    pub(crate) fn relative_luminance(self) -> f64 {
        let red = linearize(self.red);
        let green = linearize(self.green);
        let blue = linearize(self.blue);
        0.2126 * red + 0.7152 * green + 0.0722 * blue
    }
}

/// Computes the WCAG contrast ratio between two explicit colors.
pub(crate) fn contrast_ratio(left: Rgb, right: Rgb) -> f64 {
    let left = left.relative_luminance();
    let right = right.relative_luminance();
    let (lighter, darker) = if left >= right {
        (left, right)
    } else {
        (right, left)
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn parse_channel(digits: &str, start: usize) -> anyhow::Result<u8> {
    let end = start + HEX_DIGITS_PER_CHANNEL;
    let channel = digits
        .get(start..end)
        .ok_or_else(|| anyhow::anyhow!("hex color `{digits}` is too short"))?;
    Ok(u8::from_str_radix(channel, 16)?)
}

fn linearize(channel: u8) -> f64 {
    let channel = f64::from(channel) / 255.0;
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::{Rgb, contrast_ratio};

    #[test]
    fn parses_both_hex_forms() -> anyhow::Result<()> {
        let with_prefix = Rgb::from_hex("#10151C")?;
        let without_prefix = Rgb::from_hex("10151c")?;

        assert_eq!(with_prefix, Rgb::new(0x10, 0x15, 0x1C));
        assert_eq!(with_prefix, without_prefix);
        Ok(())
    }

    #[test]
    fn rejects_malformed_hex_values() {
        for value in ["", "#", "#12345", "#1234567", "#GGGGGG", "#10151C "] {
            assert!(Rgb::from_hex(value).is_err(), "accepted `{value}`");
        }
    }

    #[test]
    fn formats_each_channel_as_two_uppercase_digits() {
        assert_eq!(Rgb::new(0x10, 0x15, 0x1C).to_hex(), "#10151C");
        assert_eq!(Rgb::new(0, 0, 0).to_hex(), "#000000");
        assert_eq!(Rgb::new(255, 255, 255).to_hex(), "#FFFFFF");
    }

    #[test]
    fn black_and_white_have_the_maximum_contrast() {
        let ratio = contrast_ratio(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255));

        assert!((ratio - 21.0).abs() < 0.01, "unexpected ratio {ratio}");
    }
}
