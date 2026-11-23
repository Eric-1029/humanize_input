//! humanize_input for Linux: types text like a person, through a virtual USB
//! keyboard, on KDE Plasma (Wayland or X11).

mod app;
mod assets;
mod cut;
mod i18n;
mod params;
mod size_watch;
mod widgets;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use humanize_desktop::config::{Language, Theme};
use humanize_desktop::shortcuts::APP_ID;

use crate::app::{App, Options, Page};

const USAGE: &str = "usage: humanize_input [--text FILE] [--theme light|dark] [--lang zh|en]
                      [--page input|tuning|test|settings] [--offline] [--demo] [--screenshot FILE.png [--size WxH]]";

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--theme" => {
                options.theme = Some(match value()?.as_str() {
                    "light" => Theme::Light,
                    "dark" => Theme::Dark,
                    _ => Theme::System,
                })
            }
            "--lang" => {
                options.language = Some(if value()? == "en" { Language::English } else { Language::Chinese })
            }
            "--page" => {
                options.page = Some(match value()?.as_str() {
                    "tuning" => Page::Tuning,
                    "settings" => Page::Settings,
                    "test" => Page::Test,
                    _ => Page::Input,
                })
            }
            "--screenshot" => options.screenshot = Some(PathBuf::from(value()?)),
            "--offline" => options.offline = true,
            "--text" => options.text_file = Some(PathBuf::from(value()?)),
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--size needs WIDTHxHEIGHT")?;
                options.size =
                    Some((w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?));
            }
            "--demo" => options.demo = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    Ok(options)
}

/// The desktop portal names apps by their systemd scope. Started from a
/// terminal we would inherit the terminal's scope (and its hotkeys); move
/// into our own scope first. Skipped when already there or not possible.
fn ensure_own_scope() {
    const GUARD: &str = "HUMANIZE_INPUT_SCOPED";
    if std::env::var_os(GUARD).is_some() {
        return;
    }
    let ours = format!("app-{APP_ID}");
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if cgroup.lines().any(|l| l.rsplit('/').next().is_some_and(|unit| unit.starts_with(&ours))) {
        return;
    }
    // systemd-run needs a reachable user manager; exec would otherwise
    // replace us with a failing systemd-run (SSH, containers, no session).
    let manager = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| std::path::Path::new(&d).join("systemd/private"))
        .is_some_and(|socket| socket.exists());
    if !manager {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let unit = format!("{ours}-{}.scope", std::process::id());
    // exec only returns on failure; then carry on in the current scope.
    let _ = std::process::Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect", "--unit", &unit, "--"])
        .arg(exe)
        .args(std::env::args_os().skip(1))
        .env(GUARD, "1")
        .exec();
}

fn main() -> glib::ExitCode {
    let options = match parse_args() {
        Ok(o) => o,
        Err(message) => {
            eprintln!("{message}");
            return glib::ExitCode::FAILURE;
        }
    };
    if options.screenshot.is_none() && !options.demo && !options.offline {
        if let Ok(Some(path)) = humanize_desktop::shortcuts::ensure_desktop_entry() {
            eprintln!("registered {} for desktop portals", path.display());
        }
        ensure_own_scope();
    }
    let css = match assets::prepare() {
        Ok(css) => css,
        Err(e) => {
            eprintln!("cannot unpack assets: {e}");
            return glib::ExitCode::FAILURE;
        }
    };

    let mut builder = gtk::Application::builder().application_id(APP_ID);
    if options.screenshot.is_some() || options.demo {
        builder = builder.flags(gtk::gio::ApplicationFlags::NON_UNIQUE);
    }
    let gtk_app = builder.build();
    let state: Rc<std::cell::RefCell<Option<Rc<App>>>> = Rc::default();
    {
        let state = Rc::clone(&state);
        gtk_app.connect_startup(move |_| {
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(|_, section, error| {
                eprintln!("theme: {} at {}", error, section.to_str());
            });
            provider.load_from_string(&css);
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            let _ = &state;
        });
    }
    {
        let state = Rc::clone(&state);
        gtk_app.connect_activate(move |gtk_app| {
            let mut slot = state.borrow_mut();
            match slot.as_ref() {
                // A second launch just brings the window back.
                Some(app) => app.present(),
                None => {
                    // Keep running while the window is hidden in the tray.
                    std::mem::forget(gtk_app.hold());
                    let app = App::new(gtk_app, options.clone());
                    // Quit cleanly on logout / kill so no key stays down and
                    // the KWin script is unloaded.
                    for signal in [libc_sigterm(), libc_sigint()] {
                        let app = Rc::clone(&app);
                        glib_unix::unix_signal_add_local(signal, move || {
                            app.quit();
                            glib::ControlFlow::Break
                        });
                    }
                    *slot = Some(app);
                }
            }
        });
    }
    {
        let state = Rc::clone(&state);
        gtk_app.connect_shutdown(move |_| {
            if let Some(app) = state.borrow().as_ref() {
                app.shutdown();
            }
        });
    }
    gtk_app.run_with_args::<&str>(&[])
}

const fn libc_sigterm() -> i32 {
    15
}

const fn libc_sigint() -> i32 {
    2
}
