//! Groups key events into USB report frames.
//!
//! A USB keyboard sends one report per polling interval. The kernel turns a
//! report into events in a fixed order: modifier changes first (they are a
//! bitmap byte at the start of the report), then released keys, then newly
//! pressed keys, followed by one `SYN_REPORT`. A report also cannot show a key
//! going down and up again. We batch events the same way, starting a new
//! frame whenever merging would change what the target sees.

use std::collections::VecDeque;

use humanize_core::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub down: bool,
}

/// Takes the longest prefix of `pending` that fits in one report, in the
/// order the kernel would emit it.
pub fn take_frame(pending: &mut VecDeque<KeyEvent>) -> Vec<KeyEvent> {
    let mut frame: Vec<KeyEvent> = Vec::new();
    while let Some(&event) = pending.front() {
        // One state change per key and report.
        let same_key = frame.iter().any(|e| e.key == event.key);
        // Modifiers are reported first; one arriving after a key press would
        // jump ahead of it and change that key's character.
        let modifier_after_press =
            event.key.is_modifier() && frame.iter().any(|e| e.down && !e.key.is_modifier());
        if same_key || modifier_after_press {
            break;
        }
        frame.push(event);
        pending.pop_front();
    }
    // Stable: modifiers, then releases, then presses.
    frame.sort_by_key(|e| (!e.key.is_modifier(), e.down));
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: KeyCode = KeyCode(30);
    const B: KeyCode = KeyCode(48);
    const SHIFT: KeyCode = KeyCode::LEFT_SHIFT;

    fn ev(key: KeyCode, down: bool) -> KeyEvent {
        KeyEvent { key, down }
    }

    fn frames(events: &[KeyEvent]) -> Vec<Vec<KeyEvent>> {
        let mut pending: VecDeque<KeyEvent> = events.iter().copied().collect();
        let mut out = Vec::new();
        while !pending.is_empty() {
            out.push(take_frame(&mut pending));
        }
        out
    }

    #[test]
    fn instant_shifted_stroke_needs_two_reports() {
        // The classic driver sends Shift↓ A↓ A↑ Shift↑ with no delay.
        let out = frames(&[ev(SHIFT, true), ev(A, true), ev(A, false), ev(SHIFT, false)]);
        assert_eq!(out, vec![vec![ev(SHIFT, true), ev(A, true)], vec![ev(SHIFT, false), ev(A, false)]]);
    }

    #[test]
    fn rollover_merges_release_and_press() {
        let out = frames(&[ev(A, false), ev(B, true)]);
        assert_eq!(out, vec![vec![ev(A, false), ev(B, true)]]);
        // Press listed first still comes out after the release.
        let out = frames(&[ev(B, true), ev(A, false)]);
        assert_eq!(out, vec![vec![ev(A, false), ev(B, true)]]);
    }

    #[test]
    fn modifier_change_never_jumps_ahead_of_a_press() {
        let out = frames(&[ev(A, true), ev(SHIFT, false)]);
        assert_eq!(out, vec![vec![ev(A, true)], vec![ev(SHIFT, false)]]);
    }

    #[test]
    fn modifier_after_a_release_may_share_the_report() {
        let out = frames(&[ev(A, false), ev(SHIFT, true), ev(B, true)]);
        assert_eq!(out, vec![vec![ev(SHIFT, true), ev(A, false), ev(B, true)]]);
    }
}
