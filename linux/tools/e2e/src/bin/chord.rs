//! Presses one key chord on a fresh virtual keyboard, e.g. `chord ctrl+alt+s`.
//! Used by end-to-end checks to trigger the app's global hotkeys.

use std::time::Duration;

use humanize_core::{Driver, KeyCode};
use humanize_input::{DeviceIdentity, UinputKeyboard};

fn key(name: &str) -> Option<KeyCode> {
    Some(match name {
        "ctrl" => KeyCode::LEFT_CTRL,
        "alt" => KeyCode::LEFT_ALT,
        "shift" => KeyCode::LEFT_SHIFT,
        "s" => KeyCode(31),
        "p" => KeyCode(25),
        "q" => KeyCode(16),
        _ => return None,
    })
}

fn main() -> Result<(), String> {
    let chord = std::env::args().nth(1).ok_or("usage: chord ctrl+alt+s")?;
    let keys: Vec<KeyCode> =
        chord.split('+').map(|k| key(k).ok_or(format!("unknown key {k}"))).collect::<Result<_, _>>()?;
    let mut keyboard = UinputKeyboard::create(&DeviceIdentity {
        name: "humanize chord".into(),
        ..DeviceIdentity::default()
    })
    .map_err(|e| e.to_string())?;
    for &k in &keys {
        keyboard.key(k, true).map_err(|e| e.0)?;
        std::thread::sleep(Duration::from_millis(40));
    }
    std::thread::sleep(Duration::from_millis(60));
    for &k in keys.iter().rev() {
        keyboard.key(k, false).map_err(|e| e.0)?;
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}
