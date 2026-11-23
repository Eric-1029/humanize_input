//! Active-window tracking on KDE Plasma (Wayland and X11).
//!
//! Wayland gives ordinary clients no way to learn which window is focused,
//! but KWin runs JavaScript extensions. We load a tiny script that calls back
//! over DBus whenever the active window changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use humanize_core::{FocusTracker, WindowId};

use crate::DesktopEvent;

const PLUGIN: &str = "humanize_input_focus";
const FOCUS_PATH: &str = "/dev/humanize_input/Focus";
const FOCUS_INTERFACE: &str = "dev.humanize_input.Focus1";

/// The window KWin reports as active.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveWindow {
    /// KWin's internal UUID; empty when nothing is active.
    pub id: String,
    /// Wayland app id / X11 resource class, e.g. `org.kde.konsole`.
    pub app_id: String,
    pub caption: String,
    pub pid: u32,
}

#[derive(Default)]
struct FocusState {
    active: Option<ActiveWindow>,
    /// App ids of windows seen this run, for `app_id()` lookups.
    apps: HashMap<String, String>,
}

/// [`FocusTracker`] fed by the KWin script. Our own windows never count as
/// a target: if the user leaves focus on humanize_input, nothing is typed.
#[derive(Clone)]
pub struct KwinFocus {
    state: Arc<Mutex<FocusState>>,
    own_pid: u32,
}

impl KwinFocus {
    pub(crate) fn new() -> Self {
        Self { state: Arc::default(), own_pid: std::process::id() }
    }

    /// The active window as last reported, including our own.
    pub fn active(&self) -> Option<ActiveWindow> {
        self.lock().active.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FocusState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, window: ActiveWindow) {
        let mut state = self.lock();
        if !window.id.is_empty() {
            state.apps.insert(window.id.clone(), window.app_id.clone());
        }
        state.active = Some(window);
    }
}

impl FocusTracker for KwinFocus {
    fn current(&self) -> Option<WindowId> {
        self.lock()
            .active
            .as_ref()
            .filter(|w| !w.id.is_empty() && w.pid != self.own_pid)
            .map(|w| WindowId(w.id.clone()))
    }

    fn app_id(&self, window: &WindowId) -> Option<String> {
        self.lock().apps.get(&window.0).filter(|a| !a.is_empty()).cloned()
    }
}

struct FocusSink {
    focus: KwinFocus,
    events: async_channel::Sender<DesktopEvent>,
}

#[zbus::interface(name = "dev.humanize_input.Focus1")]
impl FocusSink {
    /// Called by the KWin script. All arguments are strings because KWin's
    /// `callDBus` marshals JavaScript numbers as doubles.
    async fn activated(&self, id: String, app_id: String, caption: String, pid: String) {
        let window = ActiveWindow { id, app_id, caption, pid: pid.parse().unwrap_or(0) };
        self.focus.update(window.clone());
        let _ = self.events.try_send(DesktopEvent::FocusChanged(window));
    }
}

#[zbus::proxy(
    interface = "org.kde.kwin.Scripting",
    default_service = "org.kde.KWin",
    default_path = "/Scripting"
)]
trait Scripting {
    #[zbus(name = "loadScript")]
    fn load_script(&self, path: &str, plugin: &str) -> zbus::Result<i32>;
    #[zbus(name = "unloadScript")]
    fn unload_script(&self, plugin: &str) -> zbus::Result<bool>;
    #[zbus(name = "start")]
    fn start(&self) -> zbus::Result<()>;
}

fn script(bus_name: &str) -> String {
    format!(
        r#"// Loaded by humanize_input to learn which window has focus.
function report(w) {{
    callDBus("{bus_name}", "{FOCUS_PATH}", "{FOCUS_INTERFACE}", "Activated",
             w ? String(w.internalId) : "",
             w ? String(w.resourceClass || "") : "",
             w ? String(w.caption || "") : "",
             w ? String(w.pid || 0) : "0");
}}
workspace.windowActivated.connect(report);
report(workspace.activeWindow);
"#
    )
}

/// Serves the callback object and loads the script into KWin.
pub(crate) async fn start(
    conn: &zbus::Connection,
    events: async_channel::Sender<DesktopEvent>,
) -> Result<KwinFocus, String> {
    let focus = KwinFocus::new();
    conn.object_server()
        .at(FOCUS_PATH, FocusSink { focus: focus.clone(), events })
        .await
        .map_err(|e| format!("serving the focus object: {e}"))?;
    let bus_name = conn.unique_name().ok_or("no unique DBus name")?.to_string();

    let dir = crate::private_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{PLUGIN}-{}.js", std::process::id()));
    std::fs::write(&path, script(&bus_name)).map_err(|e| format!("writing the KWin script: {e}"))?;

    let scripting =
        ScriptingProxy::new(conn).await.map_err(|e| format!("KWin scripting unavailable: {e}"))?;
    // A previous instance may have crashed without unloading.
    let _ = scripting.unload_script(PLUGIN).await;
    let id = scripting
        .load_script(&path.to_string_lossy(), PLUGIN)
        .await
        .map_err(|e| format!("KWin refused the script: {e}"))?;
    if id < 0 {
        return Err("KWin refused the script".into());
    }
    let run = conn
        .call_method(
            Some("org.kde.KWin"),
            format!("/Scripting/Script{id}").as_str(),
            Some("org.kde.kwin.Script"),
            "run",
            &(),
        )
        .await;
    if run.is_err() {
        scripting.start().await.map_err(|e| format!("starting the KWin script: {e}"))?;
    }
    Ok(focus)
}

pub(crate) async fn stop(conn: &zbus::Connection) {
    if let Ok(scripting) = ScriptingProxy::new(conn).await {
        let _ = scripting.unload_script(PLUGIN).await;
    }
    let dir = crate::private_dir();
    let _ = std::fs::remove_file(dir.join(format!("{PLUGIN}-{}.js", std::process::id())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_windows_and_empty_focus_are_not_targets() {
        let focus = KwinFocus::new();
        assert_eq!(focus.current(), None);
        focus.update(ActiveWindow {
            id: "{a}".into(),
            app_id: "org.kde.kate".into(),
            caption: "x".into(),
            pid: 42,
        });
        assert_eq!(focus.current(), Some(WindowId("{a}".into())));
        assert_eq!(focus.app_id(&WindowId("{a}".into())).as_deref(), Some("org.kde.kate"));

        focus.update(ActiveWindow { id: "{me}".into(), pid: std::process::id(), ..ActiveWindow::default() });
        assert_eq!(focus.current(), None);
        focus.update(ActiveWindow::default());
        assert_eq!(focus.current(), None);
        // Earlier windows keep their app id.
        assert_eq!(focus.app_id(&WindowId("{a}".into())).as_deref(), Some("org.kde.kate"));
    }

    #[test]
    fn script_embeds_our_bus_name() {
        let js = script(":1.234");
        assert!(js.contains(r#"callDBus(":1.234", "/dev/humanize_input/Focus""#));
        assert!(js.contains("workspace.windowActivated.connect(report)"));
    }
}
