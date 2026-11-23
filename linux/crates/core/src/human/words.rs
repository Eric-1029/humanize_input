//! Small English frequency tables used for rhythm.

/// The most common English words; typists fire these off as one motion.
const FAMILIAR_WORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "back", "be", "because",
    "been", "but", "by", "can", "come", "could", "day", "do", "even", "first", "for", "from", "get", "give",
    "go", "good", "had", "has", "have", "he", "her", "him", "his", "how", "i", "if", "in", "into", "is",
    "it", "its", "just", "know", "like", "look", "make", "me", "most", "my", "new", "no", "not", "now", "of",
    "on", "one", "only", "or", "other", "our", "out", "over", "people", "say", "see", "she", "so", "some",
    "take", "than", "that", "the", "their", "them", "then", "there", "these", "they", "think", "this",
    "time", "to", "two", "up", "us", "use", "want", "was", "way", "we", "well", "were", "what", "when",
    "which", "who", "will", "with", "work", "would", "year", "you", "your",
];

/// Frequent letter pairs, typed faster than their finger geometry suggests.
const COMMON_BIGRAMS: &[&str] = &[
    "th", "he", "in", "er", "an", "re", "on", "at", "en", "nd", "ti", "es", "or", "te", "of", "ed", "is",
    "it", "al", "ar", "st", "to", "nt", "ng", "se", "ha", "as", "ou", "io", "le", "ve", "co", "me", "de",
    "hi", "ri", "ro", "ic", "ne", "ea", "ra", "ce",
];

pub(crate) fn is_familiar(word: &[char]) -> bool {
    word.len() <= 7
        && FAMILIAR_WORDS
            .iter()
            .any(|w| w.len() == word.len() && w.chars().zip(word).all(|(a, b)| a == b.to_ascii_lowercase()))
}

pub(crate) fn is_common_bigram(a: char, b: char) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    COMMON_BIGRAMS.iter().any(|pair| {
        let mut chars = pair.chars();
        chars.next() == Some(a) && chars.next() == Some(b)
    })
}

/// Letters and digits form words; everything else separates them.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups_ignore_case() {
        let the: Vec<char> = "The".chars().collect();
        let theory: Vec<char> = "theory".chars().collect();
        assert!(is_familiar(&the));
        assert!(!is_familiar(&theory));
        assert!(is_common_bigram('T', 'h'));
        assert!(!is_common_bigram('q', 'z'));
    }
}
