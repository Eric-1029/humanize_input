//! System tray icon (StatusNotifierItem), shown by Plasma's system tray.

use ksni::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};

use crate::DesktopEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ShowWindow,
    Start,
    PauseResume,
    Cancel,
    Quit,
}

/// Menu texts, supplied by the UI so they follow its language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub title: String,
    pub status: String,
    pub show: String,
    pub start: String,
    pub pause_resume: String,
    pub cancel: String,
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        Self {
            title: "humanize_input".into(),
            status: String::new(),
            show: "Show window".into(),
            start: "Start typing".into(),
            pause_resume: "Pause / resume".into(),
            cancel: "Cancel".into(),
            quit: "Quit".into(),
        }
    }
}

struct AppTray {
    labels: TrayLabels,
    busy: bool,
    events: async_channel::Sender<DesktopEvent>,
}

impl AppTray {
    fn item(&self, label: &str, enabled: bool, command: TrayCommand) -> MenuItem<Self> {
        StandardItem {
            label: label.to_owned(),
            enabled,
            activate: Box::new(move |tray: &mut Self| {
                let _ = tray.events.try_send(DesktopEvent::Tray(command));
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        "humanize_input".into()
    }

    fn title(&self) -> String {
        self.labels.title.clone()
    }

    fn icon_name(&self) -> String {
        if self.busy { "input-keyboard-virtual-on" } else { "input-keyboard" }.into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.labels.title.clone(),
            description: self.labels.status.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.try_send(DesktopEvent::Tray(TrayCommand::ShowWindow));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            self.item(&self.labels.show, true, TrayCommand::ShowWindow),
            MenuItem::Separator,
            self.item(&self.labels.start, !self.busy, TrayCommand::Start),
            self.item(&self.labels.pause_resume, self.busy, TrayCommand::PauseResume),
            self.item(&self.labels.cancel, self.busy, TrayCommand::Cancel),
            MenuItem::Separator,
            self.item(&self.labels.quit, true, TrayCommand::Quit),
        ]
    }
}

pub(crate) struct TrayHandle(ksni::Handle<AppTray>);

impl TrayHandle {
    pub(crate) async fn spawn(
        labels: TrayLabels,
        events: async_channel::Sender<DesktopEvent>,
    ) -> Result<Self, String> {
        AppTray { labels, busy: false, events }.spawn().await.map(Self).map_err(|e| format!("tray icon: {e}"))
    }

    pub(crate) async fn update(&self, labels: TrayLabels, busy: bool) {
        self.0
            .update(move |tray| {
                tray.labels = labels;
                tray.busy = busy;
            })
            .await;
    }
}
