//! The typing test: copy a sample, then see (and optionally apply) the
//! model values measured from it.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use humanize_core::fit::SAMPLES;
use humanize_core::{FitReport, KeyCode, Macros, SampleEvent, fit};
use humanize_desktop::config::ModelKind;

use super::{App, Page};
use crate::i18n::{clock, count};
use crate::params;
use crate::widgets::*;

#[derive(Default)]
pub(super) struct TestState {
    /// 0–2: built-in samples; 3: the text in the Source box.
    sample: usize,
    /// Monotonic time (µs) of the first key press.
    start_us: Option<i64>,
    events: Vec<SampleEvent>,
    report: Option<FitReport>,
    /// We switched the input method off and should switch it back.
    ime_switched: bool,
    /// Suppresses recording while the box is cleared programmatically.
    resetting: bool,
    ui: Option<TestUi>,
}

struct TestUi {
    prompt: gtk::TextBuffer,
    input: gtk::TextView,
    fields: gtk::Label,
    finish: gtk::Button,
    results: gtk::Box,
    notice: (gtk::Box, gtk::Label),
}

const MIN_CHARS: usize = 30;

impl App {
    fn test_prompt(&self) -> String {
        let i = self.test.borrow().sample;
        SAMPLES.get(i).map_or_else(|| self.text(), |s| (*s).to_owned())
    }

    fn test_elapsed(&self) -> Duration {
        let start = self.test.borrow().start_us;
        start.map_or(Duration::ZERO, |s| Duration::from_micros((glib::monotonic_time() - s).max(0) as u64))
    }

    fn test_record(&self, event: impl FnOnce(Duration) -> SampleEvent, starts: bool) {
        let mut st = self.test.borrow_mut();
        if st.resetting || st.report.is_some() {
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
            &tr.t(
                "“用原文”会用“输入”页里的文字；它只能包含英文字符。",
                "Use Source text takes the text from the Input page; it must be plain ASCII.",
            ),
            &chooser,
        ));
        let prompt = gtk::TextBuffer::new(Some(&self.buffer.tag_table()));
        let prompt_view = gtk::TextView::with_buffer(&prompt);
        prompt_view.add_css_class("source");
        prompt_view.set_editable(false);
        prompt_view.set_cursor_visible(false);
        prompt_view.set_wrap_mode(gtk::WrapMode::WordChar);
        prompt_view.set_pixels_inside_wrap(4);
        prompt_view.set_top_margin(12);
        prompt_view.set_bottom_margin(12);
        prompt_view.set_left_margin(14);
        prompt_view.set_right_margin(14);
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
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        input.set_pixels_inside_wrap(4);
        input.set_top_margin(12);
        input.set_bottom_margin(12);
        input.set_left_margin(14);
        input.set_right_margin(14);
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
        prompt_view.set_vexpand(false);
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
        // off while the box has focus, and back on afterwards.
        let focus = gtk::EventControllerFocus::new();
        {
            let app = Rc::clone(self);
            focus.connect_enter(move |_| {
                if let Some(d) = &app.desktop {
                    let switched = d.set_focused_direct_input(true);
                    app.test.borrow_mut().ime_switched |= switched;
                }
            });
        }
        {
            let app = Rc::clone(self);
            focus.connect_leave(move |_| {
                let switched = std::mem::take(&mut app.test.borrow_mut().ime_switched);
                if switched && let Some(d) = &app.desktop {
                    d.set_focused_direct_input(false);
                }
            });
        }
        input.add_controller(focus);
    }

    /// Puts the cursor in the typing box (keys typed elsewhere would be lost,
    /// or worse, press buttons).
    pub(super) fn test_focus_input(&self) {
        if let Some(ui) = self.test.borrow().ui.as_ref() {
            let input = ui.input.clone();
            // After the page is mapped, or the grab does not stick.
            glib::idle_add_local_once(move || {
                input.grab_focus();
            });
        }
    }

    fn test_reset(self: &Rc<Self>) {
        {
            let mut st = self.test.borrow_mut();
            st.start_us = None;
            st.events.clear();
            st.report = None;
            st.resetting = true;
        }
        if let Some(ui) = self.test.borrow().ui.as_ref() {
            ui.input.buffer().set_text("");
            ui.input.set_editable(true);
        }
        self.test.borrow_mut().resetting = false;
        self.test_show_prompt();
        self.test_render();
        self.test_focus_input();
    }

    fn test_show_prompt(&self) {
        let prompt = self.test_prompt();
        let tr = self.tr();
        let st = self.test.borrow();
        let Some(ui) = st.ui.as_ref() else { return };
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
        drop(st);
        self.test_mark();
    }

    /// Shades the sample by how far the typed text matches it.
    fn test_mark(&self) {
        let st = self.test.borrow();
        let Some(ui) = st.ui.as_ref() else { return };
        let typed = text_of(&ui.input.buffer());
        let prompt = text_of(&ui.prompt);
        let matched = typed.chars().zip(prompt.chars()).take_while(|(a, b)| a == b).count();
        let (start, end) = ui.prompt.bounds();
        ui.prompt.remove_tag_by_name("cursor", &start, &end);
        ui.prompt.apply_tag_by_name("pending", &start, &end);
        if st.report.is_some() {
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
        let typed_n = typed.chars().count();
        let secs = self.test_elapsed().as_secs_f64();
        let wpm =
            if secs > 2.0 { format!("{:.0}", typed_n as f64 / 5.0 / (secs / 60.0)) } else { "--".into() };
        ui.fields.set_label(&if tr.zh() {
            format!(
                "{} 字符  │  已打 {}  │  用时 {}  │  {} WPM",
                count(prompt.chars().count()),
                count(typed_n),
                clock(secs as u64),
                wpm
            )
        } else {
            format!(
                "{} chars  │  typed {}  │  time {}  │  {} WPM",
                count(prompt.chars().count()),
                count(typed_n),
                clock(secs as u64),
                wpm
            )
        });
        ui.finish.set_sensitive(st.report.is_none() && typed_n >= MIN_CHARS);
    }

    fn test_changed(self: &Rc<Self>) {
        if self.test.borrow().resetting {
            return;
        }
        self.test_mark();
        let done = {
            let st = self.test.borrow();
            st.report.is_none()
                && st.ui.as_ref().is_some_and(|ui| {
                    let typed = text_of(&ui.input.buffer());
                    !typed.is_empty() && typed.trim_end() == text_of(&ui.prompt).trim_end()
                })
        };
        if done {
            // Let the last key-up arrive before analysing.
            let app = Rc::clone(self);
            glib::timeout_add_local_once(Duration::from_millis(250), move || app.test_finish());
        }
    }

    fn test_finish(self: &Rc<Self>) {
        let (prompt, events) = {
            let st = self.test.borrow();
            if st.report.is_some() {
                return;
            }
            (self.test_prompt(), st.events.clone())
        };
        let typed = events.iter().filter(|e| matches!(e, SampleEvent::Insert { .. })).count();
        if typed < MIN_CHARS {
            return;
        }
        let base = {
            let c = self.config.borrow();
            c.advanced.unwrap_or_else(|| c.preset.model())
        };
        let layout = Arc::clone(&self.layout.borrow());
        let report = fit(&prompt, &events, &layout, base);
        if let Some(ui) = self.test.borrow().ui.as_ref() {
            ui.input.set_editable(false);
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
        self.test.borrow_mut().report = Some(report);
        self.test_mark();
        self.test_render();
    }

    fn test_apply(self: &Rc<Self>) {
        let Some(report) = self.test.borrow().report.clone() else { return };
        let applied = report.items.iter().filter(|i| i.reliable).count();
        self.update_config(|c| {
            c.advanced = Some(report.model);
            c.macros = Macros::default();
            c.model = ModelKind::Human;
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
        let st = self.test.borrow();
        let Some(ui) = st.ui.as_ref() else { return };
        while let Some(child) = ui.results.first_child() {
            ui.results.remove(&child);
        }
        let Some(report) = st.report.as_ref() else { return };
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
            let name = label(
                &format!(
                    "{}{}",
                    if tr.zh() { p.zh } else { p.en },
                    if p.unit.is_empty() { String::new() } else { format!("  {}", p.unit) }
                ),
                &["name"],
            );
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
            chip.set_halign(gtk::Align::End);
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

fn text_of(buffer: &gtk::TextBuffer) -> String {
    let (s, e) = buffer.bounds();
    buffer.text(&s, &e, false).to_string()
}
