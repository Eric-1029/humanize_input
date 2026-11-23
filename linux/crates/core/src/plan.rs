//! Turns text + settings into a stream of typing actions.
//!
//! This is a faithful port of the original `TypingSessionService` loop, minus
//! all I/O: the planner only decides *what* to do, and [`crate::session`]
//! executes it. Separating the two makes the model testable without sending
//! keys and lets a dry run print the exact sequence.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::delay::next_delay_ms;
use crate::geometry::KeyboardGeometry;
use crate::keys::{KeyAction, KeyCode, Mods, Stroke};
use crate::layout::Layout;
use crate::progress::TypingProgress;
use crate::settings::TypingSettings;
use crate::text::normalize_newlines;

/// Share of repairs that happen only after typing 1–2 more characters.
const LOOKAHEAD_CHANCE: f64 = 0.35;
/// Shortest pause inside a transposed pair.
const MIN_TRANSPOSE_GAP_MS: u32 = 10;

/// One step for the executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Type this character into the target window.
    Type(char),
    /// Delete the character before the cursor.
    Backspace,
    /// Pause.
    Wait(Duration),
    /// Counters after the source character(s) of one step are done.
    Progress(TypingProgress),
}

/// Lazily yields the actions that type one text.
#[derive(Debug, Clone)]
pub struct Planner {
    text: Vec<char>,
    settings: TypingSettings,
    geometry: Arc<KeyboardGeometry>,
    rng: ChaCha8Rng,
    index: usize,
    progress: TypingProgress,
    pending: VecDeque<Action>,
}

impl Planner {
    /// A planner seeded from the OS, for real sessions.
    pub fn new(text: &str, settings: TypingSettings, geometry: Arc<KeyboardGeometry>) -> Self {
        Self::with_rng(text, settings, geometry, rand::make_rng())
    }

    /// A planner whose output is fully determined by `seed`.
    pub fn with_seed(
        text: &str,
        settings: TypingSettings,
        geometry: Arc<KeyboardGeometry>,
        seed: u64,
    ) -> Self {
        Self::with_rng(text, settings, geometry, ChaCha8Rng::seed_from_u64(seed))
    }

    fn with_rng(
        text: &str,
        settings: TypingSettings,
        geometry: Arc<KeyboardGeometry>,
        rng: ChaCha8Rng,
    ) -> Self {
        let text: Vec<char> = normalize_newlines(text).chars().collect();
        let progress = TypingProgress { total_chars: text.len(), ..TypingProgress::default() };
        Self { text, settings, geometry, rng, index: 0, progress, pending: VecDeque::new() }
    }

    /// Number of characters to type after normalization.
    pub fn total_chars(&self) -> usize {
        self.text.len()
    }

    fn roll(&mut self, percent: u32) -> bool {
        self.rng.random_range(0..100) < percent
    }

    fn wait_ms(&mut self, ms: u32) {
        self.pending.push_back(Action::Wait(Duration::from_millis(u64::from(ms))));
    }

    fn type_range(&mut self, start: usize, count: usize) {
        for i in start..start + count {
            self.pending.push_back(Action::Type(self.text[i]));
        }
    }

    fn backspaces(&mut self, count: usize) {
        for _ in 0..count {
            self.pending.push_back(Action::Backspace);
            self.wait_ms(self.settings.backspace_delay_ms);
        }
    }

    /// How many following characters get typed before a mistake is noticed.
    fn lookahead(&mut self, remaining: usize) -> usize {
        if remaining == 0 || self.rng.random::<f64>() >= LOOKAHEAD_CHANCE {
            return 0;
        }
        self.rng.random_range(1..3usize).min(remaining)
    }

    /// Notices a mistake after `lookahead` extra characters, deletes back
    /// `delete` characters, runs `retype`, then retypes the lookahead.
    fn repair(
        &mut self,
        lookahead_start: usize,
        lookahead: usize,
        delete: usize,
        retype: impl FnOnce(&mut Self),
    ) {
        self.type_range(lookahead_start, lookahead);
        self.wait_ms(self.settings.error_detect_delay_ms);
        self.backspaces(delete);
        retype(self);
        self.type_range(lookahead_start, lookahead);
    }

    /// Plans one source position (one or more characters) into `pending`.
    fn plan_step(&mut self) {
        let s = self.settings;
        let len = self.text.len();
        let index = self.index;
        let current = self.text[index];
        let next = self.text.get(index + 1).copied();

        let typo = !current.is_numeric() && self.roll(s.typo_rate_percent);
        let omission = !typo && self.roll(s.omission_rate_percent);
        let transpose = !typo
            && !omission
            && next.is_some_and(|n| !current.is_whitespace() && !n.is_whitespace())
            && self.roll(s.transpose_rate_percent);
        let will_repair = self.roll(s.repair_rate_percent);

        let mut p = self.progress;
        let consumed = if typo {
            p.typo_count += 1;
            let wrong = self.geometry.wrong_char(current, &mut self.rng);
            self.pending.push_back(Action::Type(wrong));
            if will_repair {
                let la = self.lookahead(len - (index + 1));
                self.repair(index + 1, la, la + 1, |me| me.pending.push_back(Action::Type(current)));
                p.correction_count += 1;
                1 + la
            } else {
                1
            }
        } else if omission {
            p.omission_count += 1;
            if will_repair {
                let la = self.lookahead(len - (index + 1));
                self.repair(index + 1, la, la, |me| me.pending.push_back(Action::Type(current)));
                p.correction_count += 1;
                1 + la
            } else {
                1
            }
        } else if let (true, Some(next)) = (transpose, next) {
            p.transpose_count += 1;
            let gap = (s.backspace_delay_ms / 2).max(MIN_TRANSPOSE_GAP_MS);
            let pair = |me: &mut Self, first: char, second: char| {
                me.pending.push_back(Action::Type(first));
                me.wait_ms(gap);
                me.pending.push_back(Action::Type(second));
            };
            pair(self, next, current);
            if will_repair {
                let la = self.lookahead(len - (index + 2));
                self.repair(index + 2, la, la + 2, |me| pair(me, current, next));
                p.correction_count += 1;
                2 + la
            } else {
                2
            }
        } else {
            self.pending.push_back(Action::Type(current));
            1
        };

        p.typed_chars += consumed;
        self.progress = p;
        self.index += consumed;
        self.pending.push_back(Action::Progress(p));
        let delay = next_delay_ms(&mut self.rng, s.base_delay_ms, s.jitter_percent);
        self.wait_ms(delay);
    }
}

impl Iterator for Planner {
    type Item = Action;

    fn next(&mut self) -> Option<Action> {
        if self.pending.is_empty() && self.index < self.text.len() {
            self.plan_step();
        }
        self.pending.pop_front()
    }
}

/// Converts classic character actions into instant key presses, like the
/// original `SendInput` driver: modifiers down, key down, key up, modifiers
/// up, with no time in between.
pub struct ClassicKeys {
    planner: Planner,
    layout: Arc<Layout>,
    pending: VecDeque<KeyAction>,
}

impl ClassicKeys {
    pub fn new(planner: Planner, layout: Arc<Layout>) -> Self {
        Self { planner, layout, pending: VecDeque::new() }
    }

    fn stroke(&mut self, stroke: Stroke) {
        let mod_keys: Vec<KeyCode> = stroke
            .mods
            .flags()
            .map(|f| match f {
                Mods::SHIFT => KeyCode::LEFT_SHIFT,
                Mods::CTRL => KeyCode::LEFT_CTRL,
                _ => KeyCode::RIGHT_ALT,
            })
            .collect();
        self.pending.extend(mod_keys.iter().map(|&k| KeyAction::Press(k)));
        self.pending.extend([KeyAction::Press(stroke.key), KeyAction::Release(stroke.key)]);
        self.pending.extend(mod_keys.iter().rev().map(|&k| KeyAction::Release(k)));
    }
}

impl Iterator for ClassicKeys {
    type Item = KeyAction;

    fn next(&mut self) -> Option<KeyAction> {
        while self.pending.is_empty() {
            match self.planner.next()? {
                // Characters the layout lacks were rejected before starting;
                // fallback typo letters exist on every Latin layout.
                Action::Type(c) => {
                    if let Some(stroke) = self.layout.stroke(c) {
                        self.stroke(stroke);
                    }
                }
                Action::Backspace => self.stroke(Stroke::new(KeyCode::BACKSPACE, Mods::NONE)),
                Action::Wait(d) => self.pending.push_back(KeyAction::Wait(d)),
                Action::Progress(p) => self.pending.push_back(KeyAction::Progress(p)),
            }
        }
        self.pending.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "The quick brown fox jumps over the lazy dog.\nfn main() { println!(\"42\"); }\n\tPack my box with 5 dozen liquor jugs!";

    fn geometry() -> Arc<KeyboardGeometry> {
        Arc::new(KeyboardGeometry::us_qwerty())
    }

    fn settings(typo: u32, omission: u32, transpose: u32, repair: u32) -> TypingSettings {
        TypingSettings {
            typo_rate_percent: typo,
            omission_rate_percent: omission,
            transpose_rate_percent: transpose,
            repair_rate_percent: repair,
            ..TypingSettings::default()
        }
    }

    /// Applies the actions to a text buffer, like the target editor would.
    fn replay(actions: impl IntoIterator<Item = Action>) -> String {
        let mut buffer = String::new();
        for action in actions {
            match action {
                Action::Type(c) => buffer.push(c),
                Action::Backspace => {
                    buffer.pop();
                }
                Action::Wait(_) | Action::Progress(_) => {}
            }
        }
        buffer
    }

    fn last_progress(actions: &[Action]) -> TypingProgress {
        actions
            .iter()
            .rev()
            .find_map(|a| match a {
                Action::Progress(p) => Some(*p),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn no_errors_types_text_verbatim() {
        let actions: Vec<_> = Planner::with_seed(SAMPLE, settings(0, 0, 0, 0), geometry(), 1).collect();
        assert_eq!(replay(actions.iter().copied()), SAMPLE);
        assert!(!actions.contains(&Action::Backspace));
        assert_eq!(last_progress(&actions).typed_chars, SAMPLE.chars().count());
    }

    #[test]
    fn full_repair_always_reproduces_text() {
        for seed in 0..200 {
            let planner = Planner::with_seed(SAMPLE, settings(30, 20, 20, 100), geometry(), seed);
            assert_eq!(replay(planner), SAMPLE, "seed {seed}");
        }
    }

    #[test]
    fn same_seed_same_actions() {
        let s = settings(10, 5, 5, 80);
        let a: Vec<_> = Planner::with_seed(SAMPLE, s, geometry(), 77).collect();
        let b: Vec<_> = Planner::with_seed(SAMPLE, s, geometry(), 77).collect();
        let c: Vec<_> = Planner::with_seed(SAMPLE, s, geometry(), 78).collect();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn digits_are_never_mistyped() {
        let digits = "0123456789".repeat(20);
        for seed in 0..20 {
            let out = replay(Planner::with_seed(&digits, settings(100, 0, 0, 0), geometry(), seed));
            assert_eq!(out, digits);
        }
    }

    #[test]
    fn unrepaired_omissions_drop_characters() {
        let actions: Vec<_> = Planner::with_seed("abcdef", settings(0, 100, 0, 0), geometry(), 4).collect();
        assert_eq!(replay(actions.iter().copied()), "");
        let p = last_progress(&actions);
        assert_eq!((p.typed_chars, p.omission_count, p.correction_count), (6, 6, 0));
    }

    #[test]
    fn unrepaired_transposes_swap_pairs_but_not_across_spaces() {
        let out = replay(Planner::with_seed("abcd ef", settings(0, 0, 100, 0), geometry(), 2));
        assert_eq!(out, "badc fe");
    }

    #[test]
    fn repaired_typo_counts_and_backspaces() {
        let actions: Vec<_> = Planner::with_seed("a", settings(100, 0, 0, 100), geometry(), 6).collect();
        assert_eq!(replay(actions.iter().copied()), "a");
        assert_eq!(actions.iter().filter(|a| **a == Action::Backspace).count(), 1);
        let p = last_progress(&actions);
        assert_eq!((p.typo_count, p.correction_count, p.typed_chars), (1, 1, 1));
    }

    #[test]
    fn newlines_are_normalized_and_counted_once() {
        let planner = Planner::with_seed("a\r\nb\rc", settings(0, 0, 0, 0), geometry(), 0);
        assert_eq!(planner.total_chars(), 5);
        assert_eq!(replay(planner), "a\nb\nc");
    }

    #[test]
    fn empty_text_yields_nothing() {
        assert_eq!(Planner::with_seed("", TypingSettings::default(), geometry(), 0).count(), 0);
    }

    #[test]
    fn classic_keys_type_the_same_text_instantly() {
        use crate::editor::VirtualEditor;
        let layout = Arc::new(Layout::us_qwerty());
        let planner = Planner::with_seed(SAMPLE, settings(20, 10, 10, 100), geometry(), 11);
        let actions: Vec<KeyAction> = ClassicKeys::new(planner, Arc::clone(&layout)).collect();
        let mut editor = VirtualEditor::new(layout);
        editor.apply_all(actions.iter().copied());
        assert_eq!(editor.text(), SAMPLE);
        assert!(editor.violations().is_empty(), "{:?}", editor.violations());
        assert!(editor.held().is_empty());
        assert_eq!(editor.rollovers(), 0);
    }
}
