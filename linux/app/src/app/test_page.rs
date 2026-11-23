//! The typing test: copy a sample, then see (and optionally apply) the
//! model values measured from it.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use humanize_core::fit::SAMPLES;
use humanize_core::{FitReport, KeyCode, Macros, SampleEvent, fit};

use super::{App, Page};
use crate::i18n::{clock, count};
use crate::params;
use crate::widgets::*;

#[derive(Default)]
pub(super) struct TestState {
    /// 0–2: built-in samples; 3: the text in the Source box.
    sample: usize,
    /// The text being copied, fixed when the test (re)starts, so later
    /// edits to the Source box cannot change what the sample is scored on.
    prompt: String,
    /// Monotonic time (µs) of the first key press.
    start_us: Option<i64>,
    events: Vec<SampleEvent>,
    report: Option<FitReport>,
    /// The analysis is running on a worker thread.
    analysing: bool,
    /// We switched the input method off and should switch it back.
    ime_switched: bool,
    /// Suppresses recording while the box is cleared programmatically.
    resetting: bool,
    ui: Option<TestUi>,
}

#[derive(Clone)]
struct TestUi {
    prompt: gtk::TextBuffer,
    input: gtk::TextView,
    fields: gtk::Label,
    finish: gtk::Button,
    results: gtk::Box,
    notice: (gtk::Box, gtk::Label),
}

const MIN_CHARS: usize = 30;
/// Longest Source text used as a sample; analysis time and memory grow
/// with the square of the length.
const MAX_PROMPT: usize = 3000;

/// `text` cut to at most `max` characters, at a word boundary when possible.
fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => cut[..i].to_owned(),
        _ => cut,
    }
}

impl App {
    /// Widgets of the test page, cloned out so no borrow is held while GTK
    /// emits signals that come back into the test state.
    fn test_ui(&self) -> Option<TestUi> {
        self.test.borrow().ui.clone()
    }

    fn test_elapsed(&self) -> Duration {
        let start = self.test.borrow().start_us;
        start.map_or(Duration::ZERO, |s| Duration::from_micros((glib::monotonic_time() - s).max(0) as u64))
    }

    fn test_record(&self, event: impl FnOnce(Duration) -> SampleEvent, starts: bool) {
        // Busy only while we edit the box ourselves; nothing to record then.
        let Ok(mut st) = self.test.try_borrow_mut() else { return };
        if st.resetting || st.report.is_some() || st.analysing {
            return;
        }
        let now = glib::monotonic_time();
        if st.start_us.is_none() {
            if !starts {
                return;
            }
            st.start_us = Some(now);
        }
        let t = Duration::from_micros((now - st.start_us.unwrap_or(now)).max(0) as u64);
        st.events.push(event(t));
    }

    pub(super) fn build_test_page(self: &Rc<Self>) -> gtk::Box {
        let tr = self.tr();
        let page = vbox(20);
        page.add_css_class("page");
        page.append(&page_head(
            &tr.t("测试", "Typing test"),
            "Test",
            &tr.t(
                "照着下面的文字打一遍，打错了就像平时一样改。结束后这里会列出从你的打字里测出的参数，确认后再应用到“参数”页。",
                "Copy the text below, fixing mistakes as you normally would. Afterwards this page lists the values measured from your typing; apply them to the Tuning page if they look right.",
            ),
        ));

        // Sample.
        let sample = panel();
        let fields = label("", &["field"]);
        sample.append(&panel_head(&pair("Sample", &tr.t("样本", "Sample")), Some(fields.upcast_ref())));
        let current = self.test.borrow().sample;
        let names = vec![
            tr.t("样本 1", "Sample 1"),
            tr.t("样本 2", "Sample 2"),
            tr.t("样本 3", "Sample 3"),
            tr.t("用原文", "Use Source text"),
        ];
        let chooser = {
            let app = Rc::clone(self);
            seg(&names, Some(current), move |i| {
                app.test.borrow_mut().sample = i;
                app.test_reset();
            })
        };
        sample.append(&setting_row(
            &tr.t("选择文字", "Text"),
            &if tr.zh() {
                format!("“用原文”会用“输入”页里的文字（最多前 {MAX_PROMPT} 个字符），它只能包含英文字符。")
            } else {
                format!("Use Source text takes the text from the Input page (up to {MAX_PROMPT} characters); it must be plain ASCII.")
            },
            &chooser,
        ));
        let prompt = gtk::TextBuffer::new(Some(&self.buffer.tag_table()));
        let prompt_view = gtk::TextView::with_buffer(&prompt);
        prompt_view.add_css_class("source");
        prompt_view.set_editable(false);
        prompt_view.set_cursor_visible(false);
        prompt_view.set_can_focus(false);
        style_text(&prompt_view);
        let frame = gtk::ScrolledWindow::builder()
            .child(&prompt_view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        frame.add_css_class("source-frame");
        frame.set_min_content_height(150);
        frame.set_margin_top(12);
        sample.append(&frame);
        page.append(&sample);

        // Typing box.
        let typing = panel();
        typing.append(&panel_head(&pair("Type here", &tr.t("在这里打字", "Type here")), None));
        let input = gtk::TextView::new();
        input.add_css_class("source");
        style_text(&input);
        let input_frame =
            gtk::ScrolledWindow::builder().child(&input).hscrollbar_policy(gtk::PolicyType::Never).build();
        input_frame.add_css_class("source-frame");
        input_frame.set_min_content_height(150);
        typing.append(&input_frame);
        let notice = notice(&tr.t("注意", "Note"));
        notice.0.set_margin_top(12);
        typing.append(&notice.0);
        let actions = hbox(10);
        actions.set_margin_top(16);
        let finish = go_button(&tr.t("结束并分析", "Finish and analyse"), "");
        let restart = button(&tr.t("重新开始", "Start over"), &[]);
        actions.append(&finish.2);
        actions.append(&restart.1);
        typing.append(&actions);
        page.append(&typing);

        // Results.
        let results = vbox(0);
        page.append(&results);

        self.wire_test_input(&input);
        {
            let app = Rc::clone(self);
            finish.0.connect_clicked(move |_| app.test_finish());
            let app = Rc::clone(self);
            restart.0.connect_clicked(move |_| app.test_reset());
        }

        // Rebuilt UI (e.g. language switch): an unfinished test restarts.
        let fresh = self.test.borrow().report.is_none();
        self.test.borrow_mut().ui = Some(TestUi { prompt, input, fields, finish: finish.0, results, notice });
        if fresh {
            self.test_reset();
        } else {
            self.test_show_prompt();
            self.test_render();
        }
        page
    }

    fn wire_test_input(self: &Rc<Self>, input: &gtk::TextView) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let app = Rc::clone(self);
            keys.connect_key_pressed(move |_, _, keycode, _| {
                // Hardware keycodes are XKB codes: evdev + 8.
                let key = KeyCode(keycode.saturating_sub(8) as u16);
                app.test_record(|t| SampleEvent::KeyDown { t, key }, true);
                glib::Propagation::Proceed
            });
        }
        {
            let app = Rc::clone(self);
            keys.connect_key_released(move |_, _, keycode, _| {
                let key = KeyCode(keycode.saturating_sub(8) as u16);
                app.test_record(|t| SampleEvent::KeyUp { t, key }, false);
            });
        }
        input.add_controller(keys);

        let buffer = input.buffer();
        {
            let app = Rc::clone(self);
            buffer.connect_insert_text(move |_, at, text| {
                let (at, text) = (at.offset().max(0) as usize, text.to_owned());
                app.test_record(|t| SampleEvent::Insert { t, at, text }, false);
            });
        }
        {
            let app = Rc::clone(self);
            buffer.connect_delete_range(move |_, start, end| {
                let (start, end) = (start.offset().max(0) as usize, end.offset().max(0) as usize);
                app.test_record(|t| SampleEvent::Delete { t, start, end }, false);
            });
        }
        {
            let app = Rc::clone(self);
            buffer.connect_changed(move |_| app.test_changed());
        }

        // An input method in Chinese mode would swallow the keys: switch it
        // off while the box has focus. fcitx acts on whichever window has
        // focus, so only touch it while that window is ours.
        let focus = gtk::EventControllerFocus::new();
        {
            let app = Rc::clone(self);
            focus.connect_enter(move |_| {
                if !app.window.is_active() || !app.config.borrow().switch_input_method {
                    return;
                }
                if let Some(d) = &app.desktop {
                    let switched = d.set_focused_direct_input(true);
                    app.test.borrow_mut().ime_switched |= switched;
                }
            });
        }
        {
            let app = Rc::clone(self);
            focus.connect_leave(move |_| {
                // Focus left the window (Alt+Tab): the focused input context
                // is someone else's now; restore on a later in-window leave.
                if !app.window.is_active() {
                    return;
                }
                let switched = std::mem::take(&mut app.test.borrow_mut().ime_switched);
                if switched && let Some(d) = &app.desktop {
                    d.set_focused_direct_input(false);
                }
            });
        }
        input.add_controller(focus);
    }

    /// Puts the cursor in the typing box, but only while the Test page is
    /// shown: keys typed into a hidden page would be lost.
    pub(super) fn test_focus_input(&self) {
        if self.page.get() != Page::Test {
            return;
        }
        if let Some(ui) = self.test_ui() {
            // After the page is mapped, or the grab does not stick.
            glib::idle_add_local_once(move || {
                ui.input.grab_focus();
            });
        }
    }

    fn test_reset(self: &Rc<Self>) {
        let prompt = {
            let sample = self.test.borrow().sample;
            SAMPLES.get(sample).map_or_else(|| cap(&self.text(), MAX_PROMPT), |s| (*s).to_owned())
        };
        {
            let mut st = self.test.borrow_mut();
            st.prompt = prompt;
            st.start_us = None;
            st.events.clear();
            st.report = None;
            st.analysing = false;
            st.resetting = true;
        }
        if let Some(ui) = self.test_ui() {
            ui.input.buffer().set_text("");
            ui.input.set_editable(true);
        }
        self.test.borrow_mut().resetting = false;
        self.test_show_prompt();
        self.test_render();
        self.test_focus_input();
    }

    fn test_show_prompt(&self) {
        let Some(ui) = self.test_ui() else { return };
        let prompt = self.test.borrow().prompt.clone();
        let tr = self.tr();
        ui.prompt.set_text(&prompt);
        let unsupported = self.layout.borrow().find_unsupported(&prompt).len();
        let problem = if prompt.trim().chars().count() < MIN_CHARS {
            Some(if tr.zh() {
                format!("这段文字太短，至少需要 {MIN_CHARS} 个字符。")
            } else {
                format!("This text is too short; it needs at least {MIN_CHARS} characters.")
            })
        } else if unsupported > 0 {
            Some(if tr.zh() {
                format!("这段文字里有 {unsupported} 个无法输入的字符，请换一段。")
            } else {
                format!("This text contains {unsupported} characters that cannot be typed; pick another.")
            })
        } else {
            None
        };
        set_notice(&ui.notice, problem.as_deref());
        ui.input.set_sensitive(problem.is_none());
        self.test_mark();
    }

    /// Shades the sample by how far the typed text matches it.
    fn test_mark(&self) {
        let Some(ui) = self.test_ui() else { return };
        let (prompt, finished, analysing) = {
            let st = self.test.borrow();
            (st.prompt.clone(), st.report.is_some(), st.analysing)
        };
        let typed = text_of(&ui.input.buffer());
        let matched = typed.chars().zip(prompt.chars()).take_while(|(a, b)| a == b).count();
        let (start, end) = ui.prompt.bounds();
        ui.prompt.remove_tag_by_name("cursor", &start, &end);
        ui.prompt.apply_tag_by_name("pending", &start, &end);
        let typed_n = typed.chars().count();
        ui.finish.set_sensitive(!finished && !analysing && typed_n >= MIN_CHARS);
        if finished {
            ui.prompt.remove_tag_by_name("pending", &start, &end);
            return;
        }
        let cut = ui.prompt.iter_at_offset(matched as i32);
        ui.prompt.remove_tag_by_name("pending", &start, &cut);
        let mut next = cut;
        if next.forward_char() {
            ui.prompt.apply_tag_by_name("cursor", &cut, &next);
        }
        let tr = self.tr();
        let secs = self.test_elapsed().as_secs_f64();
        let wpm =
            if secs > 2.0 { format!("{:.0}", typed_n as f64 / 5.0 / (secs / 60.0)) } else { "--".into() };
        let total = count(prompt.chars().count());
        ui.fields.set_label(&if tr.zh() {
            format!("{total} 字符  │  已打 {}  │  用时 {}  │  {wpm} WPM", count(typed_n), clock(secs as u64))
        } else {
            format!(
                "{total} chars  │  typed {}  │  time {}  │  {wpm} WPM",
                count(typed_n),
                clock(secs as u64)
            )
        });
    }

    fn test_changed(self: &Rc<Self>) {
        if self.test.borrow().resetting {
            return;
        }
        self.test_mark();
        let done = {
            let st = self.test.borrow();
            !st.analysing
                && st.report.is_none()
                && st.ui.as_ref().is_some_and(|ui| {
                    let typed = text_of(&ui.input.buffer());
                    !typed.is_empty() && typed.trim_end() == st.prompt.trim_end()
                })
        };
        if done {
            // Let the last key-up arrive before analysing.
            let app = Rc::clone(self);
            glib::timeout_add_local_once(Duration::from_millis(250), move || app.test_finish());
        }
    }

    fn test_finish(self: &Rc<Self>) {
        let tr = self.tr();
        let (prompt, events) = {
            let st = self.test.borrow();
            if st.report.is_some() || st.analysing {
                return;
            }
            (st.prompt.clone(), st.events.clone())
        };
        // Same rule as the button: enough characters, typed key by key.
        let typed_by_keys = events
            .iter()
            .filter_map(|e| {
                if let SampleEvent::Insert { text, .. } = e { Some(text.chars().count()) } else { None }
            })
            .filter(|n| *n == 1)
            .count();
        if typed_by_keys < MIN_CHARS {
            if let Some(ui) = self.test_ui() {
                set_notice(
                    &ui.notice,
                    Some(&if tr.zh() {
                        format!("至少要逐个按键打 {MIN_CHARS} 个字符才能分析；粘贴进来的文字不算。")
                    } else {
                        format!(
                            "Type at least {MIN_CHARS} characters key by key to analyse; pasted text does not count."
                        )
                    }),
                );
            }
            return;
        }
        let base = {
            let c = self.config.borrow();
            c.advanced.unwrap_or_else(|| c.preset.model())
        };
        let layout = Arc::clone(&self.layout.borrow());
        self.test.borrow_mut().analysing = true;
        if let Some(ui) = self.test_ui() {
            ui.input.set_editable(false);
            ui.finish.add_css_class("busy");
        }
        self.test_mark();

        // The fit simulates the model on the prompt; keep it off the UI thread.
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(fit(&prompt, &events, &layout, base));
        });
        let app = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Ok(report) = rx.recv().await else { return };
            app.test_finished(report);
        });
    }

    fn test_finished(self: &Rc<Self>, report: FitReport) {
        if !self.test.borrow().analysing {
            return; // reset while analysing
        }
        let reliable = report.items.iter().filter(|i| i.reliable).count();
        let tr = self.tr();
        self.add_log(&if tr.zh() {
            format!(
                "测试完成：{:.0} WPM，准确率 {:.1}%，测得 {} 项可信参数。",
                report.gross_wpm, report.accuracy, reliable
            )
        } else {
            format!(
                "Test done: {:.0} WPM, {:.1}% accurate, {} reliable values measured.",
                report.gross_wpm, report.accuracy, reliable
            )
        });
        // Debugging aid: HUMANIZE_INPUT_TEST_REPORT=FILE saves the analysis.
        if let Some(path) = std::env::var_os("HUMANIZE_INPUT_TEST_REPORT") {
            let _ = std::fs::write(path, format!("{report:#?}"));
        }
        {
            let mut st = self.test.borrow_mut();
            st.report = Some(report);
            st.analysing = false;
        }
        if let Some(ui) = self.test_ui() {
            ui.finish.remove_css_class("busy");
        }
        self.test_mark();
        self.test_render();
    }

    fn test_apply(self: &Rc<Self>) {
        let Some(report) = self.test.borrow().report.clone() else { return };
        let applied = report.items.iter().filter(|i| i.reliable).count();
        self.update_config(|c| {
            c.advanced = Some(report.model);
            c.macros = Macros::default();
        });
        let tr = self.tr();
        self.add_log(&if tr.zh() {
            format!("已把测出的 {applied} 项参数写入“参数”页，速度、准确度、稳定度滑块归位到 50。")
        } else {
            format!("Applied {applied} measured values to the Tuning page; the three sliders are back at 50.")
        });
        self.rebuild();
        self.show_page(Page::Tuning);
    }

    fn test_render(self: &Rc<Self>) {
        let Some(ui) = self.test_ui() else { return };
        while let Some(child) = ui.results.first_child() {
            ui.results.remove(&child);
        }
        let Some(report) = self.test.borrow().report.clone() else { return };
        let tr = self.tr();
        let base = {
            let c = self.config.borrow();
            c.advanced.unwrap_or_else(|| c.preset.model())
        };

        let panel = panel();
        let reliable = report.items.iter().filter(|i| i.reliable).count();
        let summary = label(
            &if tr.zh() {
                format!("{} 项可信  │  {} 项样本不足", reliable, report.items.len() - reliable)
            } else {
                format!("{} reliable  │  {} with too few samples", reliable, report.items.len() - reliable)
            },
            &["field"],
        );
        panel.append(&panel_head(&pair("Result", &tr.t("结果", "Result")), Some(summary.upcast_ref())));
        let figs = hbox(0);
        figs.set_homogeneous(true);
        for (value, caption) in [
            (format!("{:.0}", report.gross_wpm), tr.t("速度 WPM", "Speed WPM")),
            (format!("{:.1}", report.accuracy), tr.t("准确率 %", "Accuracy %")),
            (clock(report.duration.as_secs()), tr.t("用时", "Time")),
            (report.corrections.to_string(), tr.t("改正次数", "Corrections")),
            (format!("{:.0}", report.rollover_percent), tr.t("按键重叠 %", "Key overlap %")),
        ] {
            figs.append(&figure(&value, &caption, &["small"]).1);
        }
        figs.set_margin_bottom(16);
        panel.append(&figs);

        let head = hbox(12);
        head.add_css_class("row");
        for (text, width) in [
            (tr.t("参数", "Value"), 0),
            (tr.t("当前", "Now"), 9),
            (tr.t("测得", "Measured"), 9),
            (tr.t("样本", "Samples"), 12),
        ] {
            let l = label(&text, &["eyebrow"]);
            if width == 0 {
                l.set_hexpand(true);
            } else {
                l.set_width_chars(width);
                l.set_xalign(1.0);
            }
            head.append(&l);
        }
        panel.append(&head);
        let order: Vec<&str> = params::HUMAN.iter().flat_map(|g| g.params.iter().map(|p| p.id)).collect();
        let mut items: Vec<_> = report.items.iter().collect();
        items.sort_by_key(|i| order.iter().position(|id| *id == i.field).unwrap_or(usize::MAX));
        for item in items {
            let Some(p) = params::human(item.field) else { continue };
            let row = hbox(12);
            row.add_css_class("row");
            let unit = if p.unit.is_empty() { String::new() } else { format!("  {}", p.unit) };
            let name = label(&format!("{}{unit}", if tr.zh() { p.zh } else { p.en }), &["name"]);
            name.set_hexpand(true);
            row.append(&name);
            let digits = p.digits as usize;
            let now = label(&format!("{:.*}", digits, (p.get)(&base)), &["field"]);
            now.set_width_chars(9);
            now.set_xalign(1.0);
            row.append(&now);
            let measured =
                label(&format!("{:.*}", digits, item.value.clamp(p.min, p.max)), &["field", "ink"]);
            measured.set_width_chars(9);
            measured.set_xalign(1.0);
            row.append(&measured);
            let chip = label(
                &if item.reliable {
                    format!("{} {}", tr.t("可信", "ok"), item.samples)
                } else {
                    format!("{} {}", tr.t("样本少", "few"), item.samples)
                },
                &["state"],
            );
            if !item.reliable {
                chip.add_css_class("dashed");
            }
            let chip_box = hbox(0);
            chip_box.set_width_request(110);
            chip_box.set_halign(gtk::Align::End);
            chip_box.append(&spacer());
            chip_box.append(&chip);
            row.append(&chip_box);
            panel.append(&row);
        }
        let note = label(
            &tr.t(
                "只有标为“可信”的参数会被应用；其余保留当前值。疲劳相关的参数需要更长的样本，这里不测。",
                "Only values marked ok are applied; the rest keep their current setting. Fatigue needs a much longer sample and is not measured here.",
            ),
            &["hint"],
        );
        note.set_wrap(true);
        note.set_margin_top(12);
        panel.append(&note);
        let actions = hbox(10);
        actions.set_margin_top(16);
        let apply = go_button(
            &if tr.zh() { format!("应用 {reliable} 项") } else { format!("Apply {reliable} values") },
            "",
        );
        apply.0.set_sensitive(reliable > 0);
        let again = button(&tr.t("重新测试", "Test again"), &[]);
        actions.append(&apply.2);
        actions.append(&again.1);
        panel.append(&actions);
        {
            let app = Rc::clone(self);
            apply.0.connect_clicked(move |_| app.test_apply());
            let app = Rc::clone(self);
            again.0.connect_clicked(move |_| app.test_reset());
        }
        ui.results.append(&panel);
    }
}

fn style_text(view: &gtk::TextView) {
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_pixels_inside_wrap(4);
    view.set_top_margin(12);
    view.set_bottom_margin(12);
    view.set_left_margin(14);
    view.set_right_margin(14);
}

fn text_of(buffer: &gtk::TextBuffer) -> String {
    let (s, e) = buffer.bounds();
    buffer.text(&s, &e, false).to_string()
}

#[cfg(test)]
mod tests {
    use super::cap;

    #[test]
    fn long_source_text_is_capped_at_a_word_boundary() {
        assert_eq!(cap("short", 10), "short");
        assert_eq!(cap("alpha beta gamma", 12), "alpha beta");
        assert_eq!(cap("abcdefghij", 4), "abcd");
    }
}
