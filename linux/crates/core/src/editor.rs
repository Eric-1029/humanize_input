//! A simulated text field that consumes key events, for tests and dry runs.
//!
//! It reproduces what a typical editor does with raw key events, and records
//! anything a real keyboard could not do or that would misbehave on a real
//! desktop (pressing a held key, releasing an unpressed one, holding a
//! character key long enough to trigger auto-repeat).

use std::sync::Arc;
use std::time::Duration;

use crate::fit::SampleEvent;
use crate::keys::{KeyAction, KeyCode, Mods};
use crate::layout::Layout;

/// KDE's default auto-repeat delay is 600 ms; stay well clear of it.
pub const MAX_SAFE_HOLD: Duration = Duration::from_millis(300);

#[derive(Debug, Clone)]
pub struct VirtualEditor {
    layout: Arc<Layout>,
    text: String,
    /// Held keys with the time they went down.
    held: Vec<(KeyCode, Duration)>,
    now: Duration,
    violations: Vec<String>,
    longest_hold: Duration,
    rollovers: usize,
    presses: usize,
    /// When recording, every key and text change, as a GUI would see them.
    sample: Option<Vec<SampleEvent>>,
}

impl VirtualEditor {
    pub fn new(layout: Arc<Layout>) -> Self {
        Self {
            layout,
            text: String::new(),
            held: Vec::new(),
            now: Duration::ZERO,
            violations: Vec::new(),
            longest_hold: Duration::ZERO,
            rollovers: 0,
            presses: 0,
            sample: None,
        }
    }

    /// Also records key and text events, in the form the detector analyses.
    pub fn recording(mut self) -> Self {
        self.sample = Some(Vec::new());
        self
    }

    /// Recorded events (empty unless created with [`recording`](Self::recording)).
    pub fn sample_events(&self) -> &[SampleEvent] {
        self.sample.as_deref().unwrap_or_default()
    }

    fn record(&mut self, event: SampleEvent) {
        if let Some(sample) = &mut self.sample {
            sample.push(event);
        }
    }

    fn push_char(&mut self, c: char) {
        let at = self.text.chars().count();
        self.text.push(c);
        self.record(SampleEvent::Insert { t: self.now, at, text: c.to_string() });
    }

    /// Removes the last `n` characters as one edit.
    fn remove_chars(&mut self, n: usize) {
        let len = self.text.chars().count();
        let n = n.min(len);
        if n == 0 {
            return;
        }
        for _ in 0..n {
            self.text.pop();
        }
        self.record(SampleEvent::Delete { t: self.now, start: len - n, end: len });
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Problems found so far; empty means every event was physically valid.
    pub fn violations(&self) -> &[String] {
        &self.violations
    }

    /// Keys currently down.
    pub fn held(&self) -> Vec<KeyCode> {
        self.held.iter().map(|&(k, _)| k).collect()
    }

    /// Longest time any non-modifier key was held.
    pub fn longest_hold(&self) -> Duration {
        self.longest_hold
    }

    /// Presses of a character key while another character key was still down.
    pub fn rollovers(&self) -> usize {
        self.rollovers
    }

    pub fn presses(&self) -> usize {
        self.presses
    }

    pub fn elapsed(&self) -> Duration {
        self.now
    }

    pub fn apply_all(&mut self, actions: impl IntoIterator<Item = KeyAction>) {
        for action in actions {
            self.apply(action);
        }
    }

    pub fn apply(&mut self, action: KeyAction) {
        match action {
            KeyAction::Wait(d) => self.now += d,
            KeyAction::Progress(_) => {}
            KeyAction::Press(key) => self.press(key),
            KeyAction::Release(key) => self.release(key),
        }
    }

    fn mods(&self) -> Mods {
        self.held.iter().filter_map(|&(k, _)| k.modifier_flag()).fold(Mods::NONE, |a, b| a | b)
    }

    fn press(&mut self, key: KeyCode) {
        if self.held.iter().any(|&(k, _)| k == key) {
            self.violations.push(format!("{key:?} pressed while already down at {:?}", self.now));
            return;
        }
        if !key.is_modifier() {
            self.presses += 1;
            if self.held.iter().any(|&(k, _)| !k.is_modifier()) {
                self.rollovers += 1;
            }
        }
        self.held.push((key, self.now));
        self.record(SampleEvent::KeyDown { t: self.now, key });
        if key.is_modifier() {
            return;
        }

        let mods = self.mods();
        if mods.contains(Mods::CTRL) {
            if key == KeyCode::BACKSPACE {
                self.delete_word();
            } else {
                self.violations.push(format!("unexpected Ctrl+{key:?}"));
            }
            return;
        }
        match key {
            KeyCode::BACKSPACE => {
                self.remove_chars(1);
            }
            _ => match self.layout.char_for(key, mods) {
                Some(c) => self.push_char(c),
                None => self.violations.push(format!("{key:?} with {mods:?} produces nothing")),
            },
        }
    }

    fn release(&mut self, key: KeyCode) {
        let Some(pos) = self.held.iter().position(|&(k, _)| k == key) else {
            self.violations.push(format!("{key:?} released while up at {:?}", self.now));
            return;
        };
        let (_, since) = self.held.remove(pos);
        self.record(SampleEvent::KeyUp { t: self.now, key });
        if !key.is_modifier() {
            let hold = self.now - since;
            self.longest_hold = self.longest_hold.max(hold);
            if hold > MAX_SAFE_HOLD {
                self.violations.push(format!("{key:?} held {hold:?}, would auto-repeat"));
            }
        }
    }

    /// Ctrl+Backspace as GTK/Qt editors do it: skip trailing whitespace,
    /// then delete one run of word characters (or of punctuation).
    fn delete_word(&mut self) {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = chars.len();
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        if i > 0 && is_word(chars[i - 1]) {
            while i > 0 && is_word(chars[i - 1]) {
                i -= 1;
            }
        } else {
            while i > 0 && !is_word(chars[i - 1]) && !chars[i - 1].is_whitespace() {
                i -= 1;
            }
        }
        self.remove_chars(chars.len() - i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyAction::*;

    fn editor() -> VirtualEditor {
        VirtualEditor::new(Arc::new(Layout::us_qwerty()))
    }

    const A: KeyCode = KeyCode(30);
    const B: KeyCode = KeyCode(48);

    #[test]
    fn shift_rollover_and_backspace() {
        let mut e = editor();
        e.apply_all([
            Press(KeyCode::LEFT_SHIFT),
            Press(A),
            Release(KeyCode::LEFT_SHIFT),
            Press(B), // 'a' still down: rollover, and Shift is already up
            Release(A),
            Release(B),
            Press(KeyCode::BACKSPACE),
            Release(KeyCode::BACKSPACE),
        ]);
        assert_eq!(e.text(), "A");
        assert_eq!(e.rollovers(), 1);
        assert!(e.violations().is_empty(), "{:?}", e.violations());
    }

    #[test]
    fn ctrl_backspace_deletes_previous_word() {
        let mut e = editor();
        for c in "say hello  ".chars() {
            let s = e.layout.stroke(c).unwrap();
            e.apply_all([Press(s.key), Release(s.key)]);
        }
        e.apply_all([
            Press(KeyCode::LEFT_CTRL),
            Press(KeyCode::BACKSPACE),
            Release(KeyCode::BACKSPACE),
            Release(KeyCode::LEFT_CTRL),
        ]);
        assert_eq!(e.text(), "say ");
    }

    #[test]
    fn detects_impossible_and_unsafe_events() {
        let mut e = editor();
        e.apply_all([Press(A), Press(A), Release(B), Wait(Duration::from_millis(400)), Release(A)]);
        assert_eq!(e.violations().len(), 3, "{:?}", e.violations());
    }
}
