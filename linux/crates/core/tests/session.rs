//! Session behaviour, ported from the original `TypingSessionServiceTests`
//! plus cases for the split Paused / WaitingFocus states.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use humanize_core::{
    AlwaysFocused, Driver, DriverError, FocusTracker, KeyboardGeometry, LogEvent, Session, SessionEvent,
    SessionState, StartError, TypingSettings, WindowId,
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn fast_settings() -> TypingSettings {
    TypingSettings {
        base_delay_ms: 1,
        jitter_percent: 0,
        typo_rate_percent: 0,
        omission_rate_percent: 0,
        transpose_rate_percent: 0,
        repair_rate_percent: 0,
        error_detect_delay_ms: 1,
        backspace_delay_ms: 1,
        lead_in_delay_ms: 0,
    }
}

#[derive(Default, Clone)]
struct Log(Arc<Mutex<Vec<SessionEvent>>>);

impl Log {
    fn sink(&self) -> humanize_core::EventSink {
        let events = Arc::clone(&self.0);
        Arc::new(move |e| events.lock().unwrap().push(e))
    }

    fn contains(&self, event: &SessionEvent) -> bool {
        self.0.lock().unwrap().contains(event)
    }
}

fn session(driver: impl Driver + 'static, focus: Arc<dyn FocusTracker>, log: &Log) -> Session {
    Session::new(Box::new(driver), focus, Arc::new(KeyboardGeometry::us_qwerty()), log.sink())
}

fn wait_for_state(session: &Session, expected: SessionState) {
    let deadline = Instant::now() + TIMEOUT;
    while session.state() != expected {
        assert!(Instant::now() < deadline, "expected {expected:?}, still {:?}", session.state());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for(flag: &AtomicBool, what: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while !flag.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Records typed text; backspace removes the last char.
#[derive(Clone, Default)]
struct RecordingDriver {
    text: Arc<Mutex<String>>,
    backspaces: Arc<AtomicUsize>,
}

impl Driver for RecordingDriver {
    fn type_char(&mut self, c: char) -> Result<(), DriverError> {
        self.text.lock().unwrap().push(c);
        Ok(())
    }

    fn backspace(&mut self) -> Result<(), DriverError> {
        self.text.lock().unwrap().pop();
        self.backspaces.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Blocks inside the first `type_char` until released.
struct BlockingDriver {
    blocked: Arc<AtomicBool>,
    release: Receiver<()>,
}

impl BlockingDriver {
    fn new() -> (Self, Arc<AtomicBool>, Sender<()>) {
        let blocked = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        (Self { blocked: Arc::clone(&blocked), release: rx }, blocked, tx)
    }
}

impl Driver for BlockingDriver {
    fn type_char(&mut self, _: char) -> Result<(), DriverError> {
        if !self.blocked.swap(true, Ordering::SeqCst) {
            let _ = self.release.recv();
        }
        Ok(())
    }

    fn backspace(&mut self) -> Result<(), DriverError> {
        panic!("backspace was not expected");
    }
}

/// Focus is a shared, switchable window id.
#[derive(Clone)]
struct SwitchableFocus(Arc<Mutex<String>>);

impl SwitchableFocus {
    fn new(window: &str) -> Self {
        Self(Arc::new(Mutex::new(window.into())))
    }

    fn set(&self, window: &str) {
        *self.0.lock().unwrap() = window.into();
    }
}

impl FocusTracker for SwitchableFocus {
    fn current(&self) -> Option<WindowId> {
        Some(WindowId(self.0.lock().unwrap().clone()))
    }
}

/// Moves focus away right after the first typed char.
struct FocusLosingDriver {
    focus: SwitchableFocus,
    typed: Arc<AtomicBool>,
    backspaced: Arc<AtomicBool>,
}

impl Driver for FocusLosingDriver {
    fn type_char(&mut self, _: char) -> Result<(), DriverError> {
        if !self.typed.swap(true, Ordering::SeqCst) {
            self.focus.set("other");
        }
        Ok(())
    }

    fn backspace(&mut self) -> Result<(), DriverError> {
        self.backspaced.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn start_normalizes_windows_and_legacy_newlines() {
    let log = Log::default();
    let driver = RecordingDriver::default();
    let text = Arc::clone(&driver.text);
    let s = session(driver, Arc::new(AlwaysFocused), &log);

    s.start("a\r\nb\rc\nd", fast_settings()).unwrap();
    wait_for_state(&s, SessionState::Completed);

    assert_eq!(*text.lock().unwrap(), "a\nb\nc\nd");
    assert!(log.contains(&SessionEvent::Log(LogEvent::Completed)));
}

#[test]
fn cancel_rejects_restart_until_worker_has_exited() {
    let log = Log::default();
    let (driver, blocked, release) = BlockingDriver::new();
    let s = session(driver, Arc::new(AlwaysFocused), &log);

    s.start("first", fast_settings()).unwrap();
    wait_for(&blocked, "driver to block");

    s.cancel();
    assert_eq!(s.start("too soon", fast_settings()), Err(StartError::Busy));

    release.send(()).unwrap();
    wait_for_state(&s, SessionState::Idle);

    s.start("restart", fast_settings()).unwrap();
    wait_for_state(&s, SessionState::Completed);
}

#[test]
fn correction_waits_for_target_focus_before_backspacing() {
    let log = Log::default();
    let focus = SwitchableFocus::new("target");
    let typed = Arc::new(AtomicBool::new(false));
    let backspaced = Arc::new(AtomicBool::new(false));
    let driver = FocusLosingDriver {
        focus: focus.clone(),
        typed: Arc::clone(&typed),
        backspaced: Arc::clone(&backspaced),
    };
    let s = session(driver, Arc::new(focus.clone()), &log);
    let settings = TypingSettings { typo_rate_percent: 100, repair_rate_percent: 100, ..fast_settings() };

    s.start("a", settings).unwrap();
    wait_for(&typed, "first char");
    wait_for_state(&s, SessionState::WaitingFocus);
    assert!(!backspaced.load(Ordering::SeqCst));

    focus.set("target");
    wait_for(&backspaced, "backspace");
    wait_for_state(&s, SessionState::Completed);
    assert!(log.contains(&SessionEvent::Log(LogEvent::FocusLost)));
    assert!(log.contains(&SessionEvent::Log(LogEvent::FocusRestored)));
}

#[test]
fn cancel_allows_session_to_restart() {
    let log = Log::default();
    let s = session(RecordingDriver::default(), Arc::new(AlwaysFocused), &log);
    let slow = TypingSettings { base_delay_ms: 30, ..fast_settings() };

    s.start("hello world", slow).unwrap();
    wait_for_state(&s, SessionState::Running);

    s.cancel();
    wait_for_state(&s, SessionState::Idle);
    assert!(log.contains(&SessionEvent::Log(LogEvent::Canceled)));
    assert!(!s.state().is_busy());

    s.start("restart", slow).unwrap();
    s.cancel();
    wait_for_state(&s, SessionState::Idle);
}

#[test]
fn cancel_during_lead_in_returns_to_idle_without_typing() {
    let log = Log::default();
    let driver = RecordingDriver::default();
    let text = Arc::clone(&driver.text);
    let s = session(driver, Arc::new(AlwaysFocused), &log);

    s.start("abc", TypingSettings { lead_in_delay_ms: 10_000, ..fast_settings() }).unwrap();
    assert_eq!(s.state(), SessionState::Arming);

    let before = Instant::now();
    s.cancel();
    wait_for_state(&s, SessionState::Idle);
    assert!(before.elapsed() < Duration::from_secs(2), "cancel must interrupt the countdown");
    assert!(text.lock().unwrap().is_empty());
}

#[test]
fn pause_stops_typing_and_resume_finishes() {
    let log = Log::default();
    let driver = RecordingDriver::default();
    let text = Arc::clone(&driver.text);
    let s = session(driver, Arc::new(AlwaysFocused), &log);

    s.start("abcdefghij", TypingSettings { base_delay_ms: 20, ..fast_settings() }).unwrap();
    wait_for_state(&s, SessionState::Running);
    s.pause();
    assert_eq!(s.state(), SessionState::Paused);

    // At most one in-flight char may land after pausing.
    let frozen = text.lock().unwrap().len();
    std::thread::sleep(Duration::from_millis(100));
    assert!(text.lock().unwrap().len() <= frozen + 1);

    s.resume();
    wait_for_state(&s, SessionState::Completed);
    assert_eq!(*text.lock().unwrap(), "abcdefghij");
}

#[test]
fn resume_while_still_unfocused_goes_back_to_waiting_focus() {
    let log = Log::default();
    let focus = SwitchableFocus::new("target");
    let driver = RecordingDriver::default();
    let text = Arc::clone(&driver.text);
    let s = session(driver, Arc::new(focus.clone()), &log);

    s.start("abcdef", TypingSettings { base_delay_ms: 20, ..fast_settings() }).unwrap();
    wait_for_state(&s, SessionState::Running);
    focus.set("other");
    wait_for_state(&s, SessionState::WaitingFocus);

    s.pause();
    assert_eq!(s.state(), SessionState::Paused);
    s.resume();
    // The original reported Running here; we must fall back to WaitingFocus.
    wait_for_state(&s, SessionState::WaitingFocus);

    focus.set("target");
    wait_for_state(&s, SessionState::Completed);
    assert_eq!(*text.lock().unwrap(), "abcdef");
}

#[test]
fn rejects_empty_and_unsupported_text() {
    let log = Log::default();
    let s = session(RecordingDriver::default(), Arc::new(AlwaysFocused), &log);

    assert_eq!(s.start(" \n\t", fast_settings()), Err(StartError::EmptyText));
    assert_eq!(s.start("你好 world…", fast_settings()), Err(StartError::UnsupportedChars(3)));
    assert_eq!(s.state(), SessionState::Idle);
}

#[test]
fn missing_target_window_aborts_to_idle() {
    struct NoFocus;
    impl FocusTracker for NoFocus {
        fn current(&self) -> Option<WindowId> {
            None
        }
    }

    let log = Log::default();
    let s = session(RecordingDriver::default(), Arc::new(NoFocus), &log);
    s.start("abc", fast_settings()).unwrap();
    wait_for_state(&s, SessionState::Idle);
    assert!(log.contains(&SessionEvent::Log(LogEvent::NoTargetWindow)));
}

#[test]
fn driver_failure_faults_the_session() {
    struct Failing;
    impl Driver for Failing {
        fn type_char(&mut self, _: char) -> Result<(), DriverError> {
            Err(DriverError("uinput write failed".into()))
        }
        fn backspace(&mut self) -> Result<(), DriverError> {
            Ok(())
        }
    }

    let log = Log::default();
    let s = session(Failing, Arc::new(AlwaysFocused), &log);
    s.start("abc", fast_settings()).unwrap();
    wait_for_state(&s, SessionState::Faulted);
    assert!(log.contains(&SessionEvent::Log(LogEvent::DriverFailed("uinput write failed".into()))));

    // A faulted session does not block the next one.
    s.start("abc", fast_settings()).unwrap();
    wait_for_state(&s, SessionState::Faulted);
}

#[test]
fn full_repair_session_types_exact_text() {
    let log = Log::default();
    let driver = RecordingDriver::default();
    let text = Arc::clone(&driver.text);
    let backspaces = Arc::clone(&driver.backspaces);
    let s = session(driver, Arc::new(AlwaysFocused), &log);
    let settings = TypingSettings {
        typo_rate_percent: 30,
        omission_rate_percent: 20,
        transpose_rate_percent: 20,
        repair_rate_percent: 100,
        ..fast_settings()
    };

    let sample = "Typing with mistakes, then fixing them: 100% repaired.";
    s.start_seeded(sample, settings, 2026).unwrap();
    wait_for_state(&s, SessionState::Completed);

    assert_eq!(*text.lock().unwrap(), sample);
    assert!(backspaces.load(Ordering::SeqCst) > 0, "seed should produce at least one fix");
}
