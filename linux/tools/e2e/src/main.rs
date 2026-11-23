//! End-to-end check on a real desktop: opens its own window, types into it
//! through the virtual USB keyboard and compares what arrived.
//!
//! ```sh
//! cargo run -p humanize-e2e -- [--classic | --preset NAME] [--no-mistakes] [--word-redo PCT] [--seed N] [--with-ime] [TEXT]
//! ```
//!
//! Safety: keys are only sent while this window is the active one. If it
//! loses focus, typing pauses; after 5 s without focus the run is cancelled.
//! By default the window bypasses the input method (GTK_IM_MODULE=simple) so
//! an active fcitx5/IBus cannot swallow the keys; `--with-ime` keeps it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};
use humanize_core::{
    FocusTracker, HumanModel, LogEvent, Preset, Session, SessionEvent, SessionState, TypingMode,
    TypingSettings, WindowId,
};
use humanize_desktop::{Desktop, DesktopOptions};
use humanize_input::{DeviceIdentity, UinputKeyboard, caps_lock_on};

const SAMPLE: &str = "The Quick brown fox, said HELLO to the lazy dog. Then it left!\n\
    fn main() { println!(\"x = {}\", 42 * 7); }\n\tIndented: a_b-c+d=e [f] <g> ~h| 'i' `j`?";

struct WindowActive(Arc<AtomicBool>);

impl FocusTracker for WindowActive {
    fn current(&self) -> Option<WindowId> {
        let id = if self.0.load(Ordering::SeqCst) { "e2e" } else { "elsewhere" };
        Some(WindowId(id.into()))
    }
}

struct Options {
    mode: TypingMode,
    seed: Option<u64>,
    text: String,
    with_ime: bool,
    /// Attach the desktop services (KWin focus, fcitx5 switching).
    desktop: bool,
    /// Switch fcitx5 to Chinese mode in the test window before typing, to
    /// prove the session turns it off again (with --desktop). Implies --with-ime.
    ime_on: bool,
}

fn parse_args() -> Options {
    let mut mode = TypingMode::Human(HumanModel { lead_in_ms: 0, ..Preset::Professional.model() });
    let mut seed = None;
    let mut text = SAMPLE.to_owned();
    let mut with_ime = false;
    let mut desktop = false;
    let mut ime_on = false;
    let mut no_mistakes = false;
    let mut word_redo: Option<f64> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--classic" => {
                mode =
                    TypingMode::Classic(TypingSettings { lead_in_delay_ms: 0, ..TypingSettings::default() })
            }
            "--preset" => {
                let preset = match args.next().as_deref() {
                    Some("novice") => Preset::Novice,
                    Some("average") => Preset::Average,
                    Some("proficient") => Preset::Proficient,
                    _ => Preset::Professional,
                };
                mode = TypingMode::Human(HumanModel { lead_in_ms: 0, ..preset.model() });
            }
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()),
            "--with-ime" => with_ime = true,
            "--desktop" => desktop = true,
            "--ime-on" => {
                ime_on = true;
                with_ime = true;
            }
            "--no-mistakes" => no_mistakes = true,
            "--word-redo" => word_redo = args.next().and_then(|s| s.parse().ok()),
            _ => text = arg,
        }
    }
    // Every mistake must be fixed, or the comparison cannot pass.
    if let TypingMode::Human(m) = &mut mode {
        m.repair_percent = 100.0;
        if no_mistakes {
            for rate in [
                &mut m.typo_percent,
                &mut m.omission_percent,
                &mut m.transpose_percent,
                &mut m.double_press_percent,
                &mut m.case_slip_percent,
                &mut m.word_redo_percent,
            ] {
                *rate = 0.0;
            }
        }
        if let Some(pct) = word_redo {
            m.word_redo_percent = pct;
        }
    } else if let TypingMode::Classic(s) = &mut mode {
        s.repair_rate_percent = 100;
    }
    Options { mode, seed, text, with_ime, desktop, ime_on }
}

fn main() -> glib::ExitCode {
    let options = parse_args();
    if !options.with_ime {
        // SAFETY: single-threaded at this point; GTK reads it during init.
        unsafe { std::env::set_var("GTK_IM_MODULE", "simple") };
    }
    if caps_lock_on() {
        eprintln!("FAIL: Caps Lock is on");
        return glib::ExitCode::FAILURE;
    }
    let (layout, names) = match humanize_keymap::system_layout() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    eprintln!("layout: {}", names.label());
    let keyboard = match UinputKeyboard::create(&DeviceIdentity::default()) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return glib::ExitCode::FAILURE;
        }
    };

    let desktop = if options.desktop {
        match Desktop::start(DesktopOptions { tray: None, switch_input_method: true }) {
            Ok(d) => {
                for w in d.warnings() {
                    eprintln!("desktop warning: {w}");
                }
                Some(Rc::new(d))
            }
            Err(e) => {
                eprintln!("FAIL: {e}");
                return glib::ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    let active = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let mut session = Session::new(
        Box::new(keyboard),
        Arc::new(WindowActive(Arc::clone(&active))),
        Arc::new(layout),
        Arc::new(move |e| {
            let _ = tx.send(e);
        }),
    );
    if let Some(d) = &desktop {
        session = session.with_hooks(d.session_hooks());
    }
    let session = Rc::new(session);
    let rx = Rc::new(rx);
    let passed = Rc::new(Cell::new(false));
    let options = Rc::new(options);

    let app = gtk::Application::builder()
        .application_id("dev.humanize_input.E2E")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    {
        let passed = Rc::clone(&passed);
        app.connect_activate(move |app| {
            build(app, &active, &session, &rx, &options, &passed, desktop.clone());
        });
    }
    app.run_with_args::<&str>(&[]);
    if passed.get() { glib::ExitCode::SUCCESS } else { glib::ExitCode::FAILURE }
}

fn build(
    app: &gtk::Application,
    active: &Arc<AtomicBool>,
    session: &Rc<Session>,
    rx: &Rc<mpsc::Receiver<SessionEvent>>,
    options: &Rc<Options>,
    passed: &Rc<Cell<bool>>,
    desktop: Option<Rc<Desktop>>,
) {
    let view = gtk::TextView::builder().monospace(true).hexpand(true).vexpand(true).build();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("humanize_input end-to-end test")
        .default_width(900)
        .default_height(420)
        .child(&view)
        .build();
    {
        let active = Arc::clone(active);
        window.connect_is_active_notify(move |w| active.store(w.is_active(), Ordering::SeqCst));
    }
    window.present();
    view.grab_focus();

    let session = Rc::clone(session);
    let rx = Rc::clone(rx);
    let options = Rc::clone(options);
    let passed = Rc::clone(passed);
    let window_for_start = window.clone();
    glib::timeout_add_local_once(Duration::from_millis(1500), move || {
        if !window_for_start.is_active() {
            eprintln!("FAIL: the test window did not get focus; nothing was typed");
            window_for_start.close();
            return;
        }
        if options.ime_on {
            let _ = std::process::Command::new("fcitx5-remote").arg("-o").status();
            std::thread::sleep(Duration::from_millis(200));
            eprintln!("fcitx5 state before typing: {}", fcitx_state());
        }
        let started = match options.seed {
            Some(seed) => session.start_seeded(&options.text, options.mode, seed),
            None => session.start(&options.text, options.mode),
        };
        if let Err(e) = started {
            eprintln!("FAIL: {e}");
            window_for_start.close();
            return;
        }
        let begun = Instant::now();
        let unfocused_since: Rc<RefCell<Option<Instant>>> = Rc::new(RefCell::new(None));
        glib::timeout_add_local(Duration::from_millis(30), move || {
            for event in rx.try_iter() {
                match event {
                    SessionEvent::Progress(p) => eprint!("\r{}/{} chars", p.typed_chars, p.total_chars),
                    SessionEvent::Log(LogEvent::DriverFailed(e)) => eprintln!("\ndriver failed: {e}"),
                    SessionEvent::State(SessionState::Completed) => {
                        let buffer = view.buffer();
                        let typed = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                        eprintln!("\nelapsed: {:.1}s", begun.elapsed().as_secs_f64());
                        if let Some(d) = &desktop {
                            let own = d.active_window().is_some_and(|w| w.pid == std::process::id());
                            eprintln!("KWin sees the test window as active: {own}");
                            eprintln!("fcitx5 state after typing: {}", fcitx_state());
                        }
                        if typed == options.text {
                            eprintln!("PASS: {} chars arrived exactly", typed.chars().count());
                            passed.set(true);
                        } else {
                            eprintln!(
                                "FAIL: text differs\nexpected: {:?}\nreceived: {:?}",
                                options.text,
                                typed.as_str()
                            );
                        }
                        window.close();
                        return glib::ControlFlow::Break;
                    }
                    SessionEvent::State(SessionState::Faulted | SessionState::Idle) => {
                        eprintln!("\nFAIL: session ended without completing");
                        window.close();
                        return glib::ControlFlow::Break;
                    }
                    _ => {}
                }
            }
            let mut since = unfocused_since.borrow_mut();
            if session.state() == SessionState::WaitingFocus {
                let start = *since.get_or_insert_with(Instant::now);
                if start.elapsed() > Duration::from_secs(5) {
                    eprintln!("\nFAIL: lost focus for 5 s; cancelled");
                    session.cancel();
                }
            } else {
                *since = None;
            }
            glib::ControlFlow::Continue
        });
    });
}

/// fcitx5's state for the focused window: 0 closed, 1 direct, 2 active.
fn fcitx_state() -> String {
    std::process::Command::new("fcitx5-remote")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|_| "n/a".into())
}
