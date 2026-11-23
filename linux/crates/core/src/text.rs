//! Input text preparation and validation.

/// Converts Windows (`\r\n`) and classic Mac (`\r`) line endings to `\n`.
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Whether the first release can type `c`: printable ASCII, newline or tab.
pub fn is_supported_char(c: char) -> bool {
    matches!(c, ' '..='~' | '\n' | '\t')
}

/// A character the typist cannot produce, located in the normalized text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedChar {
    /// Position counted in characters (not bytes), matching GTK text iters.
    pub char_index: usize,
    pub value: char,
}

/// Lists every unsupported character in `text` after newline normalization.
///
/// `\r` is not reported because it is normalized to `\n` before typing.
pub fn find_unsupported(text: &str) -> Vec<UnsupportedChar> {
    normalize_newlines(text)
        .chars()
        .enumerate()
        .filter(|&(_, c)| !is_supported_char(c))
        .map(|(char_index, value)| UnsupportedChar { char_index, value })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_and_legacy_newlines() {
        assert_eq!(normalize_newlines("a\r\nb\rc\nd"), "a\nb\nc\nd");
    }

    #[test]
    fn ascii_text_has_no_unsupported_chars() {
        assert!(find_unsupported("Hello, world!\n\tfn main() { ~`|\\ }\r\n").is_empty());
    }

    #[test]
    fn locates_unsupported_chars_by_char_index() {
        let found = find_unsupported("a中b，\r\nc—");
        assert_eq!(
            found,
            vec![
                UnsupportedChar { char_index: 1, value: '中' },
                UnsupportedChar { char_index: 3, value: '，' },
                // "\r\n" collapses to one char, so 'c' is index 5 and '—' is 6.
                UnsupportedChar { char_index: 6, value: '—' },
            ]
        );
    }
}
