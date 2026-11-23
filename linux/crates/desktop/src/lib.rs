//! Desktop integration for humanize_input on KDE Plasma.
//!
//! [`Desktop`] runs the DBus-facing services on its own tokio runtime and
//! reports everything through one event channel:
//! - active-window tracking via a KWin script ([`kwin::KwinFocus`]),
//! - fcitx5 switching around typing ([`fcitx::ImeGuard`]),
//! - the active keyboard layout and layout switches,
//! - global hotkeys via the XDG GlobalShortcuts portal,
//! - the system tray icon.
//!
//! Every service is optional: if one is unavailable the rest still work and
//! [`Desktop::warnings`] says what is missing.

pub mod config;
pub mod fcitx;
pub mod kwin;
mod layout;
pub mod shortcuts;
pub mod tray;

use std::sync::Arc;

use humanize_core::{AlwaysFocused, FocusTracker, SessionHooks};
use humanize_keymap::XkbNames;

pub use config::Config;
pub use kwin::{ActiveWindow, KwinFocus};
pub use shortcuts::{BoundHotkey, HotkeyAction, HotkeyRequest};
pub use tray::{TrayCommand, TrayLabels};

/// Everything the desktop reports to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopEvent {
    Hotkey(HotkeyAction),
    Tray(TrayCommand),
    FocusChanged(ActiveWindow),
    LayoutChanged(XkbNames),
    /// fcitx5 was switched to direct input for the target window.
    ImeSwitchedOff,
}

pub struct DesktopOptions {
    pub tray: Option<TrayLabels>,
    pub switch_input_method: bool,
}

pub struct Desktop {
    runtime: tokio::runtime::Runtime,
    conn: zbus::Connection,
    events: async_channel::Receiver<DesktopEvent>,
    focus: Option<KwinFocus>,
    ime: Arc<fcitx::ImeGuard>,
    shortcuts: Option<shortcuts::Shortcuts>,
    tray: Option<tray::TrayHandle>,
    warnings: Vec<String>,
}

impl Desktop {
    /// Connects to the session bus and starts every available service.
    /// Fails only when there is no session bus at all.
    pub fn start(options: DesktopOptions) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("humanize-desktop")
            .enable_all()
            .build()
            .map_err(|e| format!("starting the desktop runtime: {e}"))?;
        let (tx, events) = async_channel::unbounded();
        let mut warnings = Vec::new();

        let conn = runtime
            .block_on(zbus::Connection::session())
            .map_err(|e| format!("connecting to the session bus: {e}"))?;

        let focus = match runtime.block_on(kwin::start(&conn, tx.clone())) {
            Ok(focus) => Some(focus),
            Err(e) => {
                warnings.push(format!("focus detection off: {e}"));
                None
            }
        };
        let tracker: Arc<dyn FocusTracker> = match &focus {
            Some(f) => Arc::new(f.clone()),
            None => Arc::new(AlwaysFocused),
        };
        let ime = Arc::new(fcitx::ImeGuard::new(
            runtime.handle().clone(),
            conn.clone(),
            tracker,
            options.switch_input_method,
            tx.clone(),
        ));

        let shortcuts = match runtime.block_on(shortcuts::Shortcuts::start(tx.clone())) {
            Ok(s) => Some(s),
            Err(e) => {
                warnings.push(format!("global hotkeys off: {e}"));
                None
            }
        };

        let tray = match options.tray {
            Some(labels) => match runtime.block_on(tray::TrayHandle::spawn(labels, tx.clone())) {
                Ok(t) => Some(t),
                Err(e) => {
                    warnings.push(e);
                    None
                }
            },
            None => None,
        };

        runtime.spawn(layout::watch(conn.clone(), tx));
        Ok(Self { runtime, conn, events, focus, ime, shortcuts, tray, warnings })
    }

    /// Events from all services. Clone freely; each event goes to one receiver.
    pub fn events(&self) -> async_channel::Receiver<DesktopEvent> {
        self.events.clone()
    }

    /// Services that could not start, in plain words.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Focus tracker for sessions: KWin-backed, or "always focused" when
    /// focus detection is unavailable.
    pub fn focus_tracker(&self) -> Arc<dyn FocusTracker> {
        match &self.focus {
            Some(f) => Arc::new(f.clone()),
            None => Arc::new(AlwaysFocused),
        }
    }

    /// Whether focus-loss pausing works on this desktop.
    pub fn has_focus_detection(&self) -> bool {
        self.focus.is_some()
    }

    /// The window KWin reports as active (may be our own).
    pub fn active_window(&self) -> Option<ActiveWindow> {
        self.focus.as_ref().and_then(KwinFocus::active)
    }

    /// Session hooks (fcitx5 switching).
    pub fn session_hooks(&self) -> Arc<dyn SessionHooks> {
        Arc::clone(&self.ime) as Arc<dyn SessionHooks>
    }

    pub fn set_switch_input_method(&self, enabled: bool) {
        self.ime.set_enabled(enabled);
    }

    /// The keyboard layout KDE is using now.
    pub fn active_layout(&self) -> XkbNames {
        self.runtime.block_on(layout::active(&self.conn))
    }

    pub fn has_hotkeys(&self) -> bool {
        self.shortcuts.is_some()
    }

    /// Requests the hotkeys. Blocks while KDE shows its confirmation dialog
    /// (first time only), so call it off the UI thread or accept the wait.
    pub fn bind_hotkeys(&self, requests: &[HotkeyRequest]) -> Result<Vec<BoundHotkey>, String> {
        let shortcuts = self.shortcuts.as_ref().ok_or("global hotkeys are unavailable")?;
        self.runtime.block_on(shortcuts.bind(requests))
    }

    /// Opens KDE's shortcut settings for this app.
    pub fn configure_hotkeys(&self) -> Result<(), String> {
        let shortcuts = self.shortcuts.as_ref().ok_or("global hotkeys are unavailable")?;
        self.runtime.block_on(shortcuts.configure())
    }

    pub fn update_tray(&self, labels: TrayLabels, busy: bool) {
        if let Some(tray) = &self.tray {
            self.runtime.block_on(tray.update(labels, busy));
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // Leave nothing behind in KWin.
        if self.focus.is_some() {
            self.runtime.block_on(kwin::stop(&self.conn));
        }
    }
}
