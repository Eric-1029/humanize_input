//! Physical key positions, used to pick believable wrong keys.
//!
//! Phase 1 ships the original hard-coded US QWERTY table. Phase 3 builds the
//! same structure from the live XKB layout via [`KeyboardGeometry::from_rows`].

use std::collections::HashMap;

use rand::RngExt;

/// Neighbours further than this (in key widths) are never chosen.
const MAX_NEIGHBOUR_DISTANCE: f64 = 2.4;

/// Key layout: which base characters sit where, and which shifted characters
/// share a key with them.
#[derive(Debug, Clone)]
pub struct KeyboardGeometry {
    /// For each base character, the candidate wrong keys and their weights.
    neighbours: HashMap<char, Vec<(char, f64)>>,
    /// Shifted symbol → base character on the same key (`'!'` → `'1'`).
    shifted_to_base: HashMap<char, char>,
}

impl KeyboardGeometry {
    /// The original table: US QWERTY rows with ANSI stagger.
    pub fn us_qwerty() -> Self {
        const ROWS: [&str; 4] = ["`1234567890-=", "qwertyuiop[]\\", "asdfghjkl;'", "zxcvbnm,./"];
        const OFFSETS: [f64; 4] = [0.0, 0.5, 1.0, 1.5];
        const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";
        const BASE: &str = "`1234567890-=[]\\;',./";

        let rows = ROWS.iter().zip(OFFSETS).map(|(row, offset)| (*row, offset));
        let shifted = SHIFTED.chars().zip(BASE.chars());
        Self::from_rows(rows, shifted)
    }

    /// Builds a geometry from rows of base characters (each with its
    /// horizontal stagger in key widths) and shifted → base pairs.
    pub fn from_rows<'a>(
        rows: impl IntoIterator<Item = (&'a str, f64)>,
        shifted_to_base: impl IntoIterator<Item = (char, char)>,
    ) -> Self {
        let mut positions: Vec<(char, f64, f64)> = Vec::new();
        for (row_index, (row, offset)) in rows.into_iter().enumerate() {
            for (col, key) in row.chars().enumerate() {
                if !positions.iter().any(|&(k, _, _)| k == key) {
                    positions.push((key, col as f64 + offset, row_index as f64));
                }
            }
        }

        let neighbours = positions
            .iter()
            .map(|&(key, x, y)| {
                let candidates = positions
                    .iter()
                    .filter(|&&(other, _, _)| other != key)
                    // A letter or symbol is never mistyped as a digit.
                    .filter(|&&(other, _, _)| key.is_ascii_digit() || !other.is_ascii_digit())
                    .filter_map(|&(other, ox, oy)| {
                        let distance = ((ox - x).powi(2) + (oy - y).powi(2)).sqrt();
                        (0.0001..=MAX_NEIGHBOUR_DISTANCE)
                            .contains(&distance)
                            // Closer neighbours are chosen much more often.
                            .then(|| (other, 1.0 / (1.0 + distance * distance * 2.2)))
                    })
                    .collect();
                (key, candidates)
            })
            .collect();

        Self { neighbours, shifted_to_base: shifted_to_base.into_iter().collect() }
    }

    /// Picks a weighted neighbour of `current`, keeping letter case.
    ///
    /// Shifted symbols resolve to their base key and yield an unshifted
    /// neighbour, as in the original. Returns `None` for unknown characters.
    pub fn pick_neighbour<R: rand::Rng + ?Sized>(&self, current: char, rng: &mut R) -> Option<char> {
        let (lookup, is_upper) = match self.shifted_to_base.get(&current) {
            Some(&base) => (base, false),
            None => (current.to_ascii_lowercase(), current.is_ascii_uppercase()),
        };

        let candidates = self.neighbours.get(&lookup).filter(|c| !c.is_empty())?;
        let total: f64 = candidates.iter().map(|&(_, w)| w).sum();
        let mut pick = rng.random::<f64>() * total;
        let mut selected = candidates[candidates.len() - 1].0;
        for &(key, weight) in candidates {
            pick -= weight;
            if pick <= 0.0 {
                selected = key;
                break;
            }
        }

        Some(if is_upper { selected.to_ascii_uppercase() } else { selected })
    }

    /// The wrong character typed for a typo on `current`: a neighbour when
    /// one exists, otherwise a random letter of the same case (`x`/`z` for
    /// non-letters).
    pub fn wrong_char<R: rand::Rng + ?Sized>(&self, current: char, rng: &mut R) -> char {
        if let Some(neighbour) = self.pick_neighbour(current, rng) {
            return neighbour;
        }
        if !current.is_alphabetic() {
            return if rng.random_bool(0.5) { 'x' } else { 'z' };
        }
        let base = if current.is_lowercase() { b'a' } else { b'A' };
        char::from(base + rng.random_range(0..26u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn neighbour_set(geometry: &KeyboardGeometry, c: char) -> Vec<char> {
        let mut keys: Vec<char> = geometry.neighbours[&c].iter().map(|&(k, _)| k).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn letters_never_get_digit_neighbours() {
        let geometry = KeyboardGeometry::us_qwerty();
        // 'q' sits right under '1' and '2'.
        assert!(neighbour_set(&geometry, 'q').iter().all(|c| !c.is_ascii_digit()));
        assert!(neighbour_set(&geometry, 'q').contains(&'w'));
    }

    #[test]
    fn closest_key_wins_most_often() {
        let geometry = KeyboardGeometry::us_qwerty();
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        let mut counts: HashMap<char, u32> = HashMap::new();
        for _ in 0..5000 {
            *counts.entry(geometry.pick_neighbour('g', &mut rng).unwrap()).or_default() += 1;
        }
        // Same-row neighbours (distance 1) beat the far diagonal ones.
        assert!(counts[&'f'] > counts.get(&'e').copied().unwrap_or(0));
        assert!(counts[&'h'] > counts.get(&'u').copied().unwrap_or(0));
    }

    #[test]
    fn keeps_case_and_resolves_shifted_symbols() {
        let geometry = KeyboardGeometry::us_qwerty();
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        for _ in 0..200 {
            // Neighbours of 'K' include ',' and ';', which have no case.
            assert!(!geometry.wrong_char('K', &mut rng).is_ascii_lowercase());
            // ':' lives on ';', so its neighbour is an unshifted key near ';'.
            let near_colon = geometry.wrong_char(':', &mut rng);
            assert!(neighbour_set(&geometry, ';').contains(&near_colon));
        }
    }

    #[test]
    fn unknown_chars_fall_back() {
        let geometry = KeyboardGeometry::us_qwerty();
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        for _ in 0..100 {
            assert!(matches!(geometry.wrong_char(' ', &mut rng), 'x' | 'z'));
            assert!(geometry.wrong_char('é', &mut rng).is_ascii_lowercase());
        }
    }
}
