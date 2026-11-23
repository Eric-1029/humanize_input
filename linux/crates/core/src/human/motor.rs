//! Motor layer: schedules physical key events in time.
//!
//! The intent layer calls [`Motor::press`] once per keystroke. The motor
//! picks the press time (finger/hand/bigram-aware log-normal interval), how
//! long the key stays down, and when modifiers go down and up, then emits the
//! events in time order. Keys may overlap (rollover), but modifiers never
//! change the character of a key they were not meant for: a modifier is down
//! before the press that needs it and up before the next press that doesn't.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use rand::RngExt;

use super::model::HumanModel;
use super::words::is_common_bigram;
use crate::keys::{Finger, Hand, KeyAction, KeyCode, Mods, Stroke};
use crate::layout::{KeyInfo, Layout};
use crate::progress::TypingProgress;

/// Shortest press-to-press interval a finger manages.
const MIN_INTERVAL_MS: f64 = 25.0;
/// A key cannot be pressed again sooner than this after its release.
const REPRESS_GAP: u64 = 15_000;
/// Gap kept between a modifier event and the key press it guards.
const MOD_GUARD: u64 = 10_000;
/// Keep Shift down across consecutive capitals only if the gap is short.
const MAX_MOD_HOLD_GAP: u64 = 300_000;
/// Dwell bounds; the upper one stays far below the auto-repeat delay.
const DWELL_RANGE_MS: (f64, f64) = (35.0, 220.0);

fn us(ms: f64) -> u64 {
    (ms * 1000.0).round().max(0.0) as u64
}

/// Standard normal sample (Box–Muller).
pub(crate) fn std_normal<R: rand::Rng + ?Sized>(rng: &mut R) -> f64 {
    let u1 = 1.0 - rng.random::<f64>(); // (0, 1]
    let u2 = rng.random::<f64>();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// Log-normal sample with the given median.
pub(crate) fn lognormal<R: rand::Rng + ?Sized>(rng: &mut R, median: f64, sigma: f64) -> f64 {
    median * (sigma * std_normal(rng)).exp()
}

/// Per-press context from the intent layer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PressCtx {
    /// Inside a very common word.
    pub burst: bool,
    /// Fatigue multiplier on intervals (≥ 1).
    pub slowdown: f64,
    /// Correction keystroke: uses the backspace rhythm.
    pub correction: bool,
}

struct Timed {
    at: u64,
    seq: u64,
    action: KeyAction,
}

// Min-heap on (at, seq).
impl Ord for Timed {
    fn cmp(&self, other: &Self) -> Ordering {
        (other.at, other.seq).cmp(&(self.at, self.seq))
    }
}
impl PartialOrd for Timed {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Timed {
    fn eq(&self, other: &Self) -> bool {
        (self.at, self.seq) == (other.at, other.seq)
    }
}
impl Eq for Timed {}

#[derive(Debug, Clone, Copy)]
struct HeldMod {
    flag: Mods,
    key: KeyCode,
    /// Press time of the last key that used this modifier.
    last_use: u64,
    /// Natural release time (after that key comes up).
    release_after: u64,
}

pub(crate) struct Motor {
    layout: Arc<Layout>,
    model: HumanModel,
    /// Time of the latest character-key press, in µs. Every event still to
    /// be scheduled lies strictly after it.
    now: u64,
    started: bool,
    prev: Option<(KeyCode, Option<char>)>,
    released_at: HashMap<KeyCode, u64>,
    held: Vec<HeldMod>,
    pending_think: u64,
    heap: BinaryHeap<Timed>,
    seq: u64,
    emitted_at: u64,
}

impl Motor {
    pub(crate) fn new(layout: Arc<Layout>, model: HumanModel) -> Self {
        Self {
            layout,
            model,
            now: 0,
            started: false,
            prev: None,
            released_at: HashMap::new(),
            held: Vec::new(),
            pending_think: 0,
            heap: BinaryHeap::new(),
            seq: 0,
            emitted_at: 0,
        }
    }

    /// Adds a pause before the next press.
    pub(crate) fn think(&mut self, ms: f64) {
        self.pending_think += us(ms);
    }

    pub(crate) fn progress(&mut self, p: TypingProgress) {
        self.push(self.now, KeyAction::Progress(p));
    }

    fn push(&mut self, at: u64, action: KeyAction) {
        self.seq += 1;
        self.heap.push(Timed { at, seq: self.seq, action });
    }

    fn interval_ms<R: rand::Rng + ?Sized>(
        &self,
        key: KeyCode,
        c: Option<char>,
        ctx: PressCtx,
        rng: &mut R,
    ) -> f64 {
        let m = &self.model;
        let (mean, sigma) = if ctx.correction {
            (m.backspace_interval_ms * ctx.slowdown, 0.25)
        } else {
            let mut factor = 1.0;
            if let Some((prev_key, prev_char)) = self.prev {
                let info = |k| self.layout.key_info(k).copied();
                factor *= match (info(prev_key), info(key)) {
                    _ if prev_key == key => 1.1,
                    (Some(a), Some(b)) => self.hand_factor(a, b),
                    _ => 1.0,
                };
                if let (Some(a), Some(b)) = (prev_char, c)
                    && is_common_bigram(a, b)
                {
                    factor *= m.common_bigram_factor;
                }
            }
            if ctx.burst {
                factor *= m.familiar_word_factor;
            }
            (12_000.0 / m.wpm * factor * ctx.slowdown, m.interval_sigma)
        };
        // Median below the mean so the long right tail keeps the mean on target.
        let median = mean * (-sigma * sigma / 2.0).exp();
        lognormal(rng, median, sigma).clamp(MIN_INTERVAL_MS, mean * 8.0)
    }

    fn hand_factor(&self, a: KeyInfo, b: KeyInfo) -> f64 {
        let m = &self.model;
        if a.finger == b.finger && a.finger != Finger::Thumb {
            m.same_finger_factor
        } else {
            match (a.finger.hand(), b.finger.hand()) {
                (Some(x), Some(y)) if x == y => m.same_hand_factor,
                _ => m.alternate_hand_factor,
            }
        }
    }

    /// Which physical key provides `flag` when typing `key`.
    fn modifier_key(&self, flag: Mods, key: KeyCode) -> KeyCode {
        if flag == Mods::SHIFT {
            // Touch typists use the Shift on the other hand.
            match self.layout.key_info(key).and_then(|i| i.finger.hand()) {
                Some(Hand::Left) => KeyCode::RIGHT_SHIFT,
                _ => KeyCode::LEFT_SHIFT,
            }
        } else if flag == Mods::CTRL {
            KeyCode::LEFT_CTRL
        } else {
            KeyCode::RIGHT_ALT
        }
    }

    /// Schedules one keystroke.
    pub(crate) fn press<R: rand::Rng + ?Sized>(&mut self, stroke: Stroke, ctx: PressCtx, rng: &mut R) {
        let c = self.layout.char_for(stroke.key, stroke.mods);
        let interval = if self.started { us(self.interval_ms(stroke.key, c, ctx, rng)) } else { 0 };
        let mut at = self.now + interval + std::mem::take(&mut self.pending_think);
        if let Some(&released) = self.released_at.get(&stroke.key) {
            at = at.max(released + REPRESS_GAP);
        }

        // Release modifiers this press must not see.
        let mut i = 0;
        while i < self.held.len() {
            let h = self.held[i];
            let keep = stroke.mods.contains(h.flag) && at.saturating_sub(h.release_after) <= MAX_MOD_HOLD_GAP;
            if keep {
                i += 1;
                continue;
            }
            let release = h.release_after.min(at.saturating_sub(MOD_GUARD)).max(h.last_use + MOD_GUARD);
            at = at.max(release + MOD_GUARD);
            self.push(release, KeyAction::Release(h.key));
            self.released_at.insert(h.key, release);
            self.held.remove(i);
        }

        // Press the modifiers it needs that are not down yet.
        for flag in stroke.mods.flags() {
            if self.held.iter().any(|h| h.flag == flag) {
                continue;
            }
            let key = self.modifier_key(flag, stroke.key);
            let lead = us(lognormal(rng, self.model.modifier_lead_ms, 0.3).clamp(15.0, 250.0));
            // After the previous press (so it keeps its case) and after this
            // modifier's own last release.
            let earliest =
                (self.now + MOD_GUARD).max(self.released_at.get(&key).map_or(0, |&r| r + REPRESS_GAP));
            let down = at.saturating_sub(lead).max(earliest);
            at = at.max(down + REPRESS_GAP);
            self.push(down, KeyAction::Press(key));
            self.held.push(HeldMod { flag, key, last_use: at, release_after: 0 });
        }

        let (lo, hi) = DWELL_RANGE_MS;
        let dwell = us(lognormal(rng, self.model.dwell_ms, self.model.dwell_sigma).clamp(lo, hi));
        let up = at + dwell;
        self.push(at, KeyAction::Press(stroke.key));
        self.push(up, KeyAction::Release(stroke.key));
        self.released_at.insert(stroke.key, up);

        for h in self.held.iter_mut().filter(|h| stroke.mods.contains(h.flag)) {
            h.last_use = at;
            h.release_after = up + us(lognormal(rng, 25.0, 0.4).clamp(5.0, 120.0));
        }
        self.now = at;
        self.started = true;
        self.prev = Some((stroke.key, c));
    }

    /// Moves every event that can no longer be preceded by a new one to `out`.
    pub(crate) fn drain(&mut self, out: &mut VecDeque<KeyAction>) {
        while self.heap.peek().is_some_and(|t| t.at <= self.now) {
            self.emit_next(out);
        }
    }

    /// Releases remaining modifiers and flushes everything.
    pub(crate) fn finish(&mut self, out: &mut VecDeque<KeyAction>) {
        for h in std::mem::take(&mut self.held) {
            self.push(h.release_after.max(h.last_use + MOD_GUARD), KeyAction::Release(h.key));
        }
        while !self.heap.is_empty() {
            self.emit_next(out);
        }
    }

    fn emit_next(&mut self, out: &mut VecDeque<KeyAction>) {
        let Some(t) = self.heap.pop() else { return };
        if t.at > self.emitted_at {
            out.push_back(KeyAction::Wait(Duration::from_micros(t.at - self.emitted_at)));
            self.emitted_at = t.at;
        }
        out.push_back(t.action);
    }
}
