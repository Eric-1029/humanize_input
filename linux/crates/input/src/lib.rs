//! Key injection for humanize_input.
//!
//! - [`UinputKeyboard`]: a virtual USB keyboard (uinput) that sends HID scan
//!   codes and groups events into USB-polling-rate reports.
//! - [`DryRunDriver`]: records events without touching the system.
//! - [`caps_lock_on`]: Caps Lock would invert letter case, so check first.

mod frame;
mod hid;
mod keyboard;
mod uinput;

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use humanize_core::{Driver, DriverError, KeyCode};

pub use keyboard::{DeviceIdentity, UinputKeyboard};

/// Whether any keyboard's Caps Lock LED is lit. The lock state is shared by
/// all keyboards, so typing now would invert letter case.
pub fn caps_lock_on() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/leds") else { return false };
    entries.flatten().filter(|e| e.file_name().to_string_lossy().ends_with("::capslock")).any(|e| {
        std::fs::read_to_string(e.path().join("brightness"))
            .is_ok_and(|v| v.trim().parse::<u32>().is_ok_and(|b| b > 0))
    })
}

/// One recorded event: time since the driver was created, key, direction.
pub type RecordedEvent = (Duration, KeyCode, bool);

/// A driver that only records (and optionally prints) what it would send.
#[derive(Clone)]
pub struct DryRunDriver {
    start: Instant,
    events: Arc<Mutex<Vec<RecordedEvent>>>,
    print: bool,
}

impl DryRunDriver {
    pub fn new(print: bool) -> Self {
        Self { start: Instant::now(), events: Arc::new(Mutex::new(Vec::new())), print }
    }

    /// Everything sent so far (shared between clones).
    pub fn events(&self) -> Vec<RecordedEvent> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Driver for DryRunDriver {
    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
        let at = self.start.elapsed();
        if self.print {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(
                out,
                "{:>9.3}s  {} {:>3}",
                at.as_secs_f64(),
                if down { "down" } else { "up  " },
                key.0
            );
        }
        self.events.lock().unwrap_or_else(PoisonError::into_inner).push((at, key, down));
        Ok(())
    }
}
