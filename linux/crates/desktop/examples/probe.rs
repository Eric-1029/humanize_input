//! Starts the desktop services and prints what they report for a while.
//! Does not bind hotkeys (that shows a KDE dialog) unless `--bind` is given.
//!
//! ```sh
//! cargo run -p humanize-desktop --example probe -- [SECONDS] [--bind]
//! ```

use std::time::{Duration, Instant};

use humanize_desktop::{Desktop, DesktopOptions, HotkeyAction, HotkeyRequest, TrayLabels};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(5);
    let desktop =
        Desktop::start(DesktopOptions { tray: Some(TrayLabels::default()), switch_input_method: true })?;
    for w in desktop.warnings() {
        println!("warning: {w}");
    }
    println!("focus detection: {}", desktop.has_focus_detection());
    println!("hotkeys available: {}", desktop.has_hotkeys());
    println!("layout: {}", desktop.active_layout().label());
    if args.iter().any(|a| a == "--bind") {
        let requests: Vec<HotkeyRequest> = HotkeyAction::ALL
            .iter()
            .zip(["CTRL+ALT+S", "CTRL+ALT+P", "CTRL+ALT+Q"])
            .map(|(&action, preferred)| HotkeyRequest {
                action,
                description: format!("{action:?}"),
                preferred: preferred.into(),
            })
            .collect();
        println!("bound: {:?}", desktop.bind_hotkeys(&requests));
    }
    let events = desktop.events();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    std::thread::sleep(Duration::from_millis(300));
    println!("active window: {:?}", desktop.active_window());
    while Instant::now() < deadline {
        if let Ok(event) = events.try_recv() {
            println!("event: {event:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
