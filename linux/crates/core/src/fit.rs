//! Typing test analysis: fits [`HumanModel`] values to a real typing sample.
//!
//! Input is what a text box sees while the user copies a prompt: key downs
//! and ups (physical keys, timestamps) and the text edits they cause. From
//! these we measure key holds, rhythm, pauses, Shift timing, backspacing and
//! each kind of mistake, then turn the measurements into model values.
//!
//! Interval-based values (speed, irregularity, finger/hand factors) depend on
//! how the model combines its factors, so they are calibrated by simulating
//! the model on the same prompt and comparing like with like.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::editor::VirtualEditor;
use crate::human::{HumanModel, HumanPlanner};
use crate::keys::{Finger, KeyCode};
use crate::layout::Layout;
use crate::text::normalize_newlines;

/// Built-in samples: ordinary prose with capitals and punctuation, long
/// enough (600–700 characters) for every measurement to have data.
pub const SAMPLES: [&str; 3] = [
    "The library opens at nine, but the reading room is usually empty until ten. On quiet mornings I take the \
     corner table by the window, where the light is good and the radiator hums. Today I brought two books, a \
     notebook, and a list of questions for the chapter I am writing. The first question is simple: why did the \
     old harbour close in 1931? The second one is harder. Nobody seems to remember who owned the boats, and the \
     records I found last week stop in the middle of a sentence. Still, a missing page is a clue of its own. \
     Somebody took it, or lost it, or never wrote it at all.",
    "Our team ships a small update every Thursday. On Monday we pick the work, on Tuesday we build it, and on \
     Wednesday we test everything twice. It sounds strict, but it keeps the week calm: nobody stays late, and \
     problems show up early, when they are still cheap to fix. Last month we skipped the second test because a \
     customer was waiting. The update went out on time, and then it broke the login page for three hours. Since \
     then the rule is clear. If the tests are not finished, the release waits, even if somebody important is \
     asking. People complained for a week; now they say it was the right call.",
    "My grandmother kept a garden behind the house, about twenty steps long and ten wide. She grew beans, \
     tomatoes, and a row of tall yellow flowers that she never named. Every evening in summer she walked along \
     the paths with a watering can and a pair of scissors, cutting whatever looked tired. When I visited, she \
     gave me small jobs: pull the weeds near the fence, count the ripe tomatoes, chase the cat away from the \
     seedlings. I was not very good at any of them. But I remember the smell of wet soil after rain, and the way \
     she laughed when the cat came back, again and again, as if it owned the place.",
];

/// One observation from the text box, timestamps from the start of the test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleEvent {
    KeyDown {
        t: Duration,
        key: KeyCode,
    },
    KeyUp {
        t: Duration,
        key: KeyCode,
    },
    /// `text` inserted at character offset `at`.
    Insert {
        t: Duration,
        at: usize,
        text: String,
    },
    /// Characters `start..end` removed.
    Delete {
        t: Duration,
        start: usize,
        end: usize,
    },
}

impl SampleEvent {
    pub fn time(&self) -> Duration {
        match self {
            Self::KeyDown { t, .. }
            | Self::KeyUp { t, .. }
            | Self::Insert { t, .. }
            | Self::Delete { t, .. } => *t,
        }
    }
}

/// One measured model value.
#[derive(Debug, Clone, PartialEq)]
pub struct FitItem {
    /// The [`HumanModel`] field this measures (see [`FitItem::apply`]).
    pub field: &'static str,
    pub value: f64,
    /// Observations behind the value.
    pub samples: usize,
    /// Enough observations to trust the value.
    pub reliable: bool,
}

impl FitItem {
    /// Writes the value into `model`.
    pub fn apply(&self, model: &mut HumanModel) {
        let v = self.value;
        match self.field {
            "wpm" => model.wpm = v,
            "interval_sigma" => model.interval_sigma = v,
            "same_finger_factor" => model.same_finger_factor = v,
            "same_hand_factor" => model.same_hand_factor = v,
            "alternate_hand_factor" => model.alternate_hand_factor = v,
            "common_bigram_factor" => model.common_bigram_factor = v,
            "familiar_word_factor" => model.familiar_word_factor = v,
            "dwell_ms" => model.dwell_ms = v,
            "dwell_sigma" => model.dwell_sigma = v,
            "modifier_lead_ms" => model.modifier_lead_ms = v,
            "word_pause_ms" => model.word_pause_ms = v,
            "punctuation_pause_ms" => model.punctuation_pause_ms = v,
            "sentence_pause_ms" => model.sentence_pause_ms = v,
            "paragraph_pause_ms" => model.paragraph_pause_ms = v,
            "long_pause_percent" => model.long_pause_percent = v,
            "long_pause_ms" => model.long_pause_ms = v,
            "typo_percent" => model.typo_percent = v,
            "omission_percent" => model.omission_percent = v,
            "transpose_percent" => model.transpose_percent = v,
            "double_press_percent" => model.double_press_percent = v,
            "case_slip_percent" => model.case_slip_percent = v,
            "word_redo_percent" => model.word_redo_percent = v,
            "repair_percent" => model.repair_percent = v,
            "notice_delay_ms" => model.notice_delay_ms = v,
            "lookahead_percent" => model.lookahead_percent = v,
            "backspace_interval_ms" => model.backspace_interval_ms = v,
            _ => {}
        }
    }
}

/// Everything learned from one sample.
#[derive(Debug, Clone, PartialEq)]
pub struct FitReport {
    pub items: Vec<FitItem>,
    /// `base` with every reliable item applied.
    pub model: HumanModel,
    /// Overall speed including pauses and corrections, in WPM.
    pub gross_wpm: f64,
    /// Share of the prompt that ended up correct, in percent.
    pub accuracy: f64,
    pub duration: Duration,
    pub key_presses: usize,
    /// Share of character presses made while another key was still down.
    pub rollover_percent: f64,
    pub mistakes: usize,
    pub corrections: usize,
}

impl FitReport {
    pub fn item(&self, field: &str) -> Option<&FitItem> {
        self.items.iter().find(|i| i.field == field)
    }
}

// ── Small statistics helpers ───────────────────────────────────────────────

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len().is_multiple_of(2) { (v[mid - 1] + v[mid]) / 2.0 } else { v[mid] })
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// Standard deviation of the natural log (the log-normal shape).
fn log_sigma(values: &[f64]) -> Option<f64> {
    let logs: Vec<f64> = values.iter().filter(|v| **v > 0.0).map(|v| v.ln()).collect();
    if logs.len() < 2 {
        return None;
    }
    let m = mean(&logs)?;
    Some((logs.iter().map(|l| (l - m).powi(2)).sum::<f64>() / (logs.len() - 1) as f64).sqrt())
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

// ── Intervals between typed characters ─────────────────────────────────────

/// Gaps between consecutive appended characters, sorted by context.
#[derive(Debug, Default)]
struct Intervals {
    /// Inside a word, no correction nearby: the steady rhythm.
    flow: Vec<f64>,
    same_finger: Vec<f64>,
    same_hand: Vec<f64>,
    alternate: Vec<f64>,
    bigram: Vec<f64>,
    familiar: Vec<f64>,
    unfamiliar: Vec<f64>,
    word_start: Vec<f64>,
    after_punctuation: Vec<f64>,
    sentence_start: Vec<f64>,
    after_newline: Vec<f64>,
    words: usize,
}

const COMMON_BIGRAMS: &[&str] = &[
    "th", "he", "in", "er", "an", "re", "on", "at", "en", "nd", "ti", "es", "or", "te", "of", "ed", "is",
    "it", "al", "ar", "st", "to", "nt", "ng", "se", "ha", "as", "ou", "io", "le", "ve", "co", "me", "de",
    "hi", "ri", "ro", "ic", "ne", "ea", "ra", "ce",
];
const FAMILIAR: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "back", "be", "because",
    "been", "but", "by", "can", "come", "could", "day", "do", "even", "first", "for", "from", "get", "give",
    "go", "good", "had", "has", "have", "he", "her", "him", "his", "how", "i", "if", "in", "into", "is",
    "it", "its", "just", "know", "like", "look", "make", "me", "most", "my", "new", "no", "not", "now", "of",
    "on", "one", "only", "or", "other", "our", "out", "over", "people", "say", "see", "she", "so", "some",
    "take", "than", "that", "the", "their", "them", "then", "there", "these", "they", "think", "this",
    "time", "to", "two", "up", "us", "use", "want", "was", "way", "we", "well", "were", "what", "when",
    "which", "who", "will", "with", "work", "would", "year", "you", "your",
];

fn bigram(a: char, b: char) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    COMMON_BIGRAMS.iter().any(|p| {
        let mut c = p.chars();
        c.next() == Some(a) && c.next() == Some(b)
    })
}

/// A typed character: when it appeared and the text before it.
struct Typed {
    t: f64,
    c: char,
    /// Previous two characters of the text at that moment.
    prev: Option<char>,
    prev2: Option<char>,
    /// The word this character belongs to (lower case), if any.
    word: String,
    /// A deletion happened since the previous typed character.
    after_delete: bool,
    /// Physical key that typed it, if known.
    key: Option<KeyCode>,
}

/// Replays the edits and lists every appended single character.
fn typed_chars(events: &[SampleEvent]) -> Vec<Typed> {
    let mut text: Vec<char> = Vec::new();
    let mut out = Vec::new();
    let mut deleted = false;
    let mut last_key: Option<(f64, KeyCode)> = None;
    for e in events {
        match e {
            SampleEvent::KeyDown { t, key } if !key.is_modifier() && *key != KeyCode::BACKSPACE => {
                last_key = Some((ms(*t), *key));
            }
            SampleEvent::Insert { t, at, text: s } => {
                let chars: Vec<char> = s.chars().collect();
                let at = (*at).min(text.len());
                if chars.len() == 1 && at == text.len() {
                    let c = chars[0];
                    let t = ms(*t);
                    let mut start = text.len();
                    while start > 0 && is_word_char(text[start - 1]) {
                        start -= 1;
                    }
                    let word: String = if is_word_char(c) {
                        text[start..].iter().chain([c].iter()).collect::<String>().to_ascii_lowercase()
                    } else {
                        String::new()
                    };
                    let key = last_key.filter(|(kt, _)| (t - kt).abs() < 5.0).map(|(_, k)| k);
                    out.push(Typed {
                        t,
                        c,
                        prev: text.last().copied(),
                        prev2: text.len().checked_sub(2).map(|i| text[i]),
                        word,
                        after_delete: deleted,
                        key,
                    });
                    deleted = false;
                }
                for (i, c) in chars.into_iter().enumerate() {
                    text.insert(at + i, c);
                }
            }
            SampleEvent::Delete { start, end, .. } => {
                let (s, e) = ((*start).min(text.len()), (*end).min(text.len()));
                text.drain(s..e);
                deleted = true;
            }
            _ => {}
        }
    }
    out
}

/// Long stops are not part of the rhythm.
const LONG_GAP_MS: f64 = 2500.0;

fn intervals(events: &[SampleEvent], layout: &Layout) -> Intervals {
    let typed = typed_chars(events);
    let mut iv = Intervals::default();
    for w in typed.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let gap = b.t - a.t;
        if gap <= 0.0 || b.after_delete || a.after_delete {
            continue;
        }
        let prev = a.c;
        if is_word_char(b.c) && !is_word_char(prev) {
            iv.words += 1;
        }
        if gap > LONG_GAP_MS {
            continue;
        }
        if prev == '\n' {
            iv.after_newline.push(gap);
        } else if matches!(prev, ',' | ';' | ':') {
            iv.after_punctuation.push(gap);
        } else if prev.is_whitespace() && is_word_char(b.c) && matches!(a.prev, Some('.' | '!' | '?')) {
            iv.sentence_start.push(gap);
        } else if prev.is_whitespace() && is_word_char(b.c) {
            iv.word_start.push(gap);
        } else if is_word_char(prev) && is_word_char(b.c) {
            iv.flow.push(gap);
            if FAMILIAR.contains(&b.word.as_str())
                || FAMILIAR.iter().any(|f| f.starts_with(b.word.as_str()) && f.len() > b.word.len())
            {
                iv.familiar.push(gap);
            } else {
                iv.unfamiliar.push(gap);
            }
            if bigram(prev, b.c) {
                iv.bigram.push(gap);
            }
            let fingers = a.key.zip(b.key).and_then(|(ka, kb)| {
                Some((layout.key_info(ka)?.finger, layout.key_info(kb)?.finger, ka == kb))
            });
            if let Some((fa, fb, same_key)) = fingers {
                if same_key {
                    // Repeated letters are their own case; leave them out.
                } else if fa == fb && fa != Finger::Thumb {
                    iv.same_finger.push(gap);
                } else if fa.hand().is_some() && fa.hand() == fb.hand() {
                    iv.same_hand.push(gap);
                } else {
                    iv.alternate.push(gap);
                }
            }
        }
        let _ = a.prev2;
    }
    iv
}

/// Events the model itself produces for `prompt`, for calibration.
fn simulate(prompt: &str, model: HumanModel, layout: &Arc<Layout>, seed: u64) -> Vec<SampleEvent> {
    let mut editor = VirtualEditor::new(Arc::clone(layout)).recording();
    editor.apply_all(HumanPlanner::with_seed(prompt, model, Arc::clone(layout), seed));
    editor.sample_events().to_vec()
}

// ── Mistakes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mistake {
    Typo,
    Omission,
    Transpose,
    Double,
    Case,
    WordRedo,
}

#[derive(Debug, Default)]
struct Corrections {
    /// (kind, characters typed past the mistake before noticing it)
    fixed: Vec<(Mistake, usize)>,
    /// Pause between the last typed character and the first deletion.
    notice_gaps: Vec<f64>,
}

/// Where the end of `kept` sits in the prompt. Mistakes left in earlier
/// shift the two apart, so search near `guess` for the best match of the
/// last characters.
fn locate(kept: &[char], prompt: &[char], guess: usize) -> usize {
    const WINDOW: usize = 20;
    const SLACK: isize = 6;
    let mut best = (usize::MAX, usize::MAX, guess.min(prompt.len()));
    for d in -SLACK..=SLACK {
        let Some(j) = guess.checked_add_signed(d).filter(|j| *j <= prompt.len()) else { continue };
        let l = WINDOW.min(kept.len()).min(j);
        let mismatches = kept[kept.len() - l..].iter().zip(&prompt[j - l..j]).filter(|(a, b)| a != b).count()
            + (WINDOW.min(kept.len()) - l);
        let key = (mismatches, d.unsigned_abs());
        if key < (best.0, best.1) {
            best = (key.0, key.1, j);
        }
    }
    best.2
}

/// Groups deletions into correction bursts and classifies what was fixed.
fn corrections(prompt: &[char], events: &[SampleEvent]) -> Corrections {
    let mut text: Vec<char> = Vec::new();
    let mut out = Corrections::default();
    let mut last_insert_t: Option<f64> = None;
    // Current burst: text before it, time of its first deletion.
    let mut burst: Option<(Vec<char>, f64)> = None;
    let mut last_delete_t = 0.0;
    // Prompt index minus text index, carried from burst to burst.
    let mut offset: isize = 0;

    let mut close =
        |burst: (Vec<char>, f64), text: &[char], out: &mut Corrections, last_insert_t: Option<f64>| {
            let (before, first_delete) = burst;
            let p = text.len().min(before.len());
            if text[..p] != before[..p] {
                return; // not a plain trim from the end
            }
            let wrong = &before[p..];
            let at = locate(&before[..p], prompt, p.saturating_add_signed(offset));
            offset = at as isize - p as isize;
            let expected: Vec<char> = prompt.iter().skip(at).copied().collect();
            if let Some(kind) = classify(&before[..p], wrong, &expected) {
                out.fixed.push(kind);
                if let Some(t) = last_insert_t {
                    out.notice_gaps.push(first_delete - t);
                }
            }
        };

    for e in events {
        match e {
            SampleEvent::Insert { t, at, text: s } => {
                if let Some(b) = burst.take() {
                    close(b, &text, &mut out, last_insert_t);
                }
                let at = (*at).min(text.len());
                for (i, c) in s.chars().enumerate() {
                    text.insert(at + i, c);
                }
                last_insert_t = Some(ms(*t));
            }
            SampleEvent::Delete { t, start, end } => {
                let t = ms(*t);
                if burst.is_some() && t - last_delete_t > 1500.0 {
                    let b = burst.take().expect("checked");
                    close(b, &text, &mut out, last_insert_t);
                }
                if burst.is_none() {
                    burst = Some((text.clone(), t));
                }
                last_delete_t = t;
                let (s, e) = ((*start).min(text.len()), (*end).min(text.len()));
                text.drain(s..e);
            }
            _ => {}
        }
    }
    if let Some(b) = burst.take() {
        close(b, &text, &mut out, last_insert_t);
    }
    out
}

/// What kind of mistake `wrong` (typed at the position after `kept`) was,
/// and how many characters were typed past it.
fn classify(kept: &[char], wrong: &[char], expected: &[char]) -> Option<(Mistake, usize)> {
    let k = wrong.iter().zip(expected).position(|(w, e)| w != e).or_else(|| {
        // Typed past the end of the prompt.
        (wrong.len() > expected.len()).then_some(expected.len())
    })?;
    let rest_matches = |from_w: usize, from_e: usize| {
        wrong.len() <= from_w
            || wrong[from_w..].iter().zip(expected.iter().skip(from_e)).all(|(a, b)| a == b)
                && expected.len().saturating_sub(from_e) >= wrong.len() - from_w
    };
    let w = wrong[k];
    let e = expected.get(k).copied();
    let e2 = expected.get(k + 1).copied();
    let prev = if k > 0 { Some(wrong[k - 1]) } else { kept.last().copied() };
    let after = wrong.len() - k - 1;

    if Some(w) == e2 && wrong.get(k + 1).copied() == e && rest_matches(k + 2, k + 2) {
        return Some((Mistake::Transpose, wrong.len() - k - 2));
    }
    if Some(w) == prev && rest_matches(k + 1, k) {
        return Some((Mistake::Double, after));
    }
    if Some(w) == e2 && rest_matches(k + 1, k + 2) {
        return Some((Mistake::Omission, after + 1));
    }
    if let Some(e) = e
        && w != e
        && w.eq_ignore_ascii_case(&e)
        && rest_matches(k + 1, k + 1)
    {
        return Some((Mistake::Case, after));
    }
    if rest_matches(k + 1, k + 1) {
        return Some((Mistake::Typo, after));
    }
    let mismatches = wrong.iter().zip(expected).filter(|(a, b)| a != b).count();
    if mismatches >= 2 && wrong.iter().all(|c| c.is_ascii_alphabetic()) {
        return Some((Mistake::WordRedo, 0));
    }
    Some((Mistake::Typo, after))
}

/// Mistakes left in the final text (Damerau–Levenshtein alignment).
fn unrepaired(prompt: &[char], typed: &[char]) -> HashMap<&'static str, usize> {
    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        Match,
        Sub,
        Del,
        Ins,
        Swap,
    }
    let (n, m) = (prompt.len(), typed.len());
    let mut cost = vec![vec![0usize; m + 1]; n + 1];
    let mut op = vec![vec![Op::Match; m + 1]; n + 1];
    for i in 1..=n {
        cost[i][0] = i;
        op[i][0] = Op::Del;
    }
    for j in 1..=m {
        cost[0][j] = j;
        op[0][j] = Op::Ins;
    }
    for i in 1..=n {
        for j in 1..=m {
            let same = prompt[i - 1] == typed[j - 1];
            let mut best = (cost[i - 1][j - 1] + usize::from(!same), if same { Op::Match } else { Op::Sub });
            if i > 1
                && j > 1
                && prompt[i - 1] == typed[j - 2]
                && prompt[i - 2] == typed[j - 1]
                && cost[i - 2][j - 2] + 1 < best.0
            {
                best = (cost[i - 2][j - 2] + 1, Op::Swap);
            }
            if cost[i - 1][j] + 1 < best.0 {
                best = (cost[i - 1][j] + 1, Op::Del);
            }
            if cost[i][j - 1] + 1 < best.0 {
                best = (cost[i][j - 1] + 1, Op::Ins);
            }
            cost[i][j] = best.0;
            op[i][j] = best.1;
        }
    }
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        match op[i][j] {
            Op::Match => {
                i -= 1;
                j -= 1;
            }
            Op::Sub => {
                let kind = if prompt[i - 1].eq_ignore_ascii_case(&typed[j - 1]) { "case" } else { "typo" };
                *counts.entry(kind).or_default() += 1;
                i -= 1;
                j -= 1;
            }
            Op::Swap => {
                *counts.entry("transpose").or_default() += 1;
                i -= 2;
                j -= 2;
            }
            Op::Del => {
                *counts.entry("omission").or_default() += 1;
                i -= 1;
            }
            Op::Ins => {
                // Either copy of a doubled letter may be the one aligned as extra.
                let double = (j >= 2 && typed[j - 1] == typed[j - 2]) || (j < m && typed[j - 1] == typed[j]);
                *counts.entry(if double { "double" } else { "typo" }).or_default() += 1;
                j -= 1;
            }
        }
    }
    counts
}

// ── The fit ────────────────────────────────────────────────────────────────

struct Builder {
    items: Vec<FitItem>,
}

impl Builder {
    fn add(&mut self, field: &'static str, value: Option<f64>, samples: usize, needed: usize) {
        if let Some(value) = value.filter(|v| v.is_finite()) {
            self.items.push(FitItem { field, value, samples, reliable: samples >= needed });
        }
    }
}

/// Analyses one typing sample of `prompt`. `base` provides every value the
/// sample cannot measure reliably, and is the reference for calibration.
pub fn fit(prompt: &str, events: &[SampleEvent], layout: &Arc<Layout>, base: HumanModel) -> FitReport {
    let base = base.sanitized();
    let prompt_chars: Vec<char> = normalize_newlines(prompt).chars().collect();
    let mut events: Vec<SampleEvent> = events.to_vec();
    events.sort_by_key(SampleEvent::time);
    let mut b = Builder { items: Vec::new() };

    // Keys: holds, rollover, Shift lead, backspace rhythm.
    let mut down: HashMap<KeyCode, f64> = HashMap::new();
    let (mut dwell, mut leads, mut backspace_gaps) = (Vec::new(), Vec::new(), Vec::new());
    let (mut presses, mut rollovers) = (0usize, 0usize);
    let mut shift_down_at: Option<f64> = None;
    let mut last_backspace: Option<f64> = None;
    for e in &events {
        match e {
            SampleEvent::KeyDown { t, key } => {
                let t = ms(*t);
                if down.contains_key(key) {
                    continue; // auto-repeat
                }
                let char_key = !key.is_modifier() && *key != KeyCode::BACKSPACE;
                if char_key {
                    presses += 1;
                    if down.keys().any(|k| !k.is_modifier() && *k != KeyCode::BACKSPACE) {
                        rollovers += 1;
                    }
                    if let Some(s) = shift_down_at.take()
                        && t - s < 400.0
                    {
                        leads.push(t - s);
                    }
                }
                if matches!(*key, KeyCode::LEFT_SHIFT | KeyCode::RIGHT_SHIFT) {
                    shift_down_at = Some(t);
                }
                if *key == KeyCode::BACKSPACE {
                    if let Some(l) = last_backspace
                        && t - l < 1000.0
                    {
                        backspace_gaps.push(t - l);
                    }
                    last_backspace = Some(t);
                } else if char_key {
                    last_backspace = None;
                }
                down.insert(*key, t);
            }
            SampleEvent::KeyUp { t, key } => {
                if let Some(start) = down.remove(key)
                    && !key.is_modifier()
                {
                    dwell.push(ms(*t) - start);
                }
            }
            _ => {}
        }
    }
    b.add("dwell_ms", median(&dwell), dwell.len(), 30);
    b.add("dwell_sigma", log_sigma(&dwell), dwell.len(), 30);
    b.add("modifier_lead_ms", median(&leads), leads.len(), 5);
    let backspace = median(&backspace_gaps);
    b.add("backspace_interval_ms", backspace, backspace_gaps.len(), 5);

    // Rhythm, calibrated against the model on the same prompt.
    let observed = intervals(&events, layout);
    let reference_model = HumanModel {
        typo_percent: 0.0,
        omission_percent: 0.0,
        transpose_percent: 0.0,
        double_press_percent: 0.0,
        case_slip_percent: 0.0,
        word_redo_percent: 0.0,
        fatigue_slowdown_percent: 0.0,
        fatigue_error_percent: 0.0,
        ..base
    };
    let mut reference = Intervals::default();
    for seed in 0..3 {
        let r = intervals(&simulate(prompt, reference_model, layout, 1000 + seed), layout);
        reference.flow.extend(r.flow);
        reference.same_finger.extend(r.same_finger);
        reference.same_hand.extend(r.same_hand);
        reference.alternate.extend(r.alternate);
        reference.bigram.extend(r.bigram);
        reference.familiar.extend(r.familiar);
        reference.unfamiliar.extend(r.unfamiliar);
    }
    let flow_obs = mean(&observed.flow);
    let flow_ref = mean(&reference.flow);
    let n_flow = observed.flow.len();
    b.add("wpm", flow_obs.zip(flow_ref).map(|(o, r)| base.wpm * r / o), n_flow, 40);
    let sigma = log_sigma(&observed.flow).zip(log_sigma(&reference.flow)).map(|(o, r)| {
        // Remove the spread the factors add on their own.
        let factor_var = (r * r - base.interval_sigma.powi(2)).max(0.0);
        (o * o - factor_var).max(0.0).sqrt()
    });
    b.add("interval_sigma", sigma, n_flow, 40);
    let median_flow = median(&observed.flow);
    let ratio = |cat: &[f64], all: Option<f64>| median(cat).zip(all).map(|(c, a)| c / a);
    let ref_flow = median(&reference.flow);
    for (field, obs, refc, base_value) in [
        ("same_finger_factor", &observed.same_finger, &reference.same_finger, base.same_finger_factor),
        ("same_hand_factor", &observed.same_hand, &reference.same_hand, base.same_hand_factor),
        ("alternate_hand_factor", &observed.alternate, &reference.alternate, base.alternate_hand_factor),
        ("common_bigram_factor", &observed.bigram, &reference.bigram, base.common_bigram_factor),
    ] {
        let v = ratio(obs, median_flow).zip(ratio(refc, ref_flow)).map(|(o, r)| base_value * o / r);
        b.add(field, v, obs.len(), 15);
    }
    let familiar = ratio(&observed.familiar, median(&observed.unfamiliar))
        .zip(ratio(&reference.familiar, median(&reference.unfamiliar)))
        .map(|(o, r)| base.familiar_word_factor * o / r);
    b.add("familiar_word_factor", familiar, observed.familiar.len().min(observed.unfamiliar.len()), 15);

    // Pauses: extra time on top of a normal interval.
    let extra =
        |gaps: &[f64], minus: f64| median(gaps).zip(median_flow).map(|(g, f)| (g - f - minus).max(0.0));
    let word_pause = extra(&observed.word_start, 0.0);
    b.add("word_pause_ms", word_pause, observed.word_start.len(), 10);
    let wp = word_pause.unwrap_or(base.word_pause_ms);
    b.add(
        "punctuation_pause_ms",
        extra(&observed.after_punctuation, 0.0),
        observed.after_punctuation.len(),
        3,
    );
    b.add("sentence_pause_ms", extra(&observed.sentence_start, wp), observed.sentence_start.len(), 3);
    b.add("paragraph_pause_ms", extra(&observed.after_newline, wp), observed.after_newline.len(), 2);
    let typed = typed_chars(&events);
    let long: Vec<f64> = typed
        .windows(2)
        .filter(|w| !w[1].after_delete && is_word_char(w[1].c) && !is_word_char(w[0].c))
        .map(|w| w[1].t - w[0].t)
        .filter(|g| *g > LONG_GAP_MS)
        .collect();
    let words = observed.words.max(1);
    b.add("long_pause_percent", Some(long.len() as f64 * 100.0 / words as f64), words, 40);
    b.add("long_pause_ms", median(&long).zip(median_flow).map(|(l, f)| l - f), long.len(), 2);

    // Mistakes.
    let fixed = corrections(&prompt_chars, &events);
    let mut final_text: Vec<char> = Vec::new();
    for e in &events {
        match e {
            SampleEvent::Insert { at, text, .. } => {
                let at = (*at).min(final_text.len());
                for (i, c) in text.chars().enumerate() {
                    final_text.insert(at + i, c);
                }
            }
            SampleEvent::Delete { start, end, .. } => {
                let (s, e) = ((*start).min(final_text.len()), (*end).min(final_text.len()));
                final_text.drain(s..e);
            }
            _ => {}
        }
    }
    // Only the part of the prompt that was reached counts.
    let reached = final_text.len().min(prompt_chars.len()).max(1);
    let left = unrepaired(&prompt_chars[..reached], &final_text[..final_text.len().min(reached + 2)]);
    let count = |kind: Mistake| fixed.fixed.iter().filter(|(k, _)| *k == kind).count();
    let total = |kind: Mistake, name: &str| count(kind) + left.get(name).copied().unwrap_or(0);
    let per = |n: usize, of: usize| n as f64 * 100.0 / of.max(1) as f64;
    let reached_prompt = &prompt_chars[..reached];
    let case_chars = reached_prompt
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            c.is_ascii_uppercase()
                || (c.is_ascii_lowercase() && *i > 0 && reached_prompt[i - 1].is_ascii_uppercase())
        })
        .count();
    let redo_words =
        reached_prompt.split(|c| !c.is_ascii_alphabetic()).filter(|w| w.len() >= 4).count().saturating_sub(1);
    // Each rate is per position where that mistake can happen (as in the
    // model): no wrong key on digits or spaces, no swap across a space, …
    let eligible = |allowed: &dyn Fn(usize, char) -> bool| {
        reached_prompt.iter().enumerate().filter(|&(i, &c)| allowed(i, c)).count()
    };
    let typo_pos = eligible(&|_, c| !c.is_ascii_digit() && !c.is_whitespace());
    let omit_pos = eligible(&|_, c| c != '\n');
    let swap_pos =
        eligible(&|i, c| !c.is_whitespace() && reached_prompt.get(i + 1).is_some_and(|n| !n.is_whitespace()));
    let long_enough = reached >= 200;
    let rate = |b: &mut Builder, field, n: usize, positions: usize| {
        b.add(field, Some(per(n, positions)), if long_enough { positions } else { 0 }, 1);
    };
    rate(&mut b, "typo_percent", total(Mistake::Typo, "typo"), typo_pos);
    rate(&mut b, "omission_percent", total(Mistake::Omission, "omission"), omit_pos);
    rate(&mut b, "transpose_percent", total(Mistake::Transpose, "transpose"), swap_pos);
    rate(&mut b, "double_press_percent", total(Mistake::Double, "double"), omit_pos);
    b.add(
        "case_slip_percent",
        Some(per(total(Mistake::Case, "case"), case_chars)),
        if case_chars >= 15 { case_chars } else { 0 },
        1,
    );
    b.add(
        "word_redo_percent",
        Some(per(count(Mistake::WordRedo), redo_words)),
        if redo_words >= 20 { redo_words } else { 0 },
        1,
    );

    let fixed_n = fixed.fixed.len();
    let left_n: usize = left.values().sum();
    b.add("repair_percent", Some(per(fixed_n, fixed_n + left_n)), fixed_n + left_n, 5);
    let notice =
        median(&fixed.notice_gaps).map(|g| (g - backspace.unwrap_or(base.backspace_interval_ms)).max(0.0));
    b.add("notice_delay_ms", notice, fixed.notice_gaps.len(), 5);
    let lookahead_cases: Vec<bool> = fixed
        .fixed
        .iter()
        .filter(|(k, _)| *k != Mistake::WordRedo)
        .map(|(k, la)| if *k == Mistake::Omission { *la >= 2 } else { *la >= 1 })
        .collect();
    b.add(
        "lookahead_percent",
        Some(per(lookahead_cases.iter().filter(|x| **x).count(), lookahead_cases.len())),
        lookahead_cases.len(),
        5,
    );

    let mut model = base;
    for item in b.items.iter().filter(|i| i.reliable) {
        item.apply(&mut model);
    }
    let model = model.sanitized();

    let duration = events.last().map(SampleEvent::time).unwrap_or_default();
    let minutes = duration.as_secs_f64() / 60.0;
    FitReport {
        items: b.items,
        model,
        gross_wpm: if minutes > 0.0 { final_text.len() as f64 / 5.0 / minutes } else { 0.0 },
        accuracy: (100.0 - per(left_n, reached)).max(0.0),
        duration,
        key_presses: presses,
        rollover_percent: per(rollovers, presses),
        mistakes: fixed_n + left_n,
        corrections: fixed_n,
    }
}

#[cfg(test)]
mod tests;
