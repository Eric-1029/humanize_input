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
/// Longest the UI thread may wait for fcitx.
const UI_TIMEOUT: Duration = Duration::from_millis(250);

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
    ///
    /// Does nothing when switching is turned off in the settings. Runs on the
    /// UI thread, so each call gives up after a short timeout.
    pub fn switch_focused(&self, direct: bool) -> bool {
        if !self.enabled.load(Ordering::SeqCst) {
            return false;
        }
        let conn = self.conn.clone();
        let attempt = async move {
            let proxy = ControllerProxy::new(&conn).await.ok()?;
            if direct {
                (proxy.state().await.ok()? == ACTIVE && proxy.deactivate().await.is_ok()).then_some(())
            } else {
                proxy.activate().await.ok()
            }
        };
        // The timer must be created inside the runtime: this runs on the GTK
        // thread, which has no tokio context of its own.
        let quick = async { tokio::time::timeout(UI_TIMEOUT, attempt).await.ok().flatten() };
        self.runtime.block_on(quick).is_some() && direct
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

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use humanize_core::AlwaysFocused;

    use super::*;

    /// The GTK thread has no tokio context; switching from it used to panic
    /// ("no reactor running") and abort the app when the Test page opened.
    #[test]
    fn switching_from_a_thread_without_tokio_context() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let (a, b) = UnixStream::pair().unwrap();
        // A peer with no fcitx behind it: every call fails or times out.
        let (client, _peer) = rt
            .block_on(async {
                tokio::try_join!(
                    zbus::connection::Builder::unix_stream(a).p2p().build(),
                    zbus::connection::Builder::unix_stream(b).server(zbus::Guid::generate()).p2p().build(),
                )
            })
            .unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let guard = ImeGuard::new(rt.handle().clone(), client, Arc::new(AlwaysFocused), true, tx);

        let started = std::time::Instant::now();
        assert!(!guard.switch_focused(true));
        assert!(!guard.switch_focused(false));
        assert!(started.elapsed() < UI_TIMEOUT * 4);
    }
}
