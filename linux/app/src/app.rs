//! Application state, session wiring and the three pages.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use humanize_core::{
    ClassicKeys, HumanPlanner, KeyAction, Layout, LogEvent, Macros, Planner, Preset, Session, SessionEvent,
    SessionState, StartError, TypingMode, TypingProgress, WordDelete,
};
use humanize_desktop::config::{Language, ModelKind, Theme};
use humanize_desktop::{
    BoundHotkey, Config, Desktop, DesktopEvent, DesktopOptions, HotkeyAction, HotkeyRequest, TrayCommand,
    TrayLabels,
};
use humanize_input::{DeviceIdentity, DryRunDriver, UinputKeyboard, caps_lock_on};

use crate::cut::{CUT_SMALL, CutBin};
use crate::i18n::{Tr, clock, count};
use crate::params;
use crate::widgets::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Input,
    Tuning,
    Settings,
}

/// Start-up options (mostly for development and screenshots).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Render the window to this PNG after start-up, then quit.
    pub screenshot: Option<PathBuf>,
    pub theme: Option<Theme>,
    pub page: Option<Page>,
    pub language: Option<Language>,
    /// Type the sample text into a dry-run driver (nothing reaches the system).
    pub demo: bool,
    /// Do not start desktop services or the virtual keyboard.
    pub offline: bool,
    /// Load this file into the text box at start.
    pub text_file: Option<PathBuf>,
}

enum Keyboard {
    Pending,
    Ready(String),
    Failed(String),
}

#[derive(Default)]
struct Run {
    state: Option<SessionState>,
    progress: TypingProgress,
    started: Option<Instant>,
    target: Option<String>,
    problem: Option<String>,
}

struct Ui {
    nav: Vec<(Page, gtk::Button)>,
    stack: gtk::Stack,
    layout_readout: gtk::Label,
    source_fields: gtk::Label,
    unsupported: (gtk::Box, gtk::Label),
    text_view: gtk::TextView,
    state_chip: gtk::Label,
    typed: gtk::Label,
    total: gtk::Label,
    cells: Vec<gtk::Box>,
    mistakes: gtk::Label,
    fixes: gtk::Label,
    elapsed: gtk::Label,
    remaining: gtk::Label,
    detail: gtk::Label,
    target: gtk::Label,
    start: gtk::Button,
    start_sub: gtk::Label,
    pause: gtk::Button,
    cancel: gtk::Button,
    session_notice: (gtk::Box, gtk::Label),
    profile_fields: gtk::Label,
    hotkey_values: Vec<(HotkeyAction, gtk::Label)>,
    hotkey_notice: (gtk::Box, gtk::Label),
    log_list: gtk::Box,
    log_count: gtk::Label,
}

const SAMPLE: &str = "Dear team,\n\nThe quarterly report is ready for review. Please check the figures in section 3 before Friday; \
the totals changed after we merged the March and April data.\n\nThanks, Alex";

pub struct App {
    gtk_app: gtk::Application,
    window: gtk::ApplicationWindow,
    options: Options,
    config: RefCell<Config>,
    config_path: Option<PathBuf>,
    desktop: Option<Arc<Desktop>>,
    session: RefCell<Option<Rc<Session>>>,
    keyboard: RefCell<Keyboard>,
    layout: RefCell<Arc<Layout>>,
    layout_label: RefCell<String>,
    buffer: gtk::TextBuffer,
    ui: RefCell<Option<Ui>>,
    page: Cell<Page>,
    run: RefCell<Run>,
    log: RefCell<Vec<(String, String)>>,
    hotkeys: RefCell<Vec<BoundHotkey>>,
    hotkey_error: RefCell<Option<String>>,
    estimate: Cell<Option<Duration>>,
    session_tx: async_channel::Sender<SessionEvent>,
    session_rx: async_channel::Receiver<SessionEvent>,
    save_timer: RefCell<Option<glib::SourceId>>,
    estimate_timer: RefCell<Option<glib::SourceId>>,
    /// The virtual-keyboard readout on the Settings page.
    keyboard_readout: RefCell<Option<gtk::Label>>,
}

impl App {
    pub fn new(gtk_app: &gtk::Application, options: Options) -> Rc<Self> {
        let in_memory = options.screenshot.is_some() || options.demo;
        let config_path = if in_memory { None } else { Config::default_path() };
        let first_run = config_path.as_ref().is_some_and(|p| !p.exists());
        let mut config = config_path.as_ref().and_then(|p| Config::load(p).ok()).unwrap_or_default();
        if let Some(theme) = options.theme {
            config.theme = theme;
        }
        if let Some(language) = options.language {
            config.language = language;
        }

        let desktop = if options.offline || options.demo {
            None
        } else {
            match Desktop::start(DesktopOptions {
                tray: Some(TrayLabels::default()),
                switch_input_method: config.switch_input_method,
            }) {
                Ok(d) => Some(Arc::new(d)),
                Err(e) => {
                    eprintln!("desktop services unavailable: {e}");
                    None
                }
            }
        };

        let names = desktop
            .as_ref()
            .map(|d| d.active_layout())
            .unwrap_or_else(|| humanize_keymap::XkbNames::from_kde(0));
        let (layout, layout_label) = match humanize_keymap::load(&names) {
            Ok(l) => (l, names.label()),
            Err(e) => {
                eprintln!("{e}; falling back to built-in US layout");
                (Layout::us_qwerty(), "us".to_owned())
            }
        };

        let window = gtk::ApplicationWindow::builder()
            .application(gtk_app)
            .title("humanize_input")
            .default_width(1240)
            .default_height(860)
            .build();
        window.set_size_request(980, 640);
        if options.screenshot.is_some() {
            window.set_default_size(1280, 1500);
        }
        window.add_css_class("ef");

        let buffer = gtk::TextBuffer::new(None);
        let tags = buffer.tag_table();
        tags.add(
            &gtk::TextTag::builder().name("unsupported").background("#FFFA00").foreground("#161616").build(),
        );
        tags.add(&gtk::TextTag::builder().name("pending").foreground("#8A8A8A").build());
        tags.add(&gtk::TextTag::builder().name("cursor").background("#FFFA00").foreground("#161616").build());
        if let Some(path) = &options.text_file {
            match std::fs::read_to_string(path) {
                Ok(text) => buffer.set_text(&text),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
        } else if options.demo || options.screenshot.is_some() {
            buffer.set_text(SAMPLE);
        }

        let (session_tx, session_rx) = async_channel::unbounded();
        let app = Rc::new(Self {
            gtk_app: gtk_app.clone(),
            window,
            options,
            config: RefCell::new(config),
            config_path,
            desktop,
            session: RefCell::new(None),
            keyboard: RefCell::new(Keyboard::Pending),
            layout: RefCell::new(Arc::new(layout)),
            layout_label: RefCell::new(layout_label),
            buffer,
            ui: RefCell::new(None),
            page: Cell::new(Page::Input),
            run: RefCell::new(Run::default()),
            log: RefCell::new(Vec::new()),
            hotkeys: RefCell::new(Vec::new()),
            hotkey_error: RefCell::new(None),
            estimate: Cell::new(None),
            session_tx,
            session_rx,
            save_timer: RefCell::new(None),
            estimate_timer: RefCell::new(None),
            keyboard_readout: RefCell::new(None),
        });
        if let Some(page) = app.options.page {
            app.page.set(page);
        }
        app.wire(first_run);
        app
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Stops typing (lifting every key) and quits; used for signals too.
    pub fn quit(&self) {
        self.cancel_typing();
        self.gtk_app.quit();
    }

    fn tr(&self) -> Tr {
        Tr(self.config.borrow().language)
    }

    // ── Start-up ────────────────────────────────────────────────────────

    fn wire(self: &Rc<Self>, first_run: bool) {
        self.apply_theme();
        self.build();

        {
            let app = Rc::clone(self);
            self.buffer.connect_changed(move |_| app.text_changed());
        }
        {
            let app = Rc::clone(self);
            self.window.connect_close_request(move |_| {
                if app.desktop.is_some() && app.options.screenshot.is_none() {
                    app.window.set_visible(false);
                    app.add_log(&app.tr().t(
                        "窗口已收到托盘；从托盘图标可以重新打开。",
                        "Window hidden to the tray; open it again from the tray icon.",
                    ));
                    glib::Propagation::Stop
                } else {
                    app.gtk_app.quit();
                    glib::Propagation::Proceed
                }
            });
        }

        // Session events → UI.
        {
            let app = Rc::clone(self);
            let rx = self.session_rx.clone();
            glib::spawn_future_local(async move {
                while let Ok(event) = rx.recv().await {
                    app.on_session_event(event);
                }
            });
        }
        // Desktop events → UI.
        if let Some(desktop) = &self.desktop {
            let app = Rc::clone(self);
            let rx = desktop.events();
            glib::spawn_future_local(async move {
                while let Ok(event) = rx.recv().await {
                    app.on_desktop_event(event);
                }
            });
            for warning in desktop.warnings() {
                self.add_log(warning);
            }
        }

        self.start_keyboard();
        self.bind_hotkeys();

        // Once a second while typing: elapsed and remaining time.
        {
            let app = Rc::clone(self);
            glib::timeout_add_local(Duration::from_millis(500), move || {
                if app.run.borrow().state.is_some_and(SessionState::is_busy) {
                    app.refresh_session();
                }
                glib::ControlFlow::Continue
            });
        }

        self.text_changed();
        let hidden = self.config.borrow().start_minimized && !first_run && self.desktop.is_some();
        if self.options.screenshot.is_some() || !hidden {
            self.window.present();
        }
        if first_run {
            self.schedule_save();
        }
        if let Some(path) = self.options.screenshot.clone() {
            let app = Rc::clone(self);
            let delay = if self.options.demo { 9000 } else { 1200 };
            glib::timeout_add_local_once(Duration::from_millis(delay), move || app.screenshot(&path));
        }
    }

    fn start_keyboard(self: &Rc<Self>) {
        if self.options.demo {
            *self.keyboard.borrow_mut() = Keyboard::Ready("dry-run".into());
            self.make_session(Box::new(DryRunDriver::new(false)));
            let app = Rc::clone(self);
            glib::timeout_add_local_once(Duration::from_millis(300), move || app.start_typing());
            return;
        }
        if self.options.offline {
            *self.keyboard.borrow_mut() = Keyboard::Failed(
                self.tr().t("离线模式，未创建虚拟键盘。", "Offline mode: no virtual keyboard."),
            );
            return;
        }
        let device = self.config.borrow().device.clone();
        let identity = DeviceIdentity {
            name: device.name,
            vendor: device.vendor,
            product: device.product,
            version: device.version,
            phys: None,
            polling_hz: device.polling_hz,
        };
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(UinputKeyboard::create(&identity).map_err(|e| e.to_string()));
        });
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(keyboard)) => {
                    *app.keyboard.borrow_mut() = Keyboard::Ready(keyboard.sysname().to_owned());
                    app.make_session(Box::new(keyboard));
                }
                Ok(Err(e)) => *app.keyboard.borrow_mut() = Keyboard::Failed(e),
                Err(_) => {}
            }
            app.refresh_all();
        });
    }

    fn make_session(&self, driver: Box<dyn humanize_core::Driver>) {
        // Debugging aid: HUMANIZE_INPUT_TRACE=FILE records every key event sent.
        let driver: Box<dyn humanize_core::Driver> = match std::env::var_os("HUMANIZE_INPUT_TRACE") {
            Some(path) => match TracingDriver::new(driver, std::path::Path::new(&path)) {
                Ok(t) => Box::new(t),
                Err((driver, e)) => {
                    eprintln!("trace disabled: {e}");
                    driver
                }
            },
            None => driver,
        };
        let tx = self.session_tx.clone();
        let focus = match &self.desktop {
            Some(d) => d.focus_tracker(),
            None => Arc::new(humanize_core::AlwaysFocused),
        };
        let mut session = Session::new(
            driver,
            focus,
            Arc::clone(&self.layout.borrow()),
            Arc::new(move |e| {
                let _ = tx.try_send(e);
            }),
        );
        if let Some(d) = &self.desktop {
            session = session.with_hooks(d.session_hooks());
        }
        *self.session.borrow_mut() = Some(Rc::new(session));
    }

    fn hotkey_requests(&self) -> Vec<HotkeyRequest> {
        let tr = self.tr();
        let c = self.config.borrow();
        vec![
            HotkeyRequest {
                action: HotkeyAction::Start,
                description: tr.t("开始输入", "Start typing"),
                preferred: c.hotkeys.start.clone(),
            },
            HotkeyRequest {
                action: HotkeyAction::PauseResume,
                description: tr.t("暂停 / 继续", "Pause / resume"),
                preferred: c.hotkeys.pause_resume.clone(),
            },
            HotkeyRequest {
                action: HotkeyAction::Cancel,
                description: tr.t("取消输入", "Cancel typing"),
                preferred: c.hotkeys.cancel.clone(),
            },
        ]
    }

    fn bind_hotkeys(self: &Rc<Self>) {
        // Screenshots run outside our own systemd scope; binding there would
        // file the hotkeys under the launching app.
        if self.options.screenshot.is_some() {
            return;
        }
        let Some(desktop) = self.desktop.clone() else { return };
        if !desktop.has_hotkeys() {
            *self.hotkey_error.borrow_mut() = Some(self.tr().t(
                "这个桌面不提供全局热键（GlobalShortcuts 门户）。",
                "This desktop offers no global hotkeys (GlobalShortcuts portal).",
            ));
            return;
        }
        let requests = self.hotkey_requests();
        let (tx, rx) = async_channel::bounded(1);
        // KDE may show a confirmation dialog; keep the UI responsive meanwhile.
        std::thread::spawn(move || {
            let _ = tx.send_blocking(desktop.bind_hotkeys(&requests));
        });
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                match result {
                    Ok(bound) => {
                        *app.hotkeys.borrow_mut() = bound;
                        *app.hotkey_error.borrow_mut() = None;
                    }
                    Err(e) => *app.hotkey_error.borrow_mut() = Some(e),
                }
                app.refresh_hotkeys();
            }
        });
    }

    // ── Persistence ─────────────────────────────────────────────────────

    fn schedule_save(self: &Rc<Self>) {
        if self.config_path.is_none() {
            return;
        }
        if let Some(id) = self.save_timer.borrow_mut().take() {
            id.remove();
        }
        let app = Rc::clone(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(600), move || {
            app.save_timer.borrow_mut().take();
            if let Some(path) = &app.config_path
                && let Err(e) = app.config.borrow().save(path)
            {
                app.add_log(&format!("{}: {e}", app.tr().t("保存设置失败", "Could not save settings")));
            }
        });
        *self.save_timer.borrow_mut() = Some(id);
    }

    fn update_config(self: &Rc<Self>, change: impl FnOnce(&mut Config)) {
        change(&mut self.config.borrow_mut());
        self.schedule_save();
        self.schedule_estimate();
        self.refresh_profile();
    }

    // ── Text ────────────────────────────────────────────────────────────

    fn text(&self) -> String {
        let (start, end) = self.buffer.bounds();
        self.buffer.text(&start, &end, false).to_string()
    }

    fn unsupported_count(&self) -> usize {
        self.layout.borrow().find_unsupported(&self.text()).len()
    }

    fn text_changed(self: &Rc<Self>) {
        let text = self.text();
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag_by_name("unsupported", &start, &end);
        let unsupported = self.layout.borrow().find_unsupported(&text);
        for u in &unsupported {
            let a = self.buffer.iter_at_offset(u.char_index as i32);
            let b = self.buffer.iter_at_offset(u.char_index as i32 + 1);
            self.buffer.apply_tag_by_name("unsupported", &a, &b);
        }
        self.schedule_estimate();
        self.refresh_source();
        self.refresh_session();
    }

    fn schedule_estimate(self: &Rc<Self>) {
        if let Some(id) = self.estimate_timer.borrow_mut().take() {
            id.remove();
        }
        let app = Rc::clone(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(350), move || {
            app.estimate_timer.borrow_mut().take();
            app.estimate.set(app.compute_estimate());
            app.refresh_source();
            app.refresh_session();
        });
        *self.estimate_timer.borrow_mut() = Some(id);
    }

    /// Expected typing time from a deterministic dry run of the current model.
    fn compute_estimate(&self) -> Option<Duration> {
        let text = self.text();
        if text.trim().is_empty() || self.unsupported_count() > 0 {
            return None;
        }
        let layout = Arc::clone(&self.layout.borrow());
        let mode = self.config.borrow().typing_mode();
        let actions: Box<dyn Iterator<Item = KeyAction>> = match mode {
            TypingMode::Human(model) => Box::new(HumanPlanner::with_seed(&text, model, layout, 1)),
            TypingMode::Classic(settings) => {
                let geometry = Arc::new(layout.classic_geometry());
                Box::new(ClassicKeys::new(Planner::with_seed(&text, settings, geometry, 1), layout))
            }
        };
        Some(actions.filter_map(|a| if let KeyAction::Wait(d) = a { Some(d) } else { None }).sum())
    }

    fn mark_progress(&self, typed: usize) {
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag_by_name("cursor", &start, &end);
        let cut = self.buffer.iter_at_offset(typed as i32);
        self.buffer.remove_tag_by_name("pending", &start, &cut);
        let mut next = cut;
        if next.forward_char() {
            self.buffer.apply_tag_by_name("cursor", &cut, &next);
        }
        if let Some(ui) = self.ui.borrow().as_ref() {
            let mut at = cut;
            ui.text_view.scroll_to_iter(&mut at, 0.1, false, 0.0, 0.0);
        }
    }

    fn clear_marks(&self) {
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag_by_name("pending", &start, &end);
        self.buffer.remove_tag_by_name("cursor", &start, &end);
    }

    // ── Actions ─────────────────────────────────────────────────────────

    fn start_typing(self: &Rc<Self>) {
        let tr = self.tr();
        let Some(session) = self.session.borrow().clone() else {
            self.set_problem(Some(tr.t("虚拟键盘还没准备好。", "The virtual keyboard is not ready yet.")));
            return;
        };
        if session.state().is_busy() {
            return;
        }
        if !self.options.demo && caps_lock_on() {
            self.set_problem(Some(tr.t(
                "大写锁定已打开，字母大小写会反过来。请先关掉再开始。",
                "Caps Lock is on, which would invert letter case. Turn it off first.",
            )));
            return;
        }
        let text = self.text();
        let mode = self.config.borrow().typing_mode();
        let result =
            if self.options.demo { session.start_seeded(&text, mode, 3) } else { session.start(&text, mode) };
        match result {
            Ok(()) => {
                self.set_problem(None);
                let lead = mode.lead_in().as_secs_f64();
                self.add_log(&if tr.zh() {
                    format!("{lead:.1} 秒后开始，请切换到要输入的窗口。")
                } else {
                    format!("Typing starts in {lead:.1} s; switch to the target window.")
                });
            }
            Err(StartError::EmptyText) => {
                self.set_problem(Some(tr.t("没有要输入的文字。", "There is no text to type.")))
            }
            Err(StartError::UnsupportedChars(n)) => self.set_problem(Some(if tr.zh() {
                format!("有 {n} 个字符无法输入，已在原文里标出。")
            } else {
                format!("{n} characters cannot be typed; they are marked in the text.")
            })),
            Err(StartError::Busy) => {}
        }
    }

    fn toggle_pause(&self) {
        if let Some(s) = self.session.borrow().as_ref() {
            s.toggle_pause();
        }
    }

    fn cancel_typing(&self) {
        if let Some(s) = self.session.borrow().as_ref() {
            s.cancel();
        }
    }

    fn set_problem(&self, problem: Option<String>) {
        self.run.borrow_mut().problem = problem;
        self.refresh_session();
    }

    fn add_log(&self, message: &str) {
        let time = glib::DateTime::now_local()
            .ok()
            .and_then(|t| t.format("%H:%M:%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let mut log = self.log.borrow_mut();
        log.insert(0, (time, message.to_owned()));
        log.truncate(60);
        drop(log);
        self.refresh_log();
    }

    // ── Events ──────────────────────────────────────────────────────────

    fn on_session_event(self: &Rc<Self>, event: SessionEvent) {
        let tr = self.tr();
        match event {
            SessionEvent::State(state) => {
                let previous = self.run.borrow().state;
                self.run.borrow_mut().state = Some(state);
                match state {
                    SessionState::Arming => {
                        let mut run = self.run.borrow_mut();
                        run.progress = TypingProgress::default();
                        run.started = None;
                        run.target = None;
                    }
                    SessionState::Running if previous == Some(SessionState::Arming) => {
                        let target = self.desktop.as_ref().and_then(|d| d.active_window()).map(|w| w.app_id);
                        {
                            let mut run = self.run.borrow_mut();
                            run.started = Some(Instant::now());
                            run.target = target.clone();
                        }
                        let (start, end) = self.buffer.bounds();
                        self.buffer.apply_tag_by_name("pending", &start, &end);
                        self.mark_progress(0);
                        let target = target
                            .filter(|t| !t.is_empty())
                            .unwrap_or_else(|| tr.t("当前窗口", "the focused window"));
                        self.add_log(&if tr.zh() {
                            format!("开始输入，目标：{target}。")
                        } else {
                            format!("Typing into {target}.")
                        });
                    }
                    SessionState::Completed => {
                        self.clear_marks();
                        let p = self.run.borrow().progress;
                        let secs = self.run.borrow().started.map_or(0, |s| s.elapsed().as_secs());
                        self.add_log(&if tr.zh() {
                            format!("输入完成：{} 个字符，用时 {}。", count(p.typed_chars), clock(secs))
                        } else {
                            format!("Done: {} characters in {}.", count(p.typed_chars), clock(secs))
                        });
                    }
                    SessionState::Idle | SessionState::Faulted => self.clear_marks(),
                    _ => {}
                }
                self.refresh_all();
            }
            SessionEvent::Progress(p) => {
                self.run.borrow_mut().progress = p;
                self.mark_progress(p.typed_chars);
                self.refresh_session();
            }
            SessionEvent::Log(log) => {
                let message = match log {
                    LogEvent::Paused => tr.t("已暂停，所有按键已抬起。", "Paused; all keys released."),
                    LogEvent::Resumed => tr.t("继续输入。", "Resumed."),
                    LogEvent::Canceled => tr.t("已取消。", "Canceled."),
                    LogEvent::FocusLost => tr.t(
                        "目标窗口失去焦点，等它回来后自动继续。",
                        "The target window lost focus; typing resumes when it is back.",
                    ),
                    LogEvent::FocusRestored => {
                        tr.t("目标窗口回到前台，继续输入。", "The target window is back; continuing.")
                    }
                    LogEvent::NoTargetWindow => {
                        let m = tr.t("倒计时结束时没有可输入的窗口（焦点还在本程序上）。请切换到目标窗口后再开始。", "No target window when the countdown ended (focus was still on this app). Switch to the target window and start again.");
                        self.set_problem(Some(m.clone()));
                        m
                    }
                    LogEvent::Completed => return,
                    LogEvent::DriverFailed(e) => {
                        let m = format!("{}: {e}", tr.t("虚拟键盘出错", "Virtual keyboard failed"));
                        self.set_problem(Some(m.clone()));
                        m
                    }
                };
                self.add_log(&message);
            }
        }
    }

    fn on_desktop_event(self: &Rc<Self>, event: DesktopEvent) {
        let tr = self.tr();
        match event {
            DesktopEvent::Hotkey(HotkeyAction::Start) | DesktopEvent::Tray(TrayCommand::Start) => {
                self.start_typing()
            }
            DesktopEvent::Hotkey(HotkeyAction::PauseResume)
            | DesktopEvent::Tray(TrayCommand::PauseResume) => {
                self.toggle_pause();
            }
            DesktopEvent::Hotkey(HotkeyAction::Cancel) | DesktopEvent::Tray(TrayCommand::Cancel) => {
                self.cancel_typing()
            }
            DesktopEvent::Tray(TrayCommand::ShowWindow) => self.window.present(),
            DesktopEvent::Tray(TrayCommand::Quit) => {
                self.cancel_typing();
                self.gtk_app.quit();
            }
            DesktopEvent::FocusChanged(_) => {}
            DesktopEvent::LayoutChanged(names) => match humanize_keymap::load(&names) {
                Ok(layout) => {
                    let layout = Arc::new(layout);
                    if let Some(s) = self.session.borrow().as_ref()
                        && s.set_layout(Arc::clone(&layout)).is_err()
                    {
                        self.add_log(&tr.t(
                            "输入中切换了键盘布局；本次仍按原布局输入。",
                            "Layout changed while typing; this run keeps the old layout.",
                        ));
                    }
                    *self.layout.borrow_mut() = layout;
                    *self.layout_label.borrow_mut() = names.label();
                    self.add_log(&if tr.zh() {
                        format!("键盘布局已切换为 {}。", names.label())
                    } else {
                        format!("Keyboard layout is now {}.", names.label())
                    });
                    self.text_changed();
                    self.refresh_all();
                }
                Err(e) => self.add_log(&e.to_string()),
            },
            DesktopEvent::ImeSwitchedOff => {
                self.add_log(&tr.t(
                    "已把目标窗口的输入法切到英文，结束后切回。",
                    "Switched the target's input method to direct input; it is restored afterwards.",
                ));
            }
        }
    }

    // ── Theme and screenshots ───────────────────────────────────────────

    fn apply_theme(&self) {
        self.window.remove_css_class("light");
        self.window.remove_css_class("dark");
        match self.config.borrow().theme {
            Theme::System => {}
            Theme::Light => self.window.add_css_class("light"),
            Theme::Dark => self.window.add_css_class("dark"),
        }
    }

    fn screenshot(&self, path: &std::path::Path) {
        let widget = self.window.upcast_ref::<gtk::Widget>();
        let paintable = gtk::WidgetPaintable::new(Some(widget));
        let (w, h) = (widget.width() as f64, widget.height() as f64);
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w, h);
        let saved = snapshot
            .to_node()
            .zip(widget.native().and_then(|n| n.renderer()))
            .map(|(node, renderer)| renderer.render_texture(&node, None).save_to_png(path));
        match saved {
            Some(Ok(())) => println!("screenshot: {}", path.display()),
            Some(Err(e)) => eprintln!("screenshot failed: {e}"),
            None => eprintln!("screenshot failed: nothing rendered"),
        }
        self.gtk_app.quit();
    }

    // ── Building ────────────────────────────────────────────────────────

    fn build(self: &Rc<Self>) {
        let tr = self.tr();
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(200);

        let (titlebar, nav, layout_readout) = self.build_titlebar(&stack);
        self.window.set_titlebar(Some(&titlebar));

        let mut ui = self.build_input_page(&stack, nav, layout_readout);
        stack.add_named(&scrolled(&self.build_tuning_page()), Some("tuning"));
        stack.add_named(&scrolled(&self.build_settings_page()), Some("settings"));
        ui.stack = stack.clone();

        let root = vbox(0);
        root.add_css_class("ground");
        root.append(&stack);
        stack.set_vexpand(true);
        self.window.set_child(Some(&root));
        *self.ui.borrow_mut() = Some(ui);
        self.show_page(self.page.get());
        self.refresh_all();
        if let Some(d) = &self.desktop {
            d.update_tray(self.tray_labels(), false);
        }
        let _ = tr;
    }

    fn rebuild(self: &Rc<Self>) {
        self.ui.borrow_mut().take();
        self.build();
        self.bind_hotkeys();
    }

    fn show_page(&self, page: Page) {
        self.page.set(page);
        if let Some(ui) = self.ui.borrow().as_ref() {
            ui.stack.set_visible_child_name(match page {
                Page::Input => "input",
                Page::Tuning => "tuning",
                Page::Settings => "settings",
            });
            for (p, button) in &ui.nav {
                if *p == page {
                    button.add_css_class("current");
                } else {
                    button.remove_css_class("current");
                }
            }
        }
    }

    fn build_titlebar(
        self: &Rc<Self>,
        _stack: &gtk::Stack,
    ) -> (gtk::Widget, Vec<(Page, gtk::Button)>, gtk::Label) {
        let tr = self.tr();
        let column = vbox(0);
        let tri = hbox(0);
        tri.add_css_class("tri");
        tri.set_homogeneous(true);
        for c in ["a", "b", "c"] {
            let part = hbox(0);
            part.add_css_class(c);
            tri.append(&part);
        }
        column.append(&tri);

        let bar = hbox(0);
        bar.add_css_class("cmdbar");
        let seal = label("H", &["seal"]);
        seal.set_xalign(0.5);
        seal.set_valign(gtk::Align::Center);
        bar.append(&CutBin::new(&seal, CUT_SMALL));
        let brand = label("HUMANIZE INPUT", &["brand"]);
        brand.set_valign(gtk::Align::Center);
        bar.append(&brand);

        let mut nav = Vec::new();
        for (page, zh, en) in [
            (Page::Input, "输入", "Input"),
            (Page::Tuning, "参数", "Tuning"),
            (Page::Settings, "设置", "Settings"),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("nav");
            let col = vbox(0);
            col.append(&label(if tr.zh() { zh } else { en }, &["zh"]));
            col.append(&label(&en.to_uppercase(), &["en"]));
            b.set_child(Some(&col));
            let app = Rc::clone(self);
            b.connect_clicked(move |_| app.show_page(page));
            let wrap = CutBin::new(&b, CUT_SMALL);
            wrap.set_valign(gtk::Align::Center);
            bar.append(&wrap);
            nav.push((page, b));
        }

        bar.append(&spacer());
        let layout_readout = label("", &["readout"]);
        layout_readout.set_valign(gtk::Align::Center);
        bar.append(&layout_readout);

        let lang = gtk::Button::with_label(if tr.zh() { "EN" } else { "中文" });
        lang.add_css_class("quiet");
        lang.set_tooltip_text(Some(&tr.t("切换到英文界面", "Switch to Chinese")));
        {
            let app = Rc::clone(self);
            lang.connect_clicked(move |_| {
                app.update_config(|c| {
                    c.language =
                        if c.language == Language::Chinese { Language::English } else { Language::Chinese };
                });
                app.rebuild();
            });
        }
        let lang_wrap = CutBin::new(&lang, CUT_SMALL);
        lang_wrap.set_valign(gtk::Align::Center);
        lang_wrap.set_margin_end(10);
        bar.append(&lang_wrap);

        // Own window buttons: square, text glyphs, independent of the icon theme.
        let controls = hbox(0);
        controls.add_css_class("winctl");
        for (glyph, tip, action) in [
            ("–", tr.t("最小化", "Minimize"), 0u8),
            ("□", tr.t("最大化", "Maximize"), 1),
            ("×", tr.t("关闭", "Close"), 2),
        ] {
            let b = gtk::Button::with_label(glyph);
            b.set_tooltip_text(Some(&tip));
            if action == 2 {
                b.add_css_class("close");
            }
            let window = self.window.clone();
            b.connect_clicked(move |_| match action {
                0 => window.minimize(),
                1 if window.is_maximized() => window.unmaximize(),
                1 => window.maximize(),
                _ => window.close(),
            });
            controls.append(&b);
        }
        bar.append(&controls);

        column.append(&bar);
        let handle = gtk::WindowHandle::builder().child(&column).build();
        (handle.upcast(), nav, layout_readout)
    }

    fn build_input_page(
        self: &Rc<Self>,
        stack: &gtk::Stack,
        nav: Vec<(Page, gtk::Button)>,
        layout_readout: gtk::Label,
    ) -> Ui {
        let tr = self.tr();
        let page = vbox(20);
        page.set_margin_top(28);
        page.set_margin_bottom(28);
        page.set_margin_start(40);
        page.set_margin_end(40);

        let columns = hbox(20);
        columns.set_vexpand(true);

        // Left: the text.
        let source = panel();
        source.set_hexpand(true);
        let paste = button(&tr.t("粘贴", "Paste"), &["small"]);
        let clear = button(&tr.t("清空", "Clear"), &["small"]);
        let head_end = hbox(8);
        head_end.append(&paste.1);
        head_end.append(&clear.1);
        source.append(&panel_head(&pair("Source", &tr.t("原文", "Source")), Some(head_end.upcast_ref())));
        let source_fields = label("", &["field"]);
        source_fields.set_margin_bottom(10);
        source.append(&source_fields);
        let text_view = gtk::TextView::with_buffer(&self.buffer);
        text_view.add_css_class("source");
        text_view.set_wrap_mode(gtk::WrapMode::WordChar);
        text_view.set_top_margin(12);
        text_view.set_bottom_margin(12);
        text_view.set_left_margin(14);
        text_view.set_right_margin(14);
        text_view.set_pixels_inside_wrap(4);
        let text_scroll = gtk::ScrolledWindow::builder()
            .child(&text_view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        text_scroll.add_css_class("source-frame");
        text_scroll.set_min_content_height(380);
        source.append(&text_scroll);
        let unsupported = notice(&tr.t("注意", "Note"));
        unsupported.0.set_margin_top(12);
        source.append(&unsupported.0);
        {
            let app = Rc::clone(self);
            paste.0.connect_clicked(move |_| {
                let app = Rc::clone(&app);
                glib::spawn_future_local(async move {
                    if let Ok(Some(text)) = app.window.clipboard().read_text_future().await {
                        app.buffer.set_text(&text);
                    }
                });
            });
        }
        {
            let app = Rc::clone(self);
            clear.0.connect_clicked(move |_| app.buffer.set_text(""));
        }
        columns.append(&source);

        // Right: session, profile, hotkeys.
        let right = vbox(20);
        right.set_size_request(460, -1);
        right.set_hexpand(false);

        let session = panel();
        let state_chip = label("", &["state"]);
        session
            .append(&panel_head(&pair("Session", &tr.t("会话", "Session")), Some(state_chip.upcast_ref())));
        let count_row = hbox(10);
        let typed = label("0000", &["figure", "huge"]);
        count_row.append(&typed);
        let total = label("", &["unit"]);
        total.set_valign(gtk::Align::End);
        total.set_margin_bottom(14);
        count_row.append(&total);
        session.append(&count_row);
        let (cell_row, cells) = cells();
        cell_row.set_margin_top(4);
        cell_row.set_margin_bottom(16);
        session.append(&cell_row);
        let figs = hbox(0);
        figs.set_homogeneous(true);
        let (mistakes, b1) = figure("0", &tr.t("错误", "Mistakes"), &["small"]);
        let (fixes, b2) = figure("0", &tr.t("修正", "Fixed"), &["small"]);
        let (elapsed, b3) = figure("00:00", &tr.t("用时", "Elapsed"), &["small"]);
        let (remaining, b4) = figure("--:--", &tr.t("剩余", "Remaining"), &["small"]);
        for b in [&b1, &b2, &b3, &b4] {
            figs.append(b);
        }
        session.append(&figs);
        let detail = label("", &["field"]);
        detail.set_wrap(true);
        detail.set_margin_top(12);
        session.append(&detail);
        let target = label("", &["field"]);
        target.set_wrap(true);
        target.set_margin_top(4);
        session.append(&target);
        let session_notice = notice(&tr.t("注意", "Note"));
        session_notice.0.set_margin_top(12);
        session.append(&session_notice.0);
        let actions = hbox(10);
        actions.set_margin_top(16);
        let start = go_button(&tr.t("开始输入", "Start typing"), "");
        let pause = button(&tr.t("暂停", "Pause"), &[]);
        let cancel = button(&tr.t("取消", "Cancel"), &[]);
        actions.append(&start.2);
        actions.append(&pause.1);
        actions.append(&cancel.1);
        session.append(&actions);
        {
            let app = Rc::clone(self);
            start.0.connect_clicked(move |_| app.start_typing());
            let app = Rc::clone(self);
            pause.0.connect_clicked(move |_| app.toggle_pause());
            let app = Rc::clone(self);
            cancel.0.connect_clicked(move |_| app.cancel_typing());
        }
        right.append(&session);

        let (profile, profile_fields) = self.build_profile_panel();
        right.append(&profile);

        let hotkeys = panel();
        let change = button(&tr.t("修改", "Change"), &["small"]);
        hotkeys.append(&panel_head(&pair("Hotkeys", &tr.t("热键", "Hotkeys")), Some(change.1.upcast_ref())));
        let mut hotkey_values = Vec::new();
        for (action, name) in [
            (HotkeyAction::Start, tr.t("开始输入", "Start typing")),
            (HotkeyAction::PauseResume, tr.t("暂停 / 继续", "Pause / resume")),
            (HotkeyAction::Cancel, tr.t("取消", "Cancel")),
        ] {
            let (row, value) = kv(&name, "");
            hotkeys.append(&row);
            hotkey_values.push((action, value));
        }
        let hotkey_notice = notice(&tr.t("注意", "Note"));
        hotkey_notice.0.set_margin_top(10);
        hotkeys.append(&hotkey_notice.0);
        {
            let app = Rc::clone(self);
            change.0.connect_clicked(move |_| {
                if let Some(d) = &app.desktop
                    && let Err(e) = d.configure_hotkeys()
                {
                    app.add_log(&e);
                }
            });
        }
        right.append(&hotkeys);
        columns.append(&right);
        page.append(&columns);

        // Bottom: log.
        let log = panel();
        let log_count = label("", &["field"]);
        log.append(&panel_head(&pair("Log", &tr.t("记录", "Log")), Some(log_count.upcast_ref())));
        let log_list = vbox(0);
        log.append(&log_list);
        page.append(&log);

        stack.add_named(&scrolled(&page), Some("input"));

        Ui {
            nav,
            stack: stack.clone(),
            layout_readout,
            source_fields,
            unsupported,
            text_view,
            state_chip,
            typed,
            total,
            cells,
            mistakes,
            fixes,
            elapsed,
            remaining,
            detail,
            target,
            start: start.0,
            start_sub: start.1,
            pause: pause.0,
            cancel: cancel.0,
            session_notice,
            profile_fields,
            hotkey_values,
            hotkey_notice,
            log_list,
            log_count,
        }
    }

    fn build_profile_panel(self: &Rc<Self>) -> (gtk::Box, gtk::Label) {
        let tr = self.tr();
        let c = self.config.borrow().clone();
        let profile = panel();
        profile.append(&panel_head(&pair("Profile", &tr.t("档位", "Profile")), None));

        let model_seg = {
            let app = Rc::clone(self);
            seg(
                &[tr.t("拟人", "Human"), tr.t("原版", "Original")],
                Some(usize::from(c.model == ModelKind::Classic)),
                move |i| {
                    app.update_config(|c| {
                        c.model = if i == 0 { ModelKind::Human } else { ModelKind::Classic }
                    });
                    app.rebuild();
                },
            )
        };
        profile.append(&setting_row(
            &tr.t("模型", "Model"),
            &tr.t(
                "拟人：按键级节奏和更多种错误。原版：与 Windows 版一致。",
                "Human: key-level rhythm and more kinds of mistakes. Original: same as the Windows build.",
            ),
            &model_seg,
        ));

        if c.model == ModelKind::Human {
            let presets = [
                (Preset::Novice, tr.t("生疏", "Novice")),
                (Preset::Average, tr.t("普通", "Average")),
                (Preset::Proficient, tr.t("熟练", "Skilled")),
                (Preset::Professional, tr.t("打字员", "Typist")),
            ];
            let active =
                if c.advanced.is_some() { None } else { presets.iter().position(|(p, _)| *p == c.preset) };
            let names: Vec<String> = presets.iter().map(|(_, n)| n.clone()).collect();
            let app = Rc::clone(self);
            let preset_seg = seg(&names, active, move |i| {
                app.update_config(|c| {
                    c.preset = presets[i].0;
                    c.advanced = None;
                });
            });
            let hint = if c.advanced.is_some() {
                tr.t(
                    "当前用的是“参数”页里的自定义值；选一个档位可以恢复。",
                    "Using custom values from the Tuning page; pick a profile to go back.",
                )
            } else {
                String::new()
            };
            profile.append(&setting_row(&tr.t("档位", "Profile"), &hint, &preset_seg));

            for (name, hint, get, set) in [
                (
                    tr.t("速度", "Speed"),
                    tr.t(
                        "50 为档位原值；0 慢一半，100 快一倍。",
                        "50 is the profile as is; 0 is half as fast, 100 twice as fast.",
                    ),
                    (|m: &Macros| m.speed) as fn(&Macros) -> u8,
                    (|m: &mut Macros, v| m.speed = v) as fn(&mut Macros, u8),
                ),
                (
                    tr.t("准确度", "Accuracy"),
                    tr.t(
                        "越高错误越少：0 是 4 倍，100 是四分之一。",
                        "Higher means fewer mistakes: 0 is 4×, 100 a quarter.",
                    ),
                    |m: &Macros| m.accuracy,
                    |m: &mut Macros, v| m.accuracy = v,
                ),
                (
                    tr.t("稳定度", "Steadiness"),
                    tr.t(
                        "越高节奏越稳，长停顿和疲劳越少。",
                        "Higher is a steadier rhythm with fewer long stops and less fatigue.",
                    ),
                    |m: &Macros| m.stability,
                    |m: &mut Macros, v| m.stability = v,
                ),
            ] {
                let row = hbox(10);
                let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
                scale.set_draw_value(false);
                scale.set_size_request(150, -1);
                scale.set_value(f64::from(get(&c.macros)));
                let readout = label(&format!("{:03}", get(&c.macros)), &["field", "ink"]);
                {
                    let app = Rc::clone(self);
                    let readout = readout.clone();
                    scale.connect_value_changed(move |s| {
                        let v = s.value().round() as u8;
                        readout.set_label(&format!("{v:03}"));
                        app.update_config(|c| set(&mut c.macros, v));
                    });
                }
                row.append(&scale);
                row.append(&readout);
                profile.append(&setting_row(&name, &hint, &row));
            }
        }
        let fields = label("", &["field"]);
        fields.set_margin_top(10);
        fields.set_wrap(true);
        profile.append(&fields);
        (profile, fields)
    }

    fn build_tuning_page(self: &Rc<Self>) -> gtk::Box {
        let tr = self.tr();
        let page = vbox(20);
        page.set_margin_top(28);
        page.set_margin_bottom(28);
        page.set_margin_start(40);
        page.set_margin_end(40);
        let c = self.config.borrow().clone();

        if c.model == ModelKind::Classic {
            page.append(&page_head(
                &tr.t("参数", "Tuning"),
                "Tuning",
                &tr.t(
                    "原版模型的八个数值，与 Windows 版含义相同。",
                    "The original model's eight values, as in the Windows build.",
                ),
            ));
            let panel = panel();
            panel.append(&panel_head(&pair("Original model", &tr.t("原版模型", "Original model")), None));
            for p in params::CLASSIC {
                let spin = gtk::SpinButton::with_range(0.0, f64::from(p.max), 1.0);
                spin.set_value(f64::from((p.get)(&c.classic)));
                let app = Rc::clone(self);
                let set = p.set;
                spin.connect_value_changed(move |s| {
                    app.update_config(|c| set(&mut c.classic, s.value() as u32))
                });
                let control = hbox(8);
                control.append(&spin);
                control.append(&label(p.unit, &["field"]));
                panel.append(&setting_row(if tr.zh() { p.zh } else { p.en }, "", &control));
            }
            page.append(&panel);
            return page;
        }

        page.append(&page_head(
            &tr.t("参数", "Tuning"),
            "Tuning",
            &tr.t(
                "拟人模型的全部数值。改动后，“输入”页的三个滑块在这些值的基础上再做调整。",
                "Every value of the human model. The three sliders on the Input page adjust on top of these.",
            ),
        ));
        let actions = hbox(10);
        let reset = button(&tr.t("恢复为档位值", "Reset to profile"), &[]);
        let export = button(&tr.t("导出", "Export"), &[]);
        let import = button(&tr.t("导入", "Import"), &[]);
        actions.append(&reset.1);
        actions.append(&export.1);
        actions.append(&import.1);
        page.append(&actions);
        {
            let app = Rc::clone(self);
            reset.0.connect_clicked(move |_| {
                app.update_config(|c| c.advanced = None);
                app.rebuild();
            });
            let app = Rc::clone(self);
            export.0.connect_clicked(move |_| app.export_model());
            let app = Rc::clone(self);
            import.0.connect_clicked(move |_| app.import_model());
        }

        let base = c.advanced.unwrap_or_else(|| c.preset.model());
        // Two fixed columns, balanced by row count.
        let grid = hbox(20);
        grid.set_homogeneous(true);
        let columns = [vbox(20), vbox(20)];
        for c in &columns {
            c.set_valign(gtk::Align::Start);
            grid.append(c);
        }
        let mut rows = [0usize; 2];
        for group in params::HUMAN {
            let panel = panel();
            panel.set_valign(gtk::Align::Start);
            panel.append(&panel_head(
                &pair(group.en, if tr.zh() { group.zh } else { group.en }),
                Some(
                    label(&format!("{} {}", group.params.len(), tr.t("项", "values")), &["field"])
                        .upcast_ref(),
                ),
            ));
            for p in group.params {
                let spin = gtk::SpinButton::with_range(p.min, p.max, p.step);
                spin.set_digits(p.digits);
                spin.set_value((p.get)(&base));
                spin.set_width_chars(7);
                let app = Rc::clone(self);
                let set = p.set;
                spin.connect_value_changed(move |s| {
                    app.update_config(|c| {
                        let mut m = c.advanced.unwrap_or_else(|| c.preset.model());
                        set(&mut m, s.value());
                        c.advanced = Some(m);
                    });
                });
                let control = hbox(8);
                control.append(&spin);
                let unit = label(p.unit, &["field"]);
                unit.set_width_chars(4);
                control.append(&unit);
                panel.append(&setting_row(
                    if tr.zh() { p.zh } else { p.en },
                    if tr.zh() { p.hint_zh } else { p.hint_en },
                    &control,
                ));
            }
            let side = usize::from(rows[1] < rows[0]);
            rows[side] += group.params.len() + 2;
            columns[side].append(&panel);
        }
        page.append(&grid);
        page
    }

    fn export_model(self: &Rc<Self>) {
        let model = {
            let c = self.config.borrow();
            c.advanced.unwrap_or_else(|| c.preset.model())
        };
        let dialog = gtk::FileDialog::builder()
            .title(self.tr().t("导出参数", "Export values"))
            .initial_name("humanize-model.toml")
            .build();
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.save_future(Some(&app.window)).await
                && let Some(path) = file.path()
            {
                let result = toml::to_string_pretty(&model)
                    .map_err(|e| e.to_string())
                    .and_then(|t| std::fs::write(&path, t).map_err(|e| e.to_string()));
                app.add_log(&match result {
                    Ok(()) => format!("{} {}", app.tr().t("已导出到", "Exported to"), path.display()),
                    Err(e) => format!("{}: {e}", app.tr().t("导出失败", "Export failed")),
                });
            }
        });
    }

    fn import_model(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title(self.tr().t("导入参数", "Import values")).build();
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.open_future(Some(&app.window)).await
                && let Some(path) = file.path()
            {
                let parsed = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| toml::from_str::<humanize_core::HumanModel>(&t).map_err(|e| e.to_string()));
                match parsed {
                    Ok(model) => {
                        app.update_config(|c| c.advanced = Some(model.sanitized()));
                        app.add_log(&format!("{} {}", app.tr().t("已导入", "Imported"), path.display()));
                        app.rebuild();
                    }
                    Err(e) => app.add_log(&format!("{}: {e}", app.tr().t("导入失败", "Import failed"))),
                }
            }
        });
    }

    fn build_settings_page(self: &Rc<Self>) -> gtk::Box {
        let tr = self.tr();
        let c = self.config.borrow().clone();
        let page = vbox(20);
        page.set_margin_top(28);
        page.set_margin_bottom(28);
        page.set_margin_start(40);
        page.set_margin_end(40);
        page.append(&page_head(
            &tr.t("设置", "Settings"),
            "Settings",
            &tr.t("修改后自动保存。", "Changes are saved automatically."),
        ));

        let columns = hbox(20);
        let left = vbox(20);
        left.set_hexpand(true);
        let right = vbox(20);
        right.set_hexpand(true);

        // General.
        let general = panel();
        general.append(&panel_head(&pair("General", &tr.t("通用", "General")), None));
        {
            let app = Rc::clone(self);
            let s = seg(
                &["中文".into(), "English".into()],
                Some(usize::from(c.language == Language::English)),
                move |i| {
                    app.update_config(|c| {
                        c.language = if i == 0 { Language::Chinese } else { Language::English }
                    });
                    app.rebuild();
                },
            );
            general.append(&setting_row(&tr.t("界面语言", "Language"), "", &s));
        }
        {
            let app = Rc::clone(self);
            let themes = [Theme::System, Theme::Light, Theme::Dark];
            let s = seg(
                &[tr.t("跟随系统", "System"), tr.t("浅色", "Light"), tr.t("深色", "Dark")],
                themes.iter().position(|t| *t == c.theme),
                move |i| {
                    app.update_config(|c| c.theme = themes[i]);
                    app.apply_theme();
                },
            );
            general.append(&setting_row(&tr.t("外观", "Appearance"), "", &s));
        }
        {
            let spin = gtk::SpinButton::with_range(0.0, 30000.0, 100.0);
            spin.set_value(f64::from(c.lead_in_ms));
            let app = Rc::clone(self);
            spin.connect_value_changed(move |s| app.update_config(|c| c.lead_in_ms = s.value() as u32));
            let control = hbox(8);
            control.append(&spin);
            control.append(&label("ms", &["field"]));
            general.append(&setting_row(
                &tr.t("开始前倒计时", "Countdown"),
                &tr.t(
                    "按下开始后等这么久，再把当时的前台窗口当作目标。",
                    "After you press start, wait this long, then type into whatever window is in front.",
                ),
                &control,
            ));
        }
        {
            let app = Rc::clone(self);
            let options = [WordDelete::Auto, WordDelete::CtrlBackspace, WordDelete::Backspaces];
            let s = seg(
                &[tr.t("自动", "Auto"), "Ctrl+Backspace".into(), tr.t("逐个退格", "Backspaces")],
                options.iter().position(|w| *w == c.word_delete),
                move |i| app.update_config(|c| c.word_delete = options[i]),
            );
            general.append(&setting_row(
                &tr.t("整词重打时怎么删", "Erasing a redone word"),
                &tr.t(
                    "自动：终端里逐个退格，其他程序用 Ctrl+Backspace。",
                    "Auto: Backspaces in terminals, Ctrl+Backspace elsewhere.",
                ),
                &s,
            ));
        }
        for (name, hint, get, set) in [
            (
                tr.t("启动后收到托盘", "Start in the tray"),
                tr.t("启动时不显示窗口，从托盘图标打开。", "Do not show the window at start; open it from the tray icon."),
                (|c: &Config| c.start_minimized) as fn(&Config) -> bool,
                (|c: &mut Config, v| c.start_minimized = v) as fn(&mut Config, bool),
            ),
            (
                tr.t("自动切换输入法", "Switch the input method"),
                tr.t("输入时把 fcitx5 切到英文，结束后切回，避免字母进了拼音框。", "Switch fcitx5 to direct input while typing so letters do not land in the pinyin box; restore afterwards."),
                |c: &Config| c.switch_input_method,
                |c: &mut Config, v| c.switch_input_method = v,
            ),
        ] {
            let switch = gtk::Switch::new();
            switch.set_active(get(&c));
            let app = Rc::clone(self);
            switch.connect_active_notify(move |s| {
                let v = s.is_active();
                app.update_config(|c| set(c, v));
                if let Some(d) = &app.desktop {
                    d.set_switch_input_method(app.config.borrow().switch_input_method);
                }
            });
            general.append(&setting_row(&name, &hint, &switch));
        }
        left.append(&general);

        // Device.
        let device = panel();
        device.append(&panel_head(&pair("Virtual keyboard", &tr.t("虚拟键盘", "Virtual keyboard")), None));
        {
            let entry = gtk::Entry::builder().text(&c.device.name).width_chars(18).build();
            let app = Rc::clone(self);
            entry.connect_changed(move |e| {
                let name = e.text().to_string();
                app.update_config(|c| c.device.name = name);
            });
            device.append(&setting_row(
                &tr.t("设备名", "Device name"),
                &tr.t("系统里显示的键盘名称。", "The keyboard name the system shows."),
                &entry,
            ));
        }
        for (name, get, set) in [
            (
                "VID",
                (|c: &Config| c.device.vendor) as fn(&Config) -> u16,
                (|c: &mut Config, v| c.device.vendor = v) as fn(&mut Config, u16),
            ),
            ("PID", |c: &Config| c.device.product, |c: &mut Config, v| c.device.product = v),
        ] {
            let entry =
                gtk::Entry::builder().text(format!("{:04x}", get(&c))).width_chars(6).max_length(4).build();
            let app = Rc::clone(self);
            entry.connect_changed(move |e| {
                if let Ok(v) = u16::from_str_radix(e.text().trim(), 16) {
                    app.update_config(|c| set(c, v));
                }
            });
            device.append(&setting_row(name, &tr.t("十六进制，4 位。", "Hexadecimal, 4 digits."), &entry));
        }
        {
            let rates = [125u32, 250, 500, 1000];
            let app = Rc::clone(self);
            let s = seg(
                &rates.iter().map(|r| format!("{r} Hz")).collect::<Vec<_>>(),
                rates.iter().position(|r| *r == c.device.polling_hz),
                move |i| app.update_config(|c| c.device.polling_hz = rates[i]),
            );
            device.append(&setting_row(
                &tr.t("轮询率", "Polling rate"),
                &tr.t(
                    "按键事件按这个频率成组发出，和真实 USB 键盘一样。",
                    "Key events go out in reports at this rate, like a real USB keyboard.",
                ),
                &s,
            ));
        }
        let restart = label(
            &tr.t("虚拟键盘的设置在下次启动时生效。", "Virtual keyboard settings apply at the next start."),
            &["hint"],
        );
        restart.set_margin_top(10);
        device.append(&restart);
        right.append(&device);

        // System status.
        let system = panel();
        system.append(&panel_head(&pair("System", &tr.t("系统", "System")), None));
        let yes = tr.t("可用", "available");
        let no = tr.t("不可用", "unavailable");
        for (key, value) in [
            (tr.t("键盘布局", "Keyboard layout"), self.layout_label.borrow().clone()),
            (
                tr.t("焦点检测", "Focus detection"),
                if self.desktop.as_ref().is_some_and(|d| d.has_focus_detection()) {
                    yes.clone()
                } else {
                    no.clone()
                },
            ),
            (
                tr.t("全局热键", "Global hotkeys"),
                if self.desktop.as_ref().is_some_and(|d| d.has_hotkeys()) { yes.clone() } else { no.clone() },
            ),
            (
                tr.t("配置文件", "Settings file"),
                self.config_path
                    .as_ref()
                    .map_or_else(|| tr.t("不保存", "not saved"), |p| p.display().to_string()),
            ),
        ] {
            system.append(&kv(&key, &value).0);
        }
        let (row, readout) = kv(&tr.t("虚拟键盘", "Virtual keyboard"), "");
        system.append(&row);
        *self.keyboard_readout.borrow_mut() = Some(readout);
        if let Some(d) = &self.desktop {
            for w in d.warnings() {
                let n = notice(&tr.t("注意", "Note"));
                set_notice(&n, Some(w));
                n.0.set_margin_top(10);
                system.append(&n.0);
            }
        }
        right.append(&system);

        columns.append(&left);
        columns.append(&right);
        page.append(&columns);
        page
    }

    // ── Refreshing ──────────────────────────────────────────────────────

    fn tray_labels(&self) -> TrayLabels {
        let tr = self.tr();
        let p = self.run.borrow().progress;
        TrayLabels {
            title: "humanize_input".into(),
            status: if p.total_chars > 0 {
                format!("{} / {}", p.typed_chars, p.total_chars)
            } else {
                String::new()
            },
            show: tr.t("打开窗口", "Show window"),
            start: tr.t("开始输入", "Start typing"),
            pause_resume: tr.t("暂停 / 继续", "Pause / resume"),
            cancel: tr.t("取消", "Cancel"),
            quit: tr.t("退出", "Quit"),
        }
    }

    fn keyboard_text(&self) -> String {
        let tr = self.tr();
        match &*self.keyboard.borrow() {
            Keyboard::Pending => tr.t("正在创建", "starting"),
            Keyboard::Ready(name) => name.clone(),
            Keyboard::Failed(_) => tr.t("不可用", "unavailable"),
        }
    }

    fn refresh_all(&self) {
        if let Some(l) = self.keyboard_readout.borrow().as_ref() {
            l.set_label(&self.keyboard_text());
        }
        if let Some(ui) = self.ui.borrow().as_ref() {
            ui.layout_readout.set_label(&format!(
                "{} {}",
                self.tr().t("布局", "LAYOUT"),
                self.layout_label.borrow()
            ));
        }
        self.refresh_source();
        self.refresh_session();
        self.refresh_profile();
        self.refresh_hotkeys();
        self.refresh_log();
        if let Some(d) = &self.desktop {
            let busy = self.run.borrow().state.is_some_and(SessionState::is_busy);
            d.update_tray(self.tray_labels(), busy);
        }
    }

    fn refresh_source(&self) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let tr = self.tr();
        let text = self.text();
        let chars = text.chars().count();
        let lines = if text.is_empty() { 0 } else { text.lines().count().max(1) };
        let unsupported = self.unsupported_count();
        let estimate = self.estimate.get().map_or_else(|| "--:--".into(), |d| clock(d.as_secs()));
        ui.source_fields.set_label(&if tr.zh() {
            format!(
                "{} 字符  │  {} 行  │  无法输入 {}  │  预计用时 {}",
                count(chars),
                count(lines),
                unsupported,
                estimate
            )
        } else {
            format!(
                "{} chars  │  {} lines  │  untypeable {}  │  estimated {}",
                count(chars),
                count(lines),
                unsupported,
                estimate
            )
        });
        set_notice(
            &ui.unsupported,
            (unsupported > 0)
                .then(|| {
                    if tr.zh() {
                        format!("有 {unsupported} 个字符无法输入（目前只支持英文、数字和键盘上的符号），已用黄色标出。删掉或替换后才能开始。")
                    } else {
                        format!("{unsupported} characters cannot be typed (only letters, digits and keyboard symbols for now). They are marked in yellow; remove or replace them to start.")
                    }
                })
                .as_deref(),
        );
        let busy = self.run.borrow().state.is_some_and(SessionState::is_busy);
        ui.text_view.set_editable(!busy);
    }

    fn refresh_session(&self) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let tr = self.tr();
        let run = self.run.borrow();
        let state = run.state.unwrap_or(SessionState::Idle);
        let busy = state.is_busy();
        let p = run.progress;
        let total = if p.total_chars > 0 { p.total_chars } else { self.text().chars().count() };

        let (state_text, class) = match state {
            SessionState::Idle => (tr.t("待命", "Ready"), "dashed"),
            SessionState::Arming => (tr.t("倒计时", "Countdown"), "alert"),
            SessionState::Running => (tr.t("输入中", "Typing"), ""),
            SessionState::Paused => (tr.t("已暂停", "Paused"), "dashed"),
            SessionState::WaitingFocus => (tr.t("等待焦点", "Waiting for focus"), "alert"),
            SessionState::Completed => (tr.t("已完成", "Done"), ""),
            SessionState::Faulted => (tr.t("出错", "Failed"), "alert"),
        };
        ui.state_chip.set_label(&state_text);
        for c in ["dashed", "alert"] {
            ui.state_chip.remove_css_class(c);
        }
        if !class.is_empty() {
            ui.state_chip.add_css_class(class);
        }

        ui.typed.set_label(&format!("{:04}", p.typed_chars));
        ui.total.set_label(&format!("/ {:04} {}", total, tr.t("字符", "chars")));
        set_cells(&ui.cells, if total > 0 { p.typed_chars as f64 / total as f64 } else { 0.0 });
        let mistakes = p.typo_count
            + p.omission_count
            + p.transpose_count
            + p.double_press_count
            + p.case_slip_count
            + p.word_redo_count;
        ui.mistakes.set_label(&mistakes.to_string());
        ui.fixes.set_label(&p.correction_count.to_string());
        let elapsed = run.started.map(|s| s.elapsed());
        ui.elapsed.set_label(&clock(elapsed.map_or(0, |e| e.as_secs())));
        let remaining = match (elapsed, self.estimate.get()) {
            (Some(e), _) if p.typed_chars >= 20 && total > p.typed_chars => {
                Some(e.mul_f64((total - p.typed_chars) as f64 / p.typed_chars as f64))
            }
            (Some(e), Some(est)) => Some(est.saturating_sub(e)),
            (None, Some(est)) => Some(est),
            _ => None,
        };
        ui.remaining.set_label(&remaining.map_or_else(|| "--:--".into(), |d| clock(d.as_secs())));
        ui.detail.set_label(&if tr.zh() {
            format!(
                "错字 {}  │  漏字 {}  │  颠倒 {}  │  重按 {}  │  大小写 {}  │  重打 {}",
                p.typo_count,
                p.omission_count,
                p.transpose_count,
                p.double_press_count,
                p.case_slip_count,
                p.word_redo_count
            )
        } else {
            format!(
                "wrong key {}  │  skipped {}  │  swapped {}  │  double {}  │  case {}  │  redone {}",
                p.typo_count,
                p.omission_count,
                p.transpose_count,
                p.double_press_count,
                p.case_slip_count,
                p.word_redo_count
            )
        });
        let lead = self.config.borrow().lead_in_ms as f64 / 1000.0;
        ui.target.set_label(&match (&run.target, busy) {
            (Some(t), _) if !t.is_empty() => format!("{}  {t}", tr.t("目标", "Target")),
            (_, true) if state != SessionState::Arming => format!(
                "{}  {}",
                tr.t("目标", "Target"),
                tr.t("开始时位于前台的窗口", "the window in front at the start")
            ),
            (_, true) => {
                if tr.zh() {
                    "目标  倒计时结束时位于前台的窗口".to_owned()
                } else {
                    "Target  the window in front when the countdown ends".into()
                }
            }
            _ => {
                if tr.zh() {
                    format!("目标  按下开始后 {lead:.1} 秒内切换到要输入的窗口")
                } else {
                    format!("Target  switch to the window to type into within {lead:.1} s of pressing start")
                }
            }
        });

        let keyboard_problem = match &*self.keyboard.borrow() {
            Keyboard::Failed(e) => {
                Some(format!("{}: {e}", tr.t("虚拟键盘不可用", "Virtual keyboard unavailable")))
            }
            _ => None,
        };
        set_notice(&ui.session_notice, run.problem.as_deref().or(keyboard_problem.as_deref()));

        let ready = matches!(&*self.keyboard.borrow(), Keyboard::Ready(_));
        ui.start.set_sensitive(ready && !busy && total > 0 && self.unsupported_count() == 0);
        ui.pause.set_sensitive(matches!(
            state,
            SessionState::Running | SessionState::Paused | SessionState::WaitingFocus
        ));
        ui.pause.set_label(&if state == SessionState::Paused {
            tr.t("继续", "Resume")
        } else {
            tr.t("暂停", "Pause")
        });
        ui.cancel.set_sensitive(busy);
        let start_hotkey =
            self.hotkeys.borrow().iter().find(|h| h.action == HotkeyAction::Start).map(|h| h.trigger.clone());
        ui.start_sub.set_label(&start_hotkey.unwrap_or_default());
    }

    fn refresh_profile(&self) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let tr = self.tr();
        let mode = self.config.borrow().typing_mode();
        ui.profile_fields.set_label(&match mode {
            TypingMode::Human(m) => {
                let mistakes =
                    m.typo_percent + m.omission_percent + m.transpose_percent + m.double_press_percent;
                if tr.zh() {
                    format!(
                        "约 {:.0} WPM  │  每字出错 约 {:.1}%  │  修正 {:.0}%",
                        m.wpm, mistakes, m.repair_percent
                    )
                } else {
                    format!(
                        "about {:.0} WPM  │  mistakes about {:.1}% per char  │  fixed {:.0}%",
                        m.wpm, mistakes, m.repair_percent
                    )
                }
            }
            TypingMode::Classic(s) => {
                if tr.zh() {
                    format!(
                        "基础延迟 {} ms  │  抖动 {}%  │  修复率 {}%",
                        s.base_delay_ms, s.jitter_percent, s.repair_rate_percent
                    )
                } else {
                    format!(
                        "base delay {} ms  │  jitter {}%  │  repair {}%",
                        s.base_delay_ms, s.jitter_percent, s.repair_rate_percent
                    )
                }
            }
        });
    }

    fn refresh_hotkeys(&self) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let tr = self.tr();
        let bound = self.hotkeys.borrow();
        for (action, value) in &ui.hotkey_values {
            let trigger = bound
                .iter()
                .find(|b| b.action == *action)
                .map(|b| b.trigger.clone())
                .filter(|t| !t.is_empty());
            value.set_label(&trigger.unwrap_or_else(|| tr.t("未设置", "not set")));
        }
        set_notice(&ui.hotkey_notice, self.hotkey_error.borrow().as_deref());
        drop(bound);
        self.refresh_session();
    }

    fn refresh_log(&self) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let tr = self.tr();
        while let Some(child) = ui.log_list.first_child() {
            ui.log_list.remove(&child);
        }
        let log = self.log.borrow();
        ui.log_count.set_label(&if tr.zh() {
            format!("{} 条", log.len())
        } else {
            format!("{} entries", log.len())
        });
        if log.is_empty() {
            ui.log_list.append(&label(&tr.t("开始输入后，这里按时间列出每一步：倒计时、开始、暂停、焦点变化和完成。", "Once you start typing, each step appears here in order: countdown, start, pauses, focus changes and completion."), &["empty"]));
            return;
        }
        for (time, message) in log.iter().take(12) {
            let row = hbox(12);
            row.add_css_class("log-row");
            row.append(&label(time, &["time"]));
            let m = label(message, &[]);
            m.set_wrap(true);
            m.set_hexpand(true);
            row.append(&m);
            ui.log_list.append(&row);
        }
    }
}

/// Passes events through and appends `<µs since start> <code> <0|1>` lines.
struct TracingDriver {
    inner: Box<dyn humanize_core::Driver>,
    out: std::io::BufWriter<std::fs::File>,
    start: Instant,
}

impl TracingDriver {
    fn new(
        inner: Box<dyn humanize_core::Driver>,
        path: &std::path::Path,
    ) -> Result<Self, (Box<dyn humanize_core::Driver>, std::io::Error)> {
        match std::fs::File::create(path) {
            Ok(f) => Ok(Self { inner, out: std::io::BufWriter::new(f), start: Instant::now() }),
            Err(e) => Err((inner, e)),
        }
    }
}

impl humanize_core::Driver for TracingDriver {
    fn key(&mut self, key: humanize_core::KeyCode, down: bool) -> Result<(), humanize_core::DriverError> {
        use std::io::Write;
        let _ = writeln!(self.out, "{} {} {}", self.start.elapsed().as_micros(), key.0, u8::from(down));
        let _ = self.out.flush();
        self.inner.key(key, down)
    }
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build()
}
