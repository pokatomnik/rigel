/// Decodes ordinary UTF-8 text and rejects binary/control-heavy content.
pub(crate) fn decode_text(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    is_supported_text(&text).then_some(text)
}

/// Reports whether text contains only supported printable and line-formatting characters.
pub(crate) fn is_supported_text(text: &str) -> bool {
    !text.chars().any(|character| {
        character.is_control() && !matches!(character, '\t' | '\n' | '\r' | '\u{c}')
    })
}

/// Finds a UTF-8-safe byte prefix that never exceeds the requested limit.
pub(crate) fn utf8_prefix(text: &str, max_bytes: usize) -> usize {
    text.char_indices()
        .take_while(|(index, character)| index.saturating_add(character.len_utf8()) <= max_bytes)
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8())
}
