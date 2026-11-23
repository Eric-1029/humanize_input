//! Runs a [`Planner`] against a [`Driver`] on a worker thread.
//!
//! Lifecycle: `Idle → Arming → Running ⇄ Paused / WaitingFocus → Completed |
//! Faulted`, and any state goes back to `Idle` on cancel. A new session can
//! start only after the previous worker has fully exited.
//!
//! Differences from the original Windows build:
//! - The lead-in countdown and target capture live here (`Arming`) instead of
//!   in the UI layer.
//! - User pause (`Paused`) and focus loss (`WaitingFocus`) are separate. In
//!   the original both were `Paused`, so pressing resume while the target was
//!   still unfocused reported `Running` although nothing was typed.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::geometry::KeyboardGeometry;
use crate::plan::{Action, Planner};
use crate::progress::TypingProgress;
use crate::settings::TypingSettings;
use crate::text::find_unsupported;

/// How often the worker re-checks focus while waiting for it to come back.
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(120);

/// Sends keys to whatever window has keyboard focus.
pub trait Driver: Send {
    fn type_char(&mut self, c: char) -> Result<(), DriverError>;
    fn backspace(&mut self) -> Result<(), DriverError>;
}

/// Failure reported by a [`Driver`]; the session ends as `Faulted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverError(pub String);

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DriverError {}

/// Opaque identity of a top-level window, as reported by a [`FocusTracker`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowId(pub String);

/// Reports which window currently has keyboard focus.
pub trait FocusTracker: Send + Sync {
    /// The focused window, or `None` if it cannot be determined right now.
    /// Called before every key, so it must be cheap.
    fn current(&self) -> Option<WindowId>;
}

/// For desktops without focus detection: the target is always focused.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysFocused;

impl FocusTracker for AlwaysFocused {
    fn current(&self) -> Option<WindowId> {
        Some(WindowId(String::new()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    /// Counting down the lead-in before capturing the target window.
    Arming,
    Running,
    /// Paused by the user.
    Paused,
    /// The target window lost focus; typing resumes when it comes back.
    WaitingFocus,
    Completed,
    Faulted,
}

impl SessionState {
    /// Whether a worker is between start and finish.
    pub fn is_busy(self) -> bool {
        matches!(self, Self::Arming | Self::Running | Self::Paused | Self::WaitingFocus)
    }
}

/// Things worth telling the user. Text is chosen by the UI (bilingual).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    Paused,
    Resumed,
    Canceled,
    FocusLost,
    FocusRestored,
    NoTargetWindow,
    Completed,
    DriverFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    State(SessionState),
    Progress(TypingProgress),
    Log(LogEvent),
}

/// Why [`Session::start`] refused to begin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// The text is empty or whitespace only.
    EmptyText,
    /// The text contains this many characters that cannot be typed.
    UnsupportedChars(usize),
    /// A previous session has not finished yet (including one being canceled).
    Busy,
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyText => f.write_str("text is empty"),
            Self::UnsupportedChars(n) => write!(f, "text contains {n} unsupported characters"),
            Self::Busy => f.write_str("a session is already active"),
        }
    }
}

impl std::error::Error for StartError {}

pub type EventSink = Arc<dyn Fn(SessionEvent) + Send + Sync>;

struct Control {
    state: SessionState,
    /// True from `start` until the worker thread has exited.
    worker_active: bool,
    cancel: bool,
    user_paused: bool,
}

struct Shared {
    control: Mutex<Control>,
    wake: Condvar,
    sink: EventSink,
    focus: Arc<dyn FocusTracker>,
}

/// Marker for "the session was canceled; unwind".
struct Canceled;

enum Interrupt {
    Canceled,
    Driver(DriverError),
}

impl From<Canceled> for Interrupt {
    fn from(_: Canceled) -> Self {
        Self::Canceled
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Control> {
        // The lock only guards plain flags; a panic elsewhere cannot leave
        // them half-updated, so recovering from poison is safe.
        self.control.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: SessionEvent) {
        (self.sink)(event);
    }

    /// Changes state under the lock, then emits outside it so the sink may
    /// call back into the session.
    fn transition(&self, change: impl FnOnce(&mut Control) -> Option<SessionState>) {
        let new_state = change(&mut self.lock());
        if let Some(state) = new_state {
            self.emit(SessionEvent::State(state));
        }
    }

    fn set_state(&self, state: SessionState) {
        self.transition(|c| {
            c.state = state;
            Some(state)
        });
    }

    /// Blocks while the user has paused; fails once canceled.
    fn checkpoint(&self) -> Result<(), Canceled> {
        let mut c = self.lock();
        loop {
            if c.cancel {
                return Err(Canceled);
            }
            if !c.user_paused {
                return Ok(());
            }
            c = self.wake.wait(c).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Sleeps for `duration`, waking early only to cancel.
    fn sleep(&self, duration: Duration) -> Result<(), Canceled> {
        let deadline = Instant::now() + duration;
        let mut c = self.lock();
        loop {
            if c.cancel {
                return Err(Canceled);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            c = self.wake.wait_timeout(c, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Waits until `target` has focus, reporting `WaitingFocus` meanwhile.
    fn ensure_focus(&self, target: &WindowId) -> Result<(), Canceled> {
        let mut announced = false;
        loop {
            self.checkpoint()?;
            if self.focus.current().as_ref() == Some(target) {
                break;
            }
            // Re-entered on every poll so that a user resume during focus
            // loss goes back to WaitingFocus instead of claiming Running.
            self.transition(|c| {
                (c.state == SessionState::Running).then(|| {
                    c.state = SessionState::WaitingFocus;
                    c.state
                })
            });
            if !announced {
                announced = true;
                self.emit(SessionEvent::Log(LogEvent::FocusLost));
            }
            self.sleep(FOCUS_POLL_INTERVAL)?;
        }

        let mut restored = false;
        self.transition(|c| {
            restored = c.state == SessionState::WaitingFocus;
            restored.then(|| {
                c.state = SessionState::Running;
                c.state
            })
        });
        if announced || restored {
            self.emit(SessionEvent::Log(LogEvent::FocusRestored));
        }
        Ok(())
    }
}

/// Owns the driver and runs at most one typing session at a time.
pub struct Session {
    shared: Arc<Shared>,
    driver: Arc<Mutex<Box<dyn Driver>>>,
    geometry: Arc<KeyboardGeometry>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Session {
    pub fn new(
        driver: Box<dyn Driver>,
        focus: Arc<dyn FocusTracker>,
        geometry: Arc<KeyboardGeometry>,
        sink: EventSink,
    ) -> Self {
        let control =
            Control { state: SessionState::Idle, worker_active: false, cancel: false, user_paused: false };
        Self {
            shared: Arc::new(Shared { control: Mutex::new(control), wake: Condvar::new(), sink, focus }),
            driver: Arc::new(Mutex::new(driver)),
            geometry,
            worker: Mutex::new(None),
        }
    }

    pub fn state(&self) -> SessionState {
        self.shared.lock().state
    }

    /// Validates `text` and starts typing it with a fresh random seed.
    pub fn start(&self, text: &str, settings: TypingSettings) -> Result<(), StartError> {
        self.start_with(text, settings, None)
    }

    /// Like [`start`](Self::start) but deterministic, for tests and dry runs.
    pub fn start_seeded(&self, text: &str, settings: TypingSettings, seed: u64) -> Result<(), StartError> {
        self.start_with(text, settings, Some(seed))
    }

    fn start_with(&self, text: &str, settings: TypingSettings, seed: Option<u64>) -> Result<(), StartError> {
        if text.trim().is_empty() {
            return Err(StartError::EmptyText);
        }
        let unsupported = find_unsupported(text).len();
        if unsupported > 0 {
            return Err(StartError::UnsupportedChars(unsupported));
        }

        {
            let mut c = self.shared.lock();
            if c.worker_active {
                return Err(StartError::Busy);
            }
            c.worker_active = true;
            c.cancel = false;
            c.user_paused = false;
            c.state = SessionState::Arming;
        }
        self.shared.emit(SessionEvent::State(SessionState::Arming));

        let planner = match seed {
            Some(seed) => Planner::with_seed(text, settings, Arc::clone(&self.geometry), seed),
            None => Planner::new(text, settings, Arc::clone(&self.geometry)),
        };
        let shared = Arc::clone(&self.shared);
        let driver = Arc::clone(&self.driver);
        let lead_in = Duration::from_millis(u64::from(settings.lead_in_delay_ms));
        let handle = thread::Builder::new()
            .name("humanize-typing".into())
            .spawn(move || run_worker(&shared, &driver, planner, lead_in))
            .expect("spawning the typing thread");

        // The previous worker (if any) has already cleared worker_active, so
        // joining it here returns immediately.
        if let Some(old) = self.worker_lock().replace(handle) {
            let _ = old.join();
        }
        Ok(())
    }

    /// Pauses a running (or focus-waiting) session.
    pub fn pause(&self) {
        let mut paused = false;
        self.shared.transition(|c| {
            paused = matches!(c.state, SessionState::Running | SessionState::WaitingFocus);
            paused.then(|| {
                c.user_paused = true;
                c.state = SessionState::Paused;
                c.state
            })
        });
        if paused {
            self.shared.emit(SessionEvent::Log(LogEvent::Paused));
        }
    }

    /// Resumes a user-paused session. If the target is still unfocused the
    /// worker moves on to `WaitingFocus` at its next check.
    pub fn resume(&self) {
        let mut resumed = false;
        self.shared.transition(|c| {
            resumed = c.state == SessionState::Paused;
            resumed.then(|| {
                c.user_paused = false;
                c.state = SessionState::Running;
                c.state
            })
        });
        if resumed {
            self.shared.wake.notify_all();
            self.shared.emit(SessionEvent::Log(LogEvent::Resumed));
        }
    }

    /// Pause if running, resume if paused.
    pub fn toggle_pause(&self) {
        match self.state() {
            SessionState::Paused => self.resume(),
            _ => self.pause(),
        }
    }

    /// Cancels the active session. The state becomes `Idle` once the worker
    /// has exited; until then [`start`](Self::start) returns `Busy`.
    pub fn cancel(&self) {
        let active = {
            let mut c = self.shared.lock();
            if c.worker_active {
                c.cancel = true;
                c.user_paused = false;
            }
            c.worker_active
        };
        if active {
            self.shared.wake.notify_all();
            self.shared.emit(SessionEvent::Log(LogEvent::Canceled));
        }
    }

    fn worker_lock(&self) -> MutexGuard<'_, Option<JoinHandle<()>>> {
        self.worker.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
        if let Some(handle) = self.worker_lock().take() {
            let _ = handle.join();
        }
    }
}

fn run_worker(shared: &Shared, driver: &Mutex<Box<dyn Driver>>, planner: Planner, lead_in: Duration) {
    let outcome = type_text(shared, driver, planner, lead_in);

    let final_state = match outcome {
        Ok(true) => SessionState::Completed,
        Ok(false) | Err(Interrupt::Canceled) => SessionState::Idle,
        Err(Interrupt::Driver(_)) => SessionState::Faulted,
    };
    match outcome {
        Ok(true) => shared.emit(SessionEvent::Log(LogEvent::Completed)),
        Ok(false) => shared.emit(SessionEvent::Log(LogEvent::NoTargetWindow)),
        Err(Interrupt::Driver(e)) => shared.emit(SessionEvent::Log(LogEvent::DriverFailed(e.0))),
        Err(Interrupt::Canceled) => {}
    }

    // Publish the final state and release the slot in one step, so a caller
    // that sees the final state can immediately start again.
    shared.transition(|c| {
        c.state = final_state;
        c.worker_active = false;
        c.cancel = false;
        c.user_paused = false;
        Some(final_state)
    });
}

/// Returns `Ok(false)` when no target window could be captured.
fn type_text(
    shared: &Shared,
    driver: &Mutex<Box<dyn Driver>>,
    planner: Planner,
    lead_in: Duration,
) -> Result<bool, Interrupt> {
    shared.sleep(lead_in)?;
    let Some(target) = shared.focus.current() else {
        return Ok(false);
    };
    shared.set_state(SessionState::Running);

    let mut driver = driver.lock().unwrap_or_else(PoisonError::into_inner);
    for action in planner {
        shared.checkpoint()?;
        match action {
            Action::Type(c) => {
                shared.ensure_focus(&target)?;
                driver.type_char(c).map_err(Interrupt::Driver)?;
            }
            Action::Backspace => {
                shared.ensure_focus(&target)?;
                driver.backspace().map_err(Interrupt::Driver)?;
            }
            Action::Wait(d) => shared.sleep(d)?,
            Action::Progress(p) => shared.emit(SessionEvent::Progress(p)),
        }
    }
    Ok(true)
}
