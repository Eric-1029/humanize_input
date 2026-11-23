//! Two-language strings. English eyebrows are literal translations of the
//! Chinese titles; in English mode the title line is English too.

use humanize_desktop::config::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tr(pub Language);

impl Tr {
    pub fn zh(self) -> bool {
        self.0 == Language::Chinese
    }

    /// Picks the string for the current language.
    pub fn t(self, zh: &str, en: &str) -> String {
        if self.zh() { zh } else { en }.to_owned()
    }
}

/// `01:23` (or `1:02:03` past an hour).
pub fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// Thousands separators: `12,464`.
pub fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(clock(83), "01:23");
        assert_eq!(clock(3723), "1:02:03");
        assert_eq!(count(12464), "12,464");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000_000), "1,000,000");
    }
}
