//! Keeps fcitx5 out of the way while typing.
//!
//! With an input method in Chinese mode, our ASCII keys would end up in its
//! pinyin buffer. Right before typing (target focused), switch fcitx5 to
//! direct input for that window; afterwards switch it back if we changed it
//! and the target still has focus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humanize_core::{FocusTracker, SessionHooks, WindowId};

use crate::DesktopEvent;

/// fcitx5 `State()`: 0 closed, 1 inactive (direct input), 2 active.
const ACTIVE: i32 = 2;
/// Give fcitx5 a moment to apply the switch before the first key.
const SETTLE: Duration = Duration::from_millis(80);

#[zbus::proxy(
    interface = "org.fcitx.Fcitx.Controller1",
    default_service = "org.fcitx.Fcitx5",
    default_path = "/controller"
)]
trait Controller {
    fn state(&self) -> zbus::Result<i32>;
    fn activate(&self) -> zbus::Result<()>;
    fn deactivate(&self) -> zbus::Result<()>;
}

/// [`SessionHooks`] that toggles fcitx5 around typing.
pub struct ImeGuard {
    runtime: tokio::runtime::Handle,
    conn: zbus::Connection,
    focus: Arc<dyn FocusTracker>,
    enabled: AtomicBool,
    restore: Mutex<bool>,
    events: async_channel::Sender<DesktopEvent>,
}

impl ImeGuard {
    pub(crate) fn new(
        runtime: tokio::runtime::Handle,
        conn: zbus::Connection,
        focus: Arc<dyn FocusTracker>,
        enabled: bool,
        events: async_channel::Sender<DesktopEvent>,
    ) -> Self {
        Self { runtime, conn, focus, enabled: AtomicBool::new(enabled), restore: Mutex::new(false), events }
    }

    /// Switches the focused window's input method off (`direct`) or back on.
    /// Returns whether switching off changed anything, so callers know
    /// whether to switch back later.
    pub fn switch_focused(&self, direct: bool) -> bool {
        if direct {
            self.call(async |p| p.state().await) == Some(ACTIVE)
                && self.call(async |p| p.deactivate().await).is_some()
        } else {
            self.call(async |p| p.activate().await).is_some()
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    fn call<T>(&self, f: impl AsyncFnOnce(&ControllerProxy<'_>) -> zbus::Result<T>) -> Option<T> {
        self.runtime.block_on(async {
            let proxy = ControllerProxy::new(&self.conn).await.ok()?;
            f(&proxy).await.ok()
        })
    }
}

impl SessionHooks for ImeGuard {
    fn typing_started(&self, _target: &WindowId) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        // No fcitx5 on the bus: nothing to do.
        if self.call(async |p| p.state().await) == Some(ACTIVE)
            && self.call(async |p| p.deactivate().await).is_some()
        {
            *self.restore.lock().unwrap_or_else(PoisonError::into_inner) = true;
            let _ = self.events.try_send(DesktopEvent::ImeSwitchedOff);
            std::thread::sleep(SETTLE);
        }
    }

    fn typing_finished(&self, target: &WindowId) {
        let restore = std::mem::take(&mut *self.restore.lock().unwrap_or_else(PoisonError::into_inner));
        // Only the focused window's input context can be switched; if the
        // user moved on, leave their new window alone.
        if restore && self.focus.current().as_ref() == Some(target) {
            let _ = self.call(async |p| p.activate().await);
        }
    }
}
