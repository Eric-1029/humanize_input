//! Small constructors for the recurring pieces of the interface.

use std::rc::Rc;

use gtk::prelude::*;

use crate::cut::{CUT_BUTTON, CutBin};

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).build();
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

pub fn spacer() -> gtk::Box {
    let b = hbox(0);
    b.set_hexpand(true);
    b
}

/// English eyebrow over a bold title, naming a panel or page section.
/// `eyebrow` is the literal English translation of the title.
pub fn pair(eyebrow: &str, title: &str) -> gtk::Box {
    let b = vbox(2);
    b.append(&label(&format!("// {}", eyebrow.to_uppercase()), &["eyebrow"]));
    b.append(&label(title, &["title"]));
    b
}

/// A white panel with the short ink tab on top.
pub fn panel() -> gtk::Box {
    let b = vbox(0);
    b.add_css_class("panel");
    b
}

/// Panel head row: title pair on the left, `end` on the right, hairline below.
pub fn panel_head(title: &gtk::Box, end: Option<&gtk::Widget>) -> gtk::Box {
    let head = hbox(12);
    head.add_css_class("panel-head");
    head.append(title);
    head.append(&spacer());
    if let Some(end) = end {
        end.set_valign(gtk::Align::End);
        head.append(end);
    }
    head
}

/// A cut-corner button. Returns the button (for signals) and its wrapper
/// (for layout).
pub fn button(text: &str, classes: &[&str]) -> (gtk::Button, CutBin) {
    let b = gtk::Button::with_label(text);
    b.add_css_class("btn");
    for c in classes {
        b.add_css_class(c);
    }
    let wrap = CutBin::new(&b, CUT_BUTTON);
    wrap.set_valign(gtk::Align::Center);
    (b, wrap)
}

/// Primary button with a small secondary text, e.g. the hotkey.
pub fn go_button(text: &str, sub: &str) -> (gtk::Button, gtk::Label, CutBin) {
    let b = gtk::Button::new();
    b.add_css_class("btn");
    b.add_css_class("go");
    let row = hbox(0);
    row.append(&label(text, &[]));
    let sub_label = label(sub, &["sub"]);
    row.append(&sub_label);
    b.set_child(Some(&row));
    let wrap = CutBin::new(&b, CUT_BUTTON);
    wrap.set_valign(gtk::Align::Center);
    (b, sub_label, wrap)
}

/// Segmented choice. The current segment is ink-filled (not yellow).
pub fn seg(options: &[String], active: Option<usize>, on_change: impl Fn(usize) + 'static) -> gtk::Box {
    let b = hbox(0);
    b.add_css_class("seg");
    b.set_halign(gtk::Align::Start);
    let on_change = Rc::new(on_change);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, text) in options.iter().enumerate() {
        let t = gtk::ToggleButton::with_label(text);
        if let Some(f) = &first {
            t.set_group(Some(f));
        } else {
            first = Some(t.clone());
        }
        t.set_active(active == Some(i));
        let on_change = Rc::clone(&on_change);
        t.connect_toggled(move |t| {
            if t.is_active() {
                on_change(i);
            }
        });
        b.append(&t);
    }
    b
}

/// A big number with a small label under it.
pub fn figure(value: &str, caption: &str, classes: &[&str]) -> (gtk::Label, gtk::Box) {
    let b = vbox(0);
    let v = label(value, &["figure"]);
    for c in classes {
        v.add_css_class(c);
    }
    v.set_halign(gtk::Align::Start);
    b.append(&v);
    b.append(&label(caption, &["figure-label"]));
    (v, b)
}

/// 24-cell progress bar; lit cells are yellow.
pub fn cells() -> (gtk::Box, Vec<gtk::Box>) {
    let b = hbox(3);
    b.add_css_class("cells");
    b.set_homogeneous(true);
    let cells: Vec<gtk::Box> = (0..24)
        .map(|_| {
            let c = hbox(0);
            c.add_css_class("cell");
            b.append(&c);
            c
        })
        .collect();
    (b, cells)
}

pub fn set_cells(cells: &[gtk::Box], fraction: f64) {
    let lit = (fraction.clamp(0.0, 1.0) * cells.len() as f64).round() as usize;
    for (i, c) in cells.iter().enumerate() {
        if i < lit {
            c.add_css_class("on");
        } else {
            c.remove_css_class("on");
        }
    }
}

/// Bordered note with a yellow flag. Hidden while its text is empty.
pub fn notice(flag: &str) -> (gtk::Box, gtk::Label) {
    let b = hbox(0);
    b.add_css_class("notice");
    let f = label(flag, &["flag"]);
    f.set_valign(gtk::Align::Start);
    b.append(&f);
    let text = label("", &[]);
    text.set_wrap(true);
    text.set_hexpand(true);
    b.append(&text);
    b.set_visible(false);
    (b, text)
}

pub fn set_notice(notice: &(gtk::Box, gtk::Label), text: Option<&str>) {
    notice.1.set_label(text.unwrap_or(""));
    notice.0.set_visible(text.is_some_and(|t| !t.is_empty()));
}

/// Key │ value row.
pub fn kv(key: &str, value: &str) -> (gtk::Box, gtk::Label) {
    let b = hbox(12);
    b.add_css_class("kv");
    b.add_css_class("row");
    b.append(&label(key, &["key"]));
    b.append(&spacer());
    let v = label(value, &["value"]);
    b.append(&v);
    (b, v)
}

/// Settings row: name and one-line explanation left, control right.
pub fn setting_row(name: &str, hint: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = hbox(16);
    b.add_css_class("row");
    let text = vbox(2);
    text.set_hexpand(true);
    text.append(&label(name, &["name"]));
    if !hint.is_empty() {
        let h = label(hint, &["hint"]);
        h.set_wrap(true);
        text.append(&h);
    }
    b.append(&text);
    control.set_valign(gtk::Align::Center);
    b.append(control);
    b
}

/// Page title with a hatched band, used at the top of secondary pages.
pub fn page_head(title: &str, en: &str, intro: &str) -> gtk::Box {
    let b = vbox(8);
    let row = hbox(14);
    let t = label(title, &["figure", "small"]);
    row.append(&t);
    let en = label(&en.to_uppercase(), &["eyebrow"]);
    en.set_valign(gtk::Align::End);
    en.set_margin_bottom(6);
    row.append(&en);
    let band = hbox(0);
    band.set_hexpand(true);
    band.set_valign(gtk::Align::Center);
    let lead = hbox(0);
    lead.add_css_class("hatch-lead");
    band.append(&lead);
    let hatch = hbox(0);
    hatch.add_css_class("hatch");
    hatch.set_hexpand(true);
    band.append(&hatch);
    row.append(&band);
    b.append(&row);
    if !intro.is_empty() {
        let i = label(intro, &["hint"]);
        i.set_wrap(true);
        b.append(&i);
    }
    b
}
