//! Global hotkeys through the XDG GlobalShortcuts portal.
//!
//! The app only *suggests* key combinations; KDE asks the user to confirm
//! them once and afterwards they are managed in System Settings → Shortcuts.

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::{CreateSessionOptions, Session};
use futures_util::StreamExt;

use crate::DesktopEvent;

/// Application id used to identify us to the portal and KDE.
pub const APP_ID: &str = "dev.humanize_input.HumanizeInput";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    Start,
    PauseResume,
    Cancel,
}

impl HotkeyAction {
    pub const ALL: [Self; 3] = [Self::Start, Self::PauseResume, Self::Cancel];

    fn id(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::PauseResume => "pause-resume",
            Self::Cancel => "cancel",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }
}

/// What the user wants each hotkey to be, in portal trigger syntax
/// (`CTRL+ALT+S`), plus the description KDE shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRequest {
    pub action: HotkeyAction,
    pub description: String,
    pub preferred: String,
}

/// A hotkey as the desktop actually bound it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundHotkey {
    pub action: HotkeyAction,
    /// Human-readable trigger from KDE, e.g. "Ctrl+Alt+S"; empty if unset.
    pub trigger: String,
}

pub(crate) struct Shortcuts {
    portal: GlobalShortcuts,
    session: Session<GlobalShortcuts>,
}

impl Shortcuts {
    pub(crate) async fn start(events: async_channel::Sender<DesktopEvent>) -> Result<Self, String> {
        // Non-sandboxed apps must name themselves before using portals.
        if let Ok(app_id) = APP_ID.try_into() {
            let _ = ashpd::register_host_app(app_id).await;
        }
        let portal = GlobalShortcuts::new().await.map_err(|e| format!("GlobalShortcuts portal: {e}"))?;
        let session = portal
            .create_session(CreateSessionOptions::default())
            .await
            .map_err(|e| format!("creating a shortcuts session: {e}"))?;

        let mut activated =
            portal.receive_activated().await.map_err(|e| format!("listening for shortcuts: {e}"))?;
        tokio::spawn(async move {
            while let Some(event) = activated.next().await {
                if let Some(action) = HotkeyAction::from_id(event.shortcut_id())
                    && events.send(DesktopEvent::Hotkey(action)).await.is_err()
                {
                    return;
                }
            }
        });
        Ok(Self { portal, session })
    }

    /// Asks the desktop to bind the hotkeys. The first time, KDE shows a
    /// confirmation dialog; later calls return the current bindings.
    pub(crate) async fn bind(&self, requests: &[HotkeyRequest]) -> Result<Vec<BoundHotkey>, String> {
        let shortcuts: Vec<NewShortcut> = requests
            .iter()
            .map(|r| {
                NewShortcut::new(r.action.id(), r.description.as_str())
                    .preferred_trigger(r.preferred.as_str())
            })
            .collect();
        let response = self
            .portal
            .bind_shortcuts(&self.session, &shortcuts, None, BindShortcutsOptions::default())
            .await
            .and_then(|request| request.response())
            .map_err(|e| format!("binding shortcuts: {e}"))?;
        Ok(response
            .shortcuts()
            .iter()
            .filter_map(|s| {
                HotkeyAction::from_id(s.id())
                    .map(|action| BoundHotkey { action, trigger: s.trigger_description().to_owned() })
            })
            .collect())
    }

    /// Opens the desktop's shortcut settings for this app (portal v2).
    pub(crate) async fn configure(&self) -> Result<(), String> {
        self.portal
            .configure_shortcuts(&self.session, None, Default::default())
            .await
            .map_err(|e| format!("opening shortcut settings: {e}"))
    }
}
