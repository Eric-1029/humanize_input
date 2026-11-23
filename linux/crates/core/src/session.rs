//! Runs a typing plan against a key-level [`Driver`] on a worker thread.
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
//! - Keys are never left down: pausing, losing focus, cancelling or failing
//!   lifts every held key. Modifiers that were down are pressed again right
//!   before the next key, so a capital interrupted mid-stroke stays a capital.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::geometry::KeyboardGeometry;
use crate::human::{HumanModel, HumanPlanner};
use crate::keys::{KeyAction, KeyCode};
use crate::layout::Layout;
use crate::plan::{ClassicKeys, Planner};
use crate::progress::TypingProgress;
use crate::settings::TypingSettings;

/// How often the worker re-checks focus while waiting for it to come back.
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(120);

/// Sends physical key events to whatever window has keyboard focus.
pub trait Driver: Send {
    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError>;
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
    /// Called before every key press, so it must be cheap.
    fn current(&self) -> Option<WindowId>;

    /// Application id of `window` (Wayland app id / X11 class), if known.
    /// Used to adapt behaviour to the target, e.g. word deletion in terminals.
    fn app_id(&self, _window: &WindowId) -> Option<String> {
        None
    }
}

/// Callbacks around the typing phase, run on the worker thread.
pub trait SessionHooks: Send + Sync {
    /// The target window was captured and typing is about to begin.
    fn typing_started(&self, _target: &WindowId) {}
    /// Typing ended (completed, cancelled or failed) and all keys are up.
    fn typing_finished(&self, _target: &WindowId) {}
}

/// For desktops without focus detection: the target is always focused.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysFocused;

impl FocusTracker for AlwaysFocused {
    fn current(&self) -> Option<WindowId> {
        Some(WindowId(String::new()))
    }
}

/// Which typing model drives a session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypingMode {
    /// The original Windows model, kept for compatibility.
    Classic(TypingSettings),
    /// Key-level human model with rhythm, rollover and richer mistakes.
    Human(HumanModel),
}

impl TypingMode {
    pub fn lead_in(&self) -> Duration {
        let ms = match self {
            Self::Classic(s) => s.lead_in_delay_ms,
            Self::Human(m) => m.lead_in_ms,
        };
        Duration::from_millis(u64::from(ms))
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

type ActionStream = Box<dyn Iterator<Item = KeyAction> + Send>;

/// Builds the action stream once the target's app id is known.
type PlanFactory = Box<dyn FnOnce(Option<&str>) -> ActionStream + Send>;

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
    hooks: Option<Arc<dyn SessionHooks>>,
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

impl From<DriverError> for Interrupt {
    fn from(e: DriverError) -> Self {
        Self::Driver(e)
    }
}

/// Outcome of a sleep that a user pause may cut short.
enum Slept {
    Done,
    PausedWithLeft(Duration),
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

    fn is_paused(&self) -> bool {
        self.lock().user_paused
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

    /// Sleeps for `duration` unless the user pauses first.
    fn sleep_unless_paused(&self, duration: Duration) -> Result<Slept, Canceled> {
        let deadline = Instant::now() + duration;
        let mut c = self.lock();
        loop {
            if c.cancel {
                return Err(Canceled);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(Slept::Done);
            }
            if c.user_paused {
                return Ok(Slept::PausedWithLeft(deadline - now));
            }
            c = self.wake.wait_timeout(c, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }
}

/// Plays actions through the driver while tracking which keys are down.
struct Executor<'a> {
    shared: &'a Shared,
    driver: &'a mut dyn Driver,
    target: WindowId,
    /// Keys physically down, in press order.
    down: Vec<KeyCode>,
    /// Modifiers lifted by a pause or focus loss; pressed again before the
    /// next key press unless their release comes first.
    restore: Vec<KeyCode>,
}

impl Executor<'_> {
    fn run(&mut self, actions: ActionStream) -> Result<(), Interrupt> {
        for action in actions {
            self.gate()?;
            match action {
                KeyAction::Press(key) => {
                    self.ensure_focus()?;
                    for m in std::mem::take(&mut self.restore) {
                        self.key(m, true)?;
                    }
                    self.key(key, true)?;
                }
                KeyAction::Release(key) => {
                    if let Some(pos) = self.restore.iter().position(|&k| k == key) {
                        self.restore.remove(pos);
                    } else if self.down.contains(&key) {
                        self.key(key, false)?;
                    }
                }
                KeyAction::Wait(d) => self.wait(d)?,
                KeyAction::Progress(p) => self.shared.emit(SessionEvent::Progress(p)),
            }
        }
        Ok(())
    }

    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
        self.driver.key(key, down)?;
        if down {
            self.down.push(key);
        } else {
            self.down.retain(|&k| k != key);
        }
        Ok(())
    }

    /// Lifts every held key, remembering modifiers for later.
    fn suspend(&mut self) -> Result<(), DriverError> {
        while let Some(key) = self.down.pop() {
            self.driver.key(key, false)?;
            if key.is_modifier() {
                self.restore.push(key);
            }
        }
        Ok(())
    }

    /// Best-effort release of everything, used on every exit path.
    fn release_all(&mut self) {
        while let Some(key) = self.down.pop() {
            let _ = self.driver.key(key, false);
        }
        self.restore.clear();
    }

    /// Honours a user pause: lift keys, then block until resumed.
    fn gate(&mut self) -> Result<(), Interrupt> {
        if self.shared.is_paused() {
            self.suspend()?;
        }
        Ok(self.shared.checkpoint()?)
    }

    fn wait(&mut self, duration: Duration) -> Result<(), Interrupt> {
        let mut left = duration;
        loop {
            match self.shared.sleep_unless_paused(left)? {
                Slept::Done => return Ok(()),
                Slept::PausedWithLeft(rest) => {
                    left = rest;
                    self.gate()?;
                }
            }
        }
    }

    /// Waits until the target has focus, reporting `WaitingFocus` meanwhile.
    fn ensure_focus(&mut self) -> Result<(), Interrupt> {
        let shared = self.shared;
        if shared.focus.current().as_ref() == Some(&self.target) {
            return Ok(());
        }
        // Never keep Shift or a letter down while the user is elsewhere.
        self.suspend()?;
        shared.emit(SessionEvent::Log(LogEvent::FocusLost));
        loop {
            self.gate()?;
            if shared.focus.current().as_ref() == Some(&self.target) {
                break;
            }
            // Re-entered on every poll so that a user resume during focus
            // loss goes back to WaitingFocus instead of claiming Running.
            shared.transition(|c| {
                (c.state == SessionState::Running).then(|| {
                    c.state = SessionState::WaitingFocus;
                    c.state
                })
            });
            shared.sleep(FOCUS_POLL_INTERVAL)?;
        }
        shared.transition(|c| {
            (c.state == SessionState::WaitingFocus).then(|| {
                c.state = SessionState::Running;
                c.state
            })
        });
        shared.emit(SessionEvent::Log(LogEvent::FocusRestored));
        Ok(())
    }
}

/// Owns the driver and runs at most one typing session at a time.
pub struct Session {
    shared: Arc<Shared>,
    driver: Arc<Mutex<Box<dyn Driver>>>,
    /// Layout and the classic model's neighbour table derived from it.
    layout: Mutex<(Arc<Layout>, Arc<KeyboardGeometry>)>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Session {
    pub fn new(
        driver: Box<dyn Driver>,
        focus: Arc<dyn FocusTracker>,
        layout: Arc<Layout>,
        sink: EventSink,
    ) -> Self {
        let control =
            Control { state: SessionState::Idle, worker_active: false, cancel: false, user_paused: false };
        Self {
            shared: Arc::new(Shared {
                control: Mutex::new(control),
                wake: Condvar::new(),
                sink,
                focus,
                hooks: None,
            }),
            driver: Arc::new(Mutex::new(driver)),
            layout: Mutex::new((Arc::clone(&layout), Arc::new(layout.classic_geometry()))),
            worker: Mutex::new(None),
        }
    }

    /// Installs callbacks around typing. Call before the first start.
    pub fn with_hooks(mut self, hooks: Arc<dyn SessionHooks>) -> Self {
        Arc::get_mut(&mut self.shared).expect("with_hooks before any session started").hooks = Some(hooks);
        self
    }

    pub fn state(&self) -> SessionState {
        self.shared.lock().state
    }

    /// The layout characters are mapped with.
    pub fn layout(&self) -> Arc<Layout> {
        Arc::clone(&self.layout_lock().0)
    }

    /// Switches to another keyboard layout (e.g. after the user changed it
    /// in KDE). Refused while a session is active.
    pub fn set_layout(&self, layout: Arc<Layout>) -> Result<(), StartError> {
        if self.shared.lock().worker_active {
            return Err(StartError::Busy);
        }
        let geometry = Arc::new(layout.classic_geometry());
        *self.layout_lock() = (layout, geometry);
        Ok(())
    }

    fn layout_lock(&self) -> MutexGuard<'_, (Arc<Layout>, Arc<KeyboardGeometry>)> {
        self.layout.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Validates `text` and starts typing it with a fresh random seed.
    pub fn start(&self, text: &str, mode: TypingMode) -> Result<(), StartError> {
        self.start_with(text, mode, None)
    }

    /// Like [`start`](Self::start) but deterministic, for tests and dry runs.
    pub fn start_seeded(&self, text: &str, mode: TypingMode, seed: u64) -> Result<(), StartError> {
        self.start_with(text, mode, Some(seed))
    }

    fn plan_factory(&self, text: &str, mode: TypingMode, seed: Option<u64>) -> PlanFactory {
        let (layout, geometry) = self.layout_lock().clone();
        let text = text.to_owned();
        Box::new(move |app_id: Option<&str>| -> ActionStream {
            match mode {
                TypingMode::Classic(settings) => {
                    let planner = match seed {
                        Some(seed) => Planner::with_seed(&text, settings, geometry, seed),
                        None => Planner::new(&text, settings, geometry),
                    };
                    Box::new(ClassicKeys::new(planner, layout))
                }
                TypingMode::Human(mut model) => {
                    model.word_delete = model.word_delete.resolve(app_id);
                    Box::new(match seed {
                        Some(seed) => HumanPlanner::with_seed(&text, model, layout, seed),
                        None => HumanPlanner::new(&text, model, layout),
                    })
                }
            }
        })
    }

    fn start_with(&self, text: &str, mode: TypingMode, seed: Option<u64>) -> Result<(), StartError> {
        if text.trim().is_empty() {
            return Err(StartError::EmptyText);
        }
        let unsupported = self.layout().find_unsupported(text).len();
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

        let plan = self.plan_factory(text, mode, seed);
        let shared = Arc::clone(&self.shared);
        let driver = Arc::clone(&self.driver);
        let lead_in = mode.lead_in();
        let handle = thread::Builder::new()
            .name("humanize-typing".into())
            .spawn(move || run_worker(&shared, &driver, plan, lead_in))
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
            // Wake a sleeping worker so it lifts held keys right away.
            self.shared.wake.notify_all();
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

fn run_worker(shared: &Shared, driver: &Mutex<Box<dyn Driver>>, plan: PlanFactory, lead_in: Duration) {
    let mut driver = driver.lock().unwrap_or_else(PoisonError::into_inner);
    let outcome = type_text(shared, &mut **driver, plan, lead_in);

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
    driver: &mut dyn Driver,
    plan: PlanFactory,
    lead_in: Duration,
) -> Result<bool, Interrupt> {
    shared.sleep(lead_in)?;
    let Some(target) = shared.focus.current() else {
        return Ok(false);
    };
    let actions = plan(shared.focus.app_id(&target).as_deref());
    shared.set_state(SessionState::Running);
    if let Some(hooks) = &shared.hooks {
        hooks.typing_started(&target);
    }

    let mut executor =
        Executor { shared, driver, target: target.clone(), down: Vec::new(), restore: Vec::new() };
    let result = executor.run(actions);
    executor.release_all();
    if let Some(hooks) = &shared.hooks {
        hooks.typing_finished(&target);
    }
    result.map(|()| true)
}
