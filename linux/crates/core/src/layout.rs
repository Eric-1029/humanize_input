//! Keyboard layout: which key + modifiers produce which character, and where
//! each key physically sits.
//!
//! Phase 2 ships US QWERTY (ANSI). Phase 3 fills the same structure from the
//! live XKB keymap.

use std::collections::HashMap;

use rand::RngExt;

use crate::geometry::KeyboardGeometry;
use crate::keys::{Finger, KeyCode, Mods, Stroke};
use crate::text::{UnsupportedChar, is_supported_char, normalize_newlines};

/// One US key: (base char, shifted char, evdev code, finger).
type UsKey = (char, char, u16, Finger);

/// Key-level typos only hit directly adjacent keys (incl. diagonals).
const MAX_NEIGHBOUR_DISTANCE: f64 = 1.5;

/// Where a key sits (in key widths; row 0 is the number row) and which
/// finger presses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyInfo {
    pub x: f64,
    pub y: f64,
    pub finger: Finger,
}

#[derive(Debug, Clone)]
pub struct Layout {
    name: String,
    strokes: HashMap<char, Stroke>,
    chars: HashMap<(KeyCode, Mods), char>,
    keys: HashMap<KeyCode, KeyInfo>,
    /// Character keys near each character key, with pick weights.
    neighbours: HashMap<KeyCode, Vec<(KeyCode, f64)>>,
}

impl Layout {
    /// Builds a layout. When two strokes give the same character, the first
    /// one wins (it is the one the typist uses).
    pub fn new(
        name: impl Into<String>,
        chars: impl IntoIterator<Item = (char, Stroke)>,
        keys: impl IntoIterator<Item = (KeyCode, KeyInfo)>,
    ) -> Self {
        let keys: HashMap<KeyCode, KeyInfo> = keys.into_iter().collect();
        let mut strokes = HashMap::new();
        let mut by_stroke = HashMap::new();
        for (c, stroke) in chars {
            strokes.entry(c).or_insert(stroke);
            by_stroke.entry((stroke.key, stroke.mods.level())).or_insert(c);
        }

        let mut char_keys: Vec<KeyCode> = keys
            .keys()
            .copied()
            .filter(|k| by_stroke.get(&(*k, Mods::NONE)).is_some_and(|c| !c.is_whitespace()))
            .collect();
        char_keys.sort_unstable();
        let neighbours = char_keys
            .iter()
            .map(|&key| {
                let a = keys[&key];
                let list = char_keys
                    .iter()
                    .filter(|&&other| other != key)
                    .filter_map(|&other| {
                        let b = keys[&other];
                        let distance = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
                        (distance <= MAX_NEIGHBOUR_DISTANCE)
                            .then(|| (other, 1.0 / (1.0 + distance * distance * 2.2)))
                    })
                    .collect();
                (key, list)
            })
            .collect();

        Self { name: name.into(), strokes, chars: by_stroke, keys, neighbours }
    }

    /// US QWERTY on an ANSI keyboard, touch-typing finger assignment.
    pub fn us_qwerty() -> Self {
        use Finger::*;
        const S: Mods = Mods::SHIFT;
        const N: Mods = Mods::NONE;

        // (base, shifted, evdev code, finger) per row.
        let rows: [(f64, &[UsKey]); 4] = [
            (
                0.0,
                &[
                    ('`', '~', 41, LeftPinky),
                    ('1', '!', 2, LeftPinky),
                    ('2', '@', 3, LeftRing),
                    ('3', '#', 4, LeftMiddle),
                    ('4', '$', 5, LeftIndex),
                    ('5', '%', 6, LeftIndex),
                    ('6', '^', 7, RightIndex),
                    ('7', '&', 8, RightIndex),
                    ('8', '*', 9, RightMiddle),
                    ('9', '(', 10, RightRing),
                    ('0', ')', 11, RightPinky),
                    ('-', '_', 12, RightPinky),
                    ('=', '+', 13, RightPinky),
                ],
            ),
            (
                1.5,
                &[
                    ('q', 'Q', 16, LeftPinky),
                    ('w', 'W', 17, LeftRing),
                    ('e', 'E', 18, LeftMiddle),
                    ('r', 'R', 19, LeftIndex),
                    ('t', 'T', 20, LeftIndex),
                    ('y', 'Y', 21, RightIndex),
                    ('u', 'U', 22, RightIndex),
                    ('i', 'I', 23, RightMiddle),
                    ('o', 'O', 24, RightRing),
                    ('p', 'P', 25, RightPinky),
                    ('[', '{', 26, RightPinky),
                    (']', '}', 27, RightPinky),
                    ('\\', '|', 43, RightPinky),
                ],
            ),
            (
                1.75,
                &[
                    ('a', 'A', 30, LeftPinky),
                    ('s', 'S', 31, LeftRing),
                    ('d', 'D', 32, LeftMiddle),
                    ('f', 'F', 33, LeftIndex),
                    ('g', 'G', 34, LeftIndex),
                    ('h', 'H', 35, RightIndex),
                    ('j', 'J', 36, RightIndex),
                    ('k', 'K', 37, RightMiddle),
                    ('l', 'L', 38, RightRing),
                    (';', ':', 39, RightPinky),
                    ('\'', '"', 40, RightPinky),
                ],
            ),
            (
                2.25,
                &[
                    ('z', 'Z', 44, LeftPinky),
                    ('x', 'X', 45, LeftRing),
                    ('c', 'C', 46, LeftMiddle),
                    ('v', 'V', 47, LeftIndex),
                    ('b', 'B', 48, LeftIndex),
                    ('n', 'N', 49, RightIndex),
                    ('m', 'M', 50, RightIndex),
                    (',', '<', 51, RightMiddle),
                    ('.', '>', 52, RightRing),
                    ('/', '?', 53, RightPinky),
                ],
            ),
        ];

        let mut chars = Vec::new();
        let mut keys = Vec::new();
        for (y, (x0, row)) in rows.iter().enumerate() {
            for (i, &(base, shifted, code, finger)) in row.iter().enumerate() {
                let key = KeyCode(code);
                chars.push((base, Stroke::new(key, N)));
                chars.push((shifted, Stroke::new(key, S)));
                keys.push((key, KeyInfo { x: x0 + i as f64, y: y as f64, finger }));
            }
        }
        chars.push((' ', Stroke::new(KeyCode::SPACE, N)));
        chars.push(('\n', Stroke::new(KeyCode::ENTER, N)));
        chars.push(('\t', Stroke::new(KeyCode::TAB, N)));

        let special = [
            (KeyCode::SPACE, 6.5, 4.0, Thumb),
            (KeyCode::TAB, 0.5, 1.0, LeftPinky),
            (KeyCode::ENTER, 13.5, 2.0, RightPinky),
            (KeyCode::BACKSPACE, 13.5, 0.0, RightPinky),
            (KeyCode::LEFT_SHIFT, 0.5, 3.0, LeftPinky),
            (KeyCode::RIGHT_SHIFT, 13.0, 3.0, RightPinky),
            (KeyCode::LEFT_CTRL, 0.5, 4.0, LeftPinky),
            (KeyCode::RIGHT_ALT, 9.0, 4.0, Thumb),
        ];
        keys.extend(special.map(|(key, x, y, finger)| (key, KeyInfo { x, y, finger })));

        Self::new("us", chars, keys)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// How to type `c`, if this layout can.
    pub fn stroke(&self, c: char) -> Option<Stroke> {
        self.strokes.get(&c).copied()
    }

    /// The character `key` produces with `mods` held (Ctrl is ignored).
    pub fn char_for(&self, key: KeyCode, mods: Mods) -> Option<char> {
        self.chars.get(&(key, mods.level())).copied()
    }

    pub fn key_info(&self, key: KeyCode) -> Option<&KeyInfo> {
        self.keys.get(&key)
    }

    /// Picks a weighted adjacent character key that passes `accept`.
    pub fn neighbour_key<R: rand::Rng + ?Sized>(
        &self,
        key: KeyCode,
        rng: &mut R,
        accept: impl Fn(KeyCode) -> bool,
    ) -> Option<KeyCode> {
        let candidates: Vec<(KeyCode, f64)> =
            self.neighbours.get(&key)?.iter().copied().filter(|&(k, _)| accept(k)).collect();
        let total: f64 = candidates.iter().map(|&(_, w)| w).sum();
        if candidates.is_empty() || total <= 0.0 {
            return None;
        }
        let mut pick = rng.random::<f64>() * total;
        for &(k, w) in &candidates {
            pick -= w;
            if pick <= 0.0 {
                return Some(k);
            }
        }
        candidates.last().map(|&(k, _)| k)
    }

    /// Characters this release cannot type with this layout: anything
    /// outside printable ASCII / newline / tab, or missing from the layout.
    pub fn find_unsupported(&self, text: &str) -> Vec<UnsupportedChar> {
        normalize_newlines(text)
            .chars()
            .enumerate()
            .filter(|&(_, c)| !is_supported_char(c) || self.stroke(c).is_none())
            .map(|(char_index, value)| UnsupportedChar { char_index, value })
            .collect()
    }

    /// Character-level neighbour table for the classic (original) model.
    ///
    /// For US QWERTY this is the original hard-coded table so classic mode
    /// stays byte-for-byte faithful; other layouts derive it from key
    /// positions.
    pub fn classic_geometry(&self) -> KeyboardGeometry {
        if self.name == "us" {
            return KeyboardGeometry::us_qwerty();
        }
        let mut positions: Vec<(char, f64, f64)> = Vec::new();
        let mut shifted = Vec::new();
        let mut entries: Vec<(&(KeyCode, Mods), &char)> = self.chars.iter().collect();
        entries.sort_by_key(|((k, m), _)| (*k, m.contains(Mods::SHIFT), m.contains(Mods::ALTGR)));
        for (&(key, mods), &c) in entries {
            let Some(info) = self.keys.get(&key) else { continue };
            if c.is_whitespace() {
                continue;
            }
            if mods.is_empty() {
                positions.push((c, info.x, info.y));
            } else if let Some(base) = self.char_for(key, Mods::NONE) {
                shifted.push((c, base));
            }
        }
        KeyboardGeometry::from_positions(positions, shifted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn us_covers_all_printable_ascii_and_round_trips() {
        let layout = Layout::us_qwerty();
        for c in (' '..='~').chain(['\n', '\t']) {
            let stroke = layout.stroke(c).unwrap_or_else(|| panic!("missing {c:?}"));
            assert_eq!(layout.char_for(stroke.key, stroke.mods), Some(c));
        }
        assert!(layout.find_unsupported("Hello, world!\n").is_empty());
        assert_eq!(layout.find_unsupported("aé").len(), 1);
    }

    #[test]
    fn uppercase_needs_shift_and_ctrl_is_ignored_for_chars() {
        let layout = Layout::us_qwerty();
        assert_eq!(layout.stroke('A'), Some(Stroke::new(KeyCode(30), Mods::SHIFT)));
        assert_eq!(layout.char_for(KeyCode(30), Mods::CTRL), Some('a'));
    }

    #[test]
    fn neighbours_are_adjacent_keys() {
        let layout = Layout::us_qwerty();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let g = layout.stroke('g').unwrap().key;
        for _ in 0..500 {
            let n = layout.neighbour_key(g, &mut rng, |_| true).unwrap();
            let c = layout.char_for(n, Mods::NONE).unwrap();
            assert!("tyfhvbr".contains(c) || c == 'u' || c == 'n', "unexpected neighbour {c}");
        }
        // The filter is honoured.
        let only_h = layout.neighbour_key(g, &mut rng, |k| layout.char_for(k, Mods::NONE) == Some('h'));
        assert_eq!(only_h.and_then(|k| layout.char_for(k, Mods::NONE)), Some('h'));
    }

    #[test]
    fn classic_geometry_for_us_is_the_original_table() {
        let layout = Layout::us_qwerty();
        let mut a = ChaCha8Rng::seed_from_u64(4);
        let mut b = ChaCha8Rng::seed_from_u64(4);
        let original = KeyboardGeometry::us_qwerty();
        let derived = layout.classic_geometry();
        for c in "the quick brown fox".chars() {
            assert_eq!(original.wrong_char(c, &mut a), derived.wrong_char(c, &mut b));
        }
    }
}
