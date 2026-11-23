//! End-to-end check of the typing test page: starts the app on the Test
//! page, types sample 1 on the virtual keyboard with a known model, and
//! compares the values the page measured with the model's.
//!
//! Keys go only to the app's window: typing waits whenever KWin reports
//! another window as active.
//!
//! ```sh
//! cargo build -p humanize-input-app && cargo run -p humanize-e2e --bin detector_check
//! ```

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use humanize_core::fit::SAMPLES;
use humanize_core::{HumanModel, Preset, Session, SessionEvent, SessionState, TypingMode, WordDelete};
use humanize_desktop::{Desktop, DesktopOptions};
use humanize_input::{DeviceIdentity, UinputKeyboard};

fn main() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("humanize-detector-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let report_path = dir.join("report.txt");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app_bin = exe.parent().map(|d| d.join("humanize_input")).ok_or("no target dir")?;

    let mut app = std::process::Command::new(&app_bin)
        .args(["--offline", "--page", "test", "--theme", "light"])
        .env("GTK_IM_MODULE", "simple")
        .env("HUMANIZE_INPUT_TEST_REPORT", &report_path)
        .env("XDG_CONFIG_HOME", &dir)
        .spawn()
        .map_err(|e| format!("{}: {e}", app_bin.display()))?;
    let app_pid = app.id();

    let desktop = Desktop::start(DesktopOptions { tray: None, switch_input_method: false })?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while desktop.active_window().is_none_or(|w| w.pid != app_pid) {
        if Instant::now() > deadline {
            let _ = app.kill();
            return Err("the app window never became active; nothing was typed".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(800));

    let truth = HumanModel {
        wpm: 62.0,
        dwell_ms: 112.0,
        modifier_lead_ms: 70.0,
        word_pause_ms: 70.0,
        typo_percent: 3.0,
        omission_percent: 1.0,
        repair_percent: 100.0,
        backspace_interval_ms: 125.0,
        word_redo_percent: 0.0,
        word_delete: WordDelete::Backspaces,
        long_pause_percent: 0.0,
        fatigue_slowdown_percent: 0.0,
        fatigue_error_percent: 0.0,
        lead_in_ms: 0,
        ..Preset::Average.model()
    };
    let (layout, _) = humanize_keymap::system_layout().map_err(|e| e.to_string())?;
    let mut keyboard = UinputKeyboard::create(&DeviceIdentity::default()).map_err(|e| e.to_string())?;

    // Regression check: type a little, move focus to "Start over" with
    // Ctrl+Tab and press it with Space. The reset used to panic here.
    {
        use humanize_core::{Driver, KeyCode};
        let tap = |kb: &mut UinputKeyboard, keys: &[KeyCode]| -> Result<(), String> {
            for &k in keys {
                kb.key(k, true).map_err(|e| e.0)?;
                std::thread::sleep(Duration::from_millis(40));
            }
            for &k in keys.iter().rev() {
                kb.key(k, false).map_err(|e| e.0)?;
                std::thread::sleep(Duration::from_millis(40));
            }
            std::thread::sleep(Duration::from_millis(120));
            Ok(())
        };
        for code in [35u16, 18, 38, 38, 24] {
            tap(&mut keyboard, &[KeyCode(code)])?; // h e l l o
        }
        tap(&mut keyboard, &[KeyCode::LEFT_CTRL, KeyCode::TAB])?;
        tap(&mut keyboard, &[KeyCode::SPACE])?;
        std::thread::sleep(Duration::from_millis(800));
        if app.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("FAIL: the app exited when Start over was pressed".into());
        }
        println!("Start over after typing: app still running");
    }
    let (tx, rx) = mpsc::channel();
    let session = Session::new(
        Box::new(keyboard),
        desktop.focus_tracker(),
        Arc::new(layout),
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    );
    session.start_seeded(SAMPLES[0], TypingMode::Human(truth), 5).map_err(|e| e.to_string())?;
    println!("typing sample 1 ({} chars) into the Test page…", SAMPLES[0].len());
    loop {
        match rx.recv_timeout(Duration::from_secs(300)).map_err(|e| e.to_string())? {
            SessionEvent::State(SessionState::Completed) => break,
            SessionEvent::State(SessionState::Idle | SessionState::Faulted) => {
                let _ = app.kill();
                return Err("typing stopped early".into());
            }
            _ => {}
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !report_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = app.kill();
    let report = std::fs::read_to_string(&report_path).map_err(|e| format!("no report: {e}"))?;
    let _ = std::fs::remove_dir_all(&dir);

    let value = |field: &str| -> Option<f64> {
        let i = report.find(&format!("field: \"{field}\""))?;
        let rest = &report[i..];
        let v = rest.find("value: ")? + 7;
        rest[v..].split([',', '\n']).next()?.trim().parse().ok()
    };
    let mut ok = true;
    for (field, want, tol) in [
        ("wpm", truth.wpm, 0.15),
        ("dwell_ms", truth.dwell_ms, 0.20),
        ("modifier_lead_ms", truth.modifier_lead_ms, 0.35),
        ("backspace_interval_ms", truth.backspace_interval_ms, 0.35),
        ("repair_percent", truth.repair_percent, 0.10),
    ] {
        let got = value(field);
        let good = got.is_some_and(|g| (g - want).abs() / want <= tol);
        ok &= good;
        println!(
            "{:<24} true {:>7.1}   measured {:>7}   {}",
            field,
            want,
            got.map_or("-".into(), |g| format!("{g:.1}")),
            if good { "ok" } else { "OFF" }
        );
    }
    if ok {
        println!("PASS");
        Ok(())
    } else {
        Err("FAIL: measured values too far off".into())
    }
}
