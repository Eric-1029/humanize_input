//! Types text into the focused window through the virtual keyboard.
//!
//! ```sh
//! cargo run -p humanize-input --example type -- [--dry-run] [--classic | --preset NAME] [--lead-in MS] TEXT
//! ```
//!
//! Focus the target window during the lead-in (default 3 s). Ctrl+C stops it;
//! held keys are released when the process exits.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use humanize_core::{
    AlwaysFocused, Driver, HumanModel, LogEvent, Preset, Session, SessionEvent, SessionState, TypingMode,
    TypingSettings,
};
use humanize_input::{DeviceIdentity, DryRunDriver, UinputKeyboard, caps_lock_on};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dry_run = false;
    let mut classic = false;
    let mut preset = Preset::Average;
    let mut lead_in = 3000u32;
    let mut text = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--classic" => classic = true,
            "--preset" => {
                preset = match args.next().as_deref() {
                    Some("novice") => Preset::Novice,
                    Some("proficient") => Preset::Proficient,
                    Some("professional") => Preset::Professional,
                    _ => Preset::Average,
                }
            }
            "--lead-in" => lead_in = args.next().and_then(|v| v.parse().ok()).unwrap_or(lead_in),
            _ => text = Some(arg),
        }
    }
    let text = text.ok_or("usage: type [--dry-run] [--classic | --preset NAME] [--lead-in MS] TEXT")?;

    let (layout, names) = humanize_keymap::system_layout()?;
    eprintln!("layout: {}", names.label());
    if !dry_run && caps_lock_on() {
        return Err("Caps Lock is on; turn it off first".into());
    }

    let driver: Box<dyn Driver> = if dry_run {
        Box::new(DryRunDriver::new(true))
    } else {
        Box::new(UinputKeyboard::create(&DeviceIdentity::default())?)
    };
    let (tx, rx) = mpsc::channel();
    let session = Session::new(
        driver,
        Arc::new(AlwaysFocused),
        Arc::new(layout),
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    );

    let mode = if classic {
        TypingMode::Classic(TypingSettings { lead_in_delay_ms: lead_in, ..TypingSettings::default() })
    } else {
        TypingMode::Human(HumanModel { lead_in_ms: lead_in, ..preset.model() })
    };
    session.start(&text, mode)?;
    eprintln!("typing in {:.1}s — focus the target window now", f64::from(lead_in) / 1000.0);

    loop {
        match rx.recv_timeout(Duration::from_secs(600))? {
            SessionEvent::Progress(p) => eprint!("\r{}/{} chars", p.typed_chars, p.total_chars),
            SessionEvent::Log(LogEvent::DriverFailed(e)) => return Err(e.into()),
            SessionEvent::State(SessionState::Completed | SessionState::Idle | SessionState::Faulted) => {
                break;
            }
            _ => {}
        }
    }
    eprintln!("\ndone");
    Ok(())
}
