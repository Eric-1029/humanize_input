//! Replays a key trace (HUMANIZE_INPUT_TRACE output) through the virtual
//! editor and prints the resulting text and any physical-validity problems.

use std::sync::Arc;
use std::time::Duration;

use humanize_core::{KeyAction, KeyCode, VirtualEditor};

fn main() -> Result<(), String> {
    let path = std::env::args().nth(1).ok_or("usage: replay TRACE")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let (layout, _) = humanize_keymap::system_layout().map_err(|e| e.to_string())?;
    let mut editor = VirtualEditor::new(Arc::new(layout));
    let mut last = 0u64;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(t), Some(code), Some(down)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let t: u64 = t.parse().map_err(|_| format!("bad line {line}"))?;
        editor.apply(KeyAction::Wait(Duration::from_micros(t.saturating_sub(last))));
        last = t;
        let key = KeyCode(code.parse().map_err(|_| format!("bad line {line}"))?);
        editor.apply(if down == "1" { KeyAction::Press(key) } else { KeyAction::Release(key) });
    }
    println!("{}", editor.text());
    for v in editor.violations() {
        eprintln!("violation: {v}");
    }
    Ok(())
}
