//! Presses the three hotkeys on the virtual keyboard and checks that the
//! GlobalShortcuts portal delivers them: KWin → kglobalaccel → portal → us.
//!
//! ```sh
//! cargo run -p humanize-e2e --bin hotkey_check
//! ```

use std::time::{Duration, Instant};

use humanize_core::{Driver, KeyCode};
use humanize_desktop::{Desktop, DesktopEvent, DesktopOptions, HotkeyAction, HotkeyRequest};
use humanize_input::{DeviceIdentity, UinputKeyboard};

fn chord(keyboard: &mut UinputKeyboard, letter: KeyCode) -> Result<(), String> {
    let keys = [KeyCode::LEFT_CTRL, KeyCode::LEFT_ALT, letter];
    for &k in &keys {
        keyboard.key(k, true).map_err(|e| e.0)?;
        std::thread::sleep(Duration::from_millis(40));
    }
    std::thread::sleep(Duration::from_millis(60));
    for &k in keys.iter().rev() {
        keyboard.key(k, false).map_err(|e| e.0)?;
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let desktop = Desktop::start(DesktopOptions { tray: None, switch_input_method: false })?;
    let requests: Vec<HotkeyRequest> = HotkeyAction::ALL
        .iter()
        .zip(["CTRL+ALT+S", "CTRL+ALT+P", "CTRL+ALT+Q"])
        .map(|(&action, preferred)| HotkeyRequest {
            action,
            description: format!("{action:?}"),
            preferred: preferred.into(),
        })
        .collect();
    let bound = desktop.bind_hotkeys(&requests)?;
    println!("bound: {bound:?}");
    let mut keyboard = UinputKeyboard::create(&DeviceIdentity::default()).map_err(|e| e.to_string())?;
    let events = desktop.events();

    let mut received = Vec::new();
    for (letter, expected) in [
        (KeyCode(31), HotkeyAction::Start),
        (KeyCode(25), HotkeyAction::PauseResume),
        (KeyCode(16), HotkeyAction::Cancel),
    ] {
        chord(&mut keyboard, letter)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(DesktopEvent::Hotkey(action)) = events.try_recv() {
                received.push(action);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        println!("{expected:?}: {}", if received.last() == Some(&expected) { "received" } else { "MISSING" });
    }
    if received == HotkeyAction::ALL {
        println!("PASS");
        Ok(())
    } else {
        Err(format!("FAIL: received {received:?}"))
    }
}
