//! The human typing model: decides what to type and where it goes wrong
//! (intent), then hands each keystroke to the [`motor::Motor`] for timing.

pub mod model;
mod motor;
mod words;

use std::collections::VecDeque;
use std::sync::Arc;

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

pub use model::{HumanModel, Macros, Preset, WordDelete};
use motor::{Motor, PressCtx, lognormal};
use words::{is_familiar, is_word_char};

use crate::keys::{KeyAction, KeyCode, Mods, Stroke};
use crate::layout::Layout;
use crate::progress::TypingProgress;
use crate::text::normalize_newlines;

/// Spread of pause lengths around their medians.
const PAUSE_SIGMA: f64 = 0.4;
/// Share of characters fumbled in a word that gets redone.
const REDO_FUMBLE_PERCENT: f64 = 40.0;
/// Pause after erasing a fumbled word, before retyping it.
const REDO_RESTART_MS: f64 = 250.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slip {
    Typo,
    Omission,
    Transpose,
    Double,
    Case,
}

/// Lazily yields the key actions that type one text like a person.
pub struct HumanPlanner {
    text: Vec<char>,
    model: HumanModel,
    layout: Arc<Layout>,
    rng: ChaCha8Rng,
    motor: Motor,
    index: usize,
    progress: TypingProgress,
    /// End of the familiar word being typed (bursts run up to here).
    burst_until: usize,
    /// False right after a mistake was left in place; the text before the
    /// cursor then differs from the source and word deletion is unsafe.
    tail_matches: bool,
    out: VecDeque<KeyAction>,
    finished: bool,
}

impl HumanPlanner {
    /// A planner seeded from the OS, for real sessions.
    pub fn new(text: &str, model: HumanModel, layout: Arc<Layout>) -> Self {
        Self::with_rng(text, model, layout, rand::make_rng())
    }

    /// A planner whose output is fully determined by `seed`.
    pub fn with_seed(text: &str, model: HumanModel, layout: Arc<Layout>, seed: u64) -> Self {
        Self::with_rng(text, model, layout, ChaCha8Rng::seed_from_u64(seed))
    }

    fn with_rng(text: &str, model: HumanModel, layout: Arc<Layout>, rng: ChaCha8Rng) -> Self {
        let model = model.sanitized();
        let text: Vec<char> = normalize_newlines(text).chars().collect();
        let progress = TypingProgress { total_chars: text.len(), ..TypingProgress::default() };
        Self {
            motor: Motor::new(Arc::clone(&layout), model),
            text,
            model,
            layout,
            rng,
            index: 0,
            progress,
            burst_until: 0,
            tail_matches: true,
            out: VecDeque::new(),
            finished: false,
        }
    }

    pub fn total_chars(&self) -> usize {
        self.text.len()
    }

    fn roll(&mut self, percent: f64) -> bool {
        self.rng.random::<f64>() * 100.0 < percent
    }

    /// (interval slowdown, error multiplier) from fatigue so far.
    fn fatigue(&self) -> (f64, f64) {
        let thousands = self.progress.typed_chars as f64 / 1000.0;
        let slow = (self.model.fatigue_slowdown_percent / 100.0 * thousands).min(0.3);
        let err = (self.model.fatigue_error_percent / 100.0 * thousands).min(0.5);
        (1.0 + slow, 1.0 + err)
    }

    fn think(&mut self, median_ms: f64) {
        if median_ms > 0.0 {
            let ms = lognormal(&mut self.rng, median_ms, PAUSE_SIGMA);
            self.motor.think(ms);
        }
    }

    fn press(&mut self, stroke: Stroke, correction: bool) {
        let ctx = PressCtx { burst: self.index < self.burst_until, slowdown: self.fatigue().0, correction };
        self.motor.press(stroke, ctx, &mut self.rng);
    }

    fn type_char(&mut self, c: char) {
        if let Some(stroke) = self.layout.stroke(c) {
            self.press(stroke, false);
        }
    }

    fn type_range(&mut self, from: usize, count: usize) {
        for j in from..from + count {
            self.type_char(self.text[j]);
        }
    }

    fn backspaces(&mut self, count: usize) {
        for _ in 0..count {
            self.press(Stroke::new(KeyCode::BACKSPACE, Mods::NONE), true);
        }
    }

    /// An adjacent key with the same modifiers whose character passes `ok`.
    fn wrong_stroke(&mut self, stroke: Stroke, ok: impl Fn(char) -> bool) -> Option<Stroke> {
        let layout = Arc::clone(&self.layout);
        layout
            .neighbour_key(stroke.key, &mut self.rng, |k| layout.char_for(k, stroke.mods).is_some_and(&ok))
            .map(|key| Stroke::new(key, stroke.mods))
    }

    /// Thinking pauses that belong before position `i`.
    fn boundary_pause(&mut self, i: usize) {
        let c = self.text[i];
        let (Some(&prev), prev2) = (i.checked_sub(1).and_then(|j| self.text.get(j)), i.checked_sub(2)) else {
            return;
        };
        let m = self.model;
        if prev == '\n' {
            if c != '\n' {
                let blank_line = prev2.is_some_and(|j| self.text[j] == '\n');
                self.think(m.paragraph_pause_ms * if blank_line { 1.0 } else { 0.6 });
            }
        } else if matches!(prev, ',' | ';' | ':') {
            self.think(m.punctuation_pause_ms);
        } else if prev.is_whitespace()
            && !c.is_whitespace()
            && prev2.is_some_and(|j| matches!(self.text[j], '.' | '!' | '?'))
        {
            self.think(m.sentence_pause_ms);
        }
    }

    /// Word-level decisions at the first character of a word.
    fn start_word(&mut self, start: usize) {
        let end = start + self.text[start..].iter().take_while(|&&c| is_word_char(c)).count();
        let familiar = is_familiar(&self.text[start..end]);
        self.burst_until = if familiar { end } else { start };

        let m = self.model;
        if self.roll(m.long_pause_percent) {
            self.think(m.long_pause_ms);
        }
        self.think(m.word_pause_ms);

        // Word deletion must only ever remove the fumbled letters: the word
        // has to be pure letters, follow whitespace, and the output so far
        // must match the source.
        let redo_ok = end - start >= 4
            && start > 0
            && self.text[start - 1].is_whitespace()
            && self.tail_matches
            && self.text[start..end].iter().all(char::is_ascii_alphabetic);
        if redo_ok && self.roll(m.word_redo_percent * self.fatigue().1) {
            self.redo_word(start, end);
        }
    }

    /// Fumbles the first few letters of a word, erases them and starts over.
    fn redo_word(&mut self, start: usize, end: usize) {
        let typed = self.rng.random_range(2..end - start);
        let forced = self.rng.random_range(0..typed);
        for j in 0..typed {
            let Some(stroke) = self.layout.stroke(self.text[start + j]) else { continue };
            let fumble = j == forced || self.roll(REDO_FUMBLE_PERCENT);
            let wrong = if fumble { self.wrong_stroke(stroke, |c| c.is_ascii_alphabetic()) } else { None };
            self.press(wrong.unwrap_or(stroke), false);
        }
        self.think(self.model.notice_delay_ms);
        let ctrl_backspace = match self.model.word_delete {
            WordDelete::CtrlBackspace => self.layout.modifier_keys().ctrl.is_some(),
            // Unresolved Auto means the target is unknown: play it safe.
            WordDelete::Auto | WordDelete::Backspaces => false,
        };
        if ctrl_backspace {
            self.press(Stroke::new(KeyCode::BACKSPACE, Mods::CTRL), true);
        } else {
            self.backspaces(typed);
        }
        self.think(REDO_RESTART_MS);
        self.progress.word_redo_count += 1;
        self.progress.correction_count += 1;
    }

    fn choose_slip(&mut self, i: usize, err: f64) -> Option<Slip> {
        let m = self.model;
        let c = self.text[i];
        let prev = i.checked_sub(1).map(|j| self.text[j]);
        let next = self.text.get(i + 1).copied();
        let case_matters = c.is_ascii_uppercase()
            || (c.is_ascii_lowercase() && prev.is_some_and(|p| p.is_ascii_uppercase()));

        let options = [
            (Slip::Typo, !c.is_ascii_digit() && !c.is_whitespace(), m.typo_percent),
            (Slip::Omission, c != '\n', m.omission_percent),
            (
                Slip::Transpose,
                next.is_some_and(|n| {
                    !c.is_whitespace() && !n.is_whitespace() && self.layout.stroke(n).is_some()
                }),
                m.transpose_percent,
            ),
            (Slip::Double, c != '\n', m.double_press_percent),
            (Slip::Case, case_matters, m.case_slip_percent),
        ];
        let mut pick = self.rng.random::<f64>() * 100.0;
        for (slip, allowed, rate) in options {
            if allowed {
                pick -= rate * err;
                if pick < 0.0 {
                    return Some(slip);
                }
            }
        }
        None
    }

    /// How many following characters get typed before a mistake is noticed.
    /// A skipped character is only noticed once the next one appears.
    fn lookahead(&mut self, from: usize, omission: bool) -> usize {
        let available =
            self.text[from.min(self.text.len())..].iter().take(3).take_while(|&&c| c != '\n').count();
        if available == 0 {
            return 0;
        }
        let more = usize::from(self.roll(self.model.lookahead_percent));
        let wanted = if omission {
            1 + more
        } else if more == 1 {
            self.rng.random_range(1..=2)
        } else {
            0
        };
        wanted.min(available)
    }

    fn plan_step(&mut self) {
        let i = self.index;
        let c = self.text[i];
        let Some(stroke) = self.layout.stroke(c) else {
            // Unreachable after validation; skip rather than stall.
            self.index += 1;
            self.progress.typed_chars += 1;
            return;
        };

        self.boundary_pause(i);
        if is_word_char(c) && !i.checked_sub(1).is_some_and(|j| is_word_char(self.text[j])) {
            self.start_word(i);
        }

        let mut slip = self.choose_slip(i, self.fatigue().1);
        let mut consumed = 1;
        let mut wrong_output = 0;
        match slip {
            None => self.press(stroke, false),
            Some(Slip::Typo) => {
                match self.wrong_stroke(stroke, |w| !w.is_ascii_digit() && !w.is_whitespace()) {
                    Some(wrong) => {
                        self.press(wrong, false);
                        wrong_output = 1;
                        self.progress.typo_count += 1;
                    }
                    None => {
                        self.press(stroke, false);
                        slip = None;
                    }
                }
            }
            Some(Slip::Omission) => self.progress.omission_count += 1,
            Some(Slip::Transpose) => {
                self.type_char(self.text[i + 1]);
                self.press(stroke, false);
                consumed = 2;
                wrong_output = 2;
                self.progress.transpose_count += 1;
            }
            Some(Slip::Double) => {
                self.press(stroke, false);
                self.press(stroke, false);
                wrong_output = 2;
                self.progress.double_press_count += 1;
            }
            Some(Slip::Case) => {
                let mods = if c.is_ascii_uppercase() {
                    stroke.mods - Mods::SHIFT
                } else {
                    stroke.mods | Mods::SHIFT
                };
                self.press(Stroke::new(stroke.key, mods), false);
                wrong_output = 1;
                self.progress.case_slip_count += 1;
            }
        }

        self.tail_matches = true;
        if let Some(slip) = slip {
            if self.roll(self.model.repair_percent) {
                let la = self.lookahead(i + consumed, slip == Slip::Omission);
                self.type_range(i + consumed, la);
                self.think(self.model.notice_delay_ms);
                self.backspaces(wrong_output + la);
                self.type_range(i, consumed + la);
                consumed += la;
                self.progress.correction_count += 1;
            } else {
                self.tail_matches = false;
            }
        }

        self.index += consumed;
        self.progress.typed_chars += consumed;
        self.motor.progress(self.progress);
    }
}

impl Iterator for HumanPlanner {
    type Item = KeyAction;

    fn next(&mut self) -> Option<KeyAction> {
        loop {
            if let Some(action) = self.out.pop_front() {
                return Some(action);
            }
            if self.finished {
                return None;
            }
            if self.index < self.text.len() {
                self.plan_step();
                self.motor.drain(&mut self.out);
            } else {
                self.motor.finish(&mut self.out);
                self.finished = true;
            }
        }
    }
}

#[cfg(test)]
mod tests;
