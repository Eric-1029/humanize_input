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

/// Portals identify non-sandboxed apps by a desktop file named after the app
/// id; without one, KDE refuses with "An app id is required". Packages ship
/// the file in /usr/share/applications; when running from a build tree, write
/// a hidden one to ~/.local/share/applications. Returns the file written.
pub fn ensure_desktop_entry() -> std::io::Result<Option<std::path::PathBuf>> {
    use std::path::{Path, PathBuf};
    let file = format!("{APP_ID}.desktop");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let exists = data_home
        .iter()
        .cloned()
        .chain(data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .any(|dir| dir.join("applications").join(&file).exists());
    if exists {
        return Ok(None);
    }
    let Some(dir) = data_home.map(|d| d.join("applications")) else { return Ok(None) };
    let exe = std::env::current_exe()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=humanize_input\nComment=Types text like a person through a virtual keyboard\n\
         Exec={}\nIcon=input-keyboard\nTerminal=false\nNoDisplay=true\n\
         X-Comment=Written by a development build so desktop portals recognise the app.\n",
        Path::new(&exe).display()
    );
    std::fs::write(&path, entry)?;
    Ok(Some(path))
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
