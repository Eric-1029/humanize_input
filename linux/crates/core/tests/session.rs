//! Session behaviour, ported from the original `TypingSessionServiceTests`
//! plus cases for the split Paused / WaitingFocus states and for keys never
//! being left down.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use humanize_core::{
    AlwaysFocused, Driver, DriverError, EventSink, FocusTracker, HumanModel, KeyAction, KeyCode, Layout,
    LogEvent, Preset, Session, SessionEvent, SessionState, StartError, TypingMode, TypingSettings,
    VirtualEditor, WindowId,
};

const TIMEOUT: Duration = Duration::from_secs(10);

fn layout() -> Arc<Layout> {
    Arc::new(Layout::us_qwerty())
}

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

fn classic(settings: TypingSettings) -> TypingMode {
    TypingMode::Classic(settings)
}

/// A fast human model so tests stay quick.
fn quick_human() -> HumanModel {
    HumanModel {
        wpm: 400.0, // sanitized to 250
        word_pause_ms: 0.0,
        punctuation_pause_ms: 0.0,
        sentence_pause_ms: 0.0,
        paragraph_pause_ms: 0.0,
        long_pause_percent: 0.0,
        notice_delay_ms: 5.0,
        backspace_interval_ms: 30.0,
        modifier_lead_ms: 15.0,
        dwell_ms: 40.0,
        lead_in_ms: 0,
        ..Preset::Professional.model()
    }
}

#[derive(Default, Clone)]
struct Log(Arc<Mutex<Vec<SessionEvent>>>);

impl Log {
    fn sink(&self) -> EventSink {
        let events = Arc::clone(&self.0);
        Arc::new(move |e| events.lock().unwrap().push(e))
    }

    fn contains(&self, event: &SessionEvent) -> bool {
        self.0.lock().unwrap().contains(event)
    }
}

fn session(driver: impl Driver + 'static, focus: Arc<dyn FocusTracker>, log: &Log) -> Session {
    Session::new(Box::new(driver), focus, layout(), log.sink())
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for_state(session: &Session, expected: SessionState) {
    wait_until(&format!("{expected:?}"), || session.state() == expected);
}

/// Feeds key events into a shared virtual editor.
#[derive(Clone)]
struct EditorDriver(Arc<Mutex<VirtualEditor>>);

impl EditorDriver {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(VirtualEditor::new(layout()))))
    }

    fn text(&self) -> String {
        self.0.lock().unwrap().text().to_owned()
    }

    fn held(&self) -> Vec<KeyCode> {
        self.0.lock().unwrap().held()
    }

    fn violations(&self) -> Vec<String> {
        self.0.lock().unwrap().violations().to_vec()
    }
}

impl Driver for EditorDriver {
    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
        let action = if down { KeyAction::Press(key) } else { KeyAction::Release(key) };
        self.0.lock().unwrap().apply(action);
        Ok(())
    }
}

/// Blocks inside the first character-key press until released.
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
    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
        assert_ne!(key, KeyCode::BACKSPACE, "backspace was not expected");
        if down && !key.is_modifier() && !self.blocked.swap(true, Ordering::SeqCst) {
            let _ = self.release.recv();
        }
        Ok(())
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

/// Moves focus away right after the first character press.
struct FocusLosingDriver {
    inner: EditorDriver,
    focus: SwitchableFocus,
    typed: Arc<AtomicBool>,
    backspaced: Arc<AtomicBool>,
}

impl Driver for FocusLosingDriver {
    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
        self.inner.key(key, down)?;
        if down && key == KeyCode::BACKSPACE {
            self.backspaced.store(true, Ordering::SeqCst);
        } else if down && !key.is_modifier() && !self.typed.swap(true, Ordering::SeqCst) {
            self.focus.set("other");
        }
        Ok(())
    }
}

#[test]
fn start_normalizes_windows_and_legacy_newlines() {
    let log = Log::default();
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(AlwaysFocused), &log);

    s.start("a\r\nB\rc\nd", classic(fast_settings())).unwrap();
    wait_for_state(&s, SessionState::Completed);

    assert_eq!(driver.text(), "a\nB\nc\nd");
    assert!(driver.violations().is_empty(), "{:?}", driver.violations());
    assert!(log.contains(&SessionEvent::Log(LogEvent::Completed)));
}

#[test]
fn cancel_rejects_restart_until_worker_has_exited() {
    let log = Log::default();
    let (driver, blocked, release) = BlockingDriver::new();
    let s = session(driver, Arc::new(AlwaysFocused), &log);

    s.start("first", classic(fast_settings())).unwrap();
    wait_until("driver to block", || blocked.load(Ordering::SeqCst));

    s.cancel();
    assert_eq!(s.start("too soon", classic(fast_settings())), Err(StartError::Busy));

    release.send(()).unwrap();
    wait_for_state(&s, SessionState::Idle);

    s.start("restart", classic(fast_settings())).unwrap();
    wait_for_state(&s, SessionState::Completed);
}

#[test]
fn correction_waits_for_target_focus_before_backspacing() {
    let log = Log::default();
    let focus = SwitchableFocus::new("target");
    let typed = Arc::new(AtomicBool::new(false));
    let backspaced = Arc::new(AtomicBool::new(false));
    let driver = FocusLosingDriver {
        inner: EditorDriver::new(),
        focus: focus.clone(),
        typed: Arc::clone(&typed),
        backspaced: Arc::clone(&backspaced),
    };
    let s = session(driver, Arc::new(focus.clone()), &log);
    let settings = TypingSettings { typo_rate_percent: 100, repair_rate_percent: 100, ..fast_settings() };

    s.start("a", classic(settings)).unwrap();
    wait_until("first char", || typed.load(Ordering::SeqCst));
    wait_for_state(&s, SessionState::WaitingFocus);
    assert!(!backspaced.load(Ordering::SeqCst));

    focus.set("target");
    wait_until("backspace", || backspaced.load(Ordering::SeqCst));
    wait_for_state(&s, SessionState::Completed);
    assert!(log.contains(&SessionEvent::Log(LogEvent::FocusLost)));
    assert!(log.contains(&SessionEvent::Log(LogEvent::FocusRestored)));
}

#[test]
fn cancel_allows_session_to_restart() {
    let log = Log::default();
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(AlwaysFocused), &log);
    let slow = classic(TypingSettings { base_delay_ms: 30, ..fast_settings() });

    s.start("hello world", slow).unwrap();
    wait_for_state(&s, SessionState::Running);

    s.cancel();
    wait_for_state(&s, SessionState::Idle);
    assert!(log.contains(&SessionEvent::Log(LogEvent::Canceled)));
    assert!(!s.state().is_busy());
    assert!(driver.held().is_empty());

    s.start("restart", slow).unwrap();
    s.cancel();
    wait_for_state(&s, SessionState::Idle);
}

#[test]
fn cancel_during_lead_in_returns_to_idle_without_typing() {
    let log = Log::default();
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(AlwaysFocused), &log);

    s.start("abc", classic(TypingSettings { lead_in_delay_ms: 10_000, ..fast_settings() })).unwrap();
    assert_eq!(s.state(), SessionState::Arming);

    let before = Instant::now();
    s.cancel();
    wait_for_state(&s, SessionState::Idle);
    assert!(before.elapsed() < Duration::from_secs(2), "cancel must interrupt the countdown");
    assert!(driver.text().is_empty());
}

#[test]
fn human_session_types_exact_text_with_repairs() {
    let log = Log::default();
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(AlwaysFocused), &log);
    let model = HumanModel {
        typo_percent: 10.0,
        omission_percent: 5.0,
        transpose_percent: 5.0,
        double_press_percent: 5.0,
        case_slip_percent: 30.0,
        word_redo_percent: 30.0,
        repair_percent: 100.0,
        ..quick_human()
    };

    let text = "Typing With Mistakes, then fixing them: ALL repaired.\n\tDone!";
    s.start_seeded(text, TypingMode::Human(model), 2026).unwrap();
    wait_for_state(&s, SessionState::Completed);

    assert_eq!(driver.text(), text);
    assert!(driver.violations().is_empty(), "{:?}", driver.violations());
    assert!(driver.held().is_empty());
}

#[test]
fn pausing_lifts_every_key_and_resume_keeps_capitals_right() {
    let log = Log::default();
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(AlwaysFocused), &log);
    let text = "SHOUTING IN CAPITALS FOR A WHILE, THEN Calm Words With Shift.";
    let model = HumanModel { wpm: 180.0, ..quick_human() };

    s.start_seeded(text, TypingMode::Human(model), 7).unwrap();
    wait_for_state(&s, SessionState::Running);
    let mut pauses = 0;
    while s.state() != SessionState::Completed {
        std::thread::sleep(Duration::from_millis(23));
        s.pause();
        if s.state() != SessionState::Paused {
            break; // finished in between
        }
        pauses += 1;
        wait_until("keys lifted", || driver.held().is_empty());
        std::thread::sleep(Duration::from_millis(10));
        assert!(driver.held().is_empty(), "keys went down while paused");
        s.resume();
    }
    wait_for_state(&s, SessionState::Completed);

    assert!(pauses >= 3, "only {pauses} pauses happened");
    assert_eq!(driver.text(), text);
    assert!(driver.violations().is_empty(), "{:?}", driver.violations());
}

#[test]
fn resume_while_still_unfocused_goes_back_to_waiting_focus() {
    let log = Log::default();
    let focus = SwitchableFocus::new("target");
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(focus.clone()), &log);

    s.start("abcdef", classic(TypingSettings { base_delay_ms: 20, ..fast_settings() })).unwrap();
    wait_for_state(&s, SessionState::Running);
    focus.set("other");
    wait_for_state(&s, SessionState::WaitingFocus);
    assert!(driver.held().is_empty());

    s.pause();
    assert_eq!(s.state(), SessionState::Paused);
    s.resume();
    // The original reported Running here; we must fall back to WaitingFocus.
    wait_for_state(&s, SessionState::WaitingFocus);

    focus.set("target");
    wait_for_state(&s, SessionState::Completed);
    assert_eq!(driver.text(), "abcdef");
}

#[test]
fn rejects_empty_and_unsupported_text() {
    let log = Log::default();
    let s = session(EditorDriver::new(), Arc::new(AlwaysFocused), &log);

    assert_eq!(s.start(" \n\t", classic(fast_settings())), Err(StartError::EmptyText));
    assert_eq!(
        s.start("你好 world…", TypingMode::Human(HumanModel::default())),
        Err(StartError::UnsupportedChars(3))
    );
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
    let s = session(EditorDriver::new(), Arc::new(NoFocus), &log);
    s.start("abc", classic(fast_settings())).unwrap();
    wait_for_state(&s, SessionState::Idle);
    assert!(log.contains(&SessionEvent::Log(LogEvent::NoTargetWindow)));
}

#[test]
fn driver_failure_faults_the_session_and_lifts_keys() {
    /// Fails on the first character key, after Shift is already down.
    struct Failing(Arc<Mutex<Vec<(KeyCode, bool)>>>);
    impl Driver for Failing {
        fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
            if down && !key.is_modifier() {
                return Err(DriverError("uinput write failed".into()));
            }
            self.0.lock().unwrap().push((key, down));
            Ok(())
        }
    }

    let log = Log::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let s = session(Failing(Arc::clone(&events)), Arc::new(AlwaysFocused), &log);
    s.start("Abc", classic(fast_settings())).unwrap();
    wait_for_state(&s, SessionState::Faulted);
    assert!(log.contains(&SessionEvent::Log(LogEvent::DriverFailed("uinput write failed".into()))));
    assert_eq!(*events.lock().unwrap(), [(KeyCode::RIGHT_SHIFT, true), (KeyCode::RIGHT_SHIFT, false)]);

    // A faulted session does not block the next one.
    s.start("abc", classic(fast_settings())).unwrap();
    wait_for_state(&s, SessionState::Faulted);
}

#[test]
fn full_repair_classic_session_types_exact_text() {
    let log = Log::default();
    let driver = EditorDriver::new();
    let s = session(driver.clone(), Arc::new(AlwaysFocused), &log);
    let settings = TypingSettings {
        typo_rate_percent: 30,
        omission_rate_percent: 20,
        transpose_rate_percent: 20,
        repair_rate_percent: 100,
        ..fast_settings()
    };

    let sample = "Typing with mistakes, then fixing them: 100% repaired.";
    s.start_seeded(sample, classic(settings), 2026).unwrap();
    wait_for_state(&s, SessionState::Completed);

    assert_eq!(driver.text(), sample);
    assert!(driver.violations().is_empty(), "{:?}", driver.violations());
}

#[test]
fn hooks_run_around_typing_and_word_delete_follows_the_target_app() {
    use std::sync::atomic::AtomicUsize;

    struct Terminal;
    impl FocusTracker for Terminal {
        fn current(&self) -> Option<WindowId> {
            Some(WindowId("t".into()))
        }
        fn app_id(&self, _: &WindowId) -> Option<String> {
            Some("org.kde.konsole".into())
        }
    }
    #[derive(Default)]
    struct Counter(AtomicUsize, AtomicUsize);
    impl humanize_core::SessionHooks for Counter {
        fn typing_started(&self, target: &WindowId) {
            assert_eq!(target.0, "t");
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn typing_finished(&self, _: &WindowId) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Counts Ctrl presses on top of the editor.
    #[derive(Clone)]
    struct CtrlSpy(EditorDriver, Arc<AtomicUsize>);
    impl Driver for CtrlSpy {
        fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
            if down && matches!(key, KeyCode::LEFT_CTRL | KeyCode::RIGHT_CTRL) {
                self.1.fetch_add(1, Ordering::SeqCst);
            }
            self.0.key(key, down)
        }
    }

    let log = Log::default();
    let driver = EditorDriver::new();
    let ctrl_presses = Arc::new(AtomicUsize::new(0));
    let counter = Arc::new(Counter::default());
    let spy = CtrlSpy(driver.clone(), Arc::clone(&ctrl_presses));
    let s = session(spy, Arc::new(Terminal), &log).with_hooks(counter.clone());
    let model = HumanModel { word_redo_percent: 100.0, repair_percent: 100.0, ..quick_human() };
    let text = "type several longer words here";
    s.start_seeded(text, TypingMode::Human(model), 1).unwrap();
    wait_for_state(&s, SessionState::Completed);

    assert_eq!(driver.text(), text);
    // In a terminal, fumbled words are erased with Backspace, never Ctrl.
    assert_eq!(ctrl_presses.load(Ordering::SeqCst), 0);
    assert!(driver.violations().is_empty(), "{:?}", driver.violations());
    assert_eq!((counter.0.load(Ordering::SeqCst), counter.1.load(Ordering::SeqCst)), (1, 1));
}
