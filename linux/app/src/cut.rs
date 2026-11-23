//! A one-child container that cuts one corner off what it draws.
//!
//! GTK CSS has no clip-path, so the corner is cut in the snapshot: the child
//! (with its CSS background, hover wipe and focus ring) is painted through a
//! polygon. A cut corner means "you can press this"; static boxes stay square.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{graphene, gsk};

/// Corner sizes in px.
pub const CUT_SMALL: f32 = 7.0;
pub const CUT_BUTTON: f32 = 8.0;

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Default)]
    pub struct CutBin {
        pub size: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CutBin {
        const NAME: &'static str = "HumanizeCutBin";
        type Type = super::CutBin;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for CutBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for CutBin {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let c = self.size.get().min(w / 2.0).min(h / 2.0);
            if c <= 0.0 {
                self.parent_snapshot(snapshot);
                return;
            }
            // Bottom-right corner cut.
            let path = gsk::PathBuilder::new();
            path.move_to(0.0, 0.0);
            path.line_to(w, 0.0);
            path.line_to(w, h - c);
            path.line_to(w - c, h);
            path.line_to(0.0, h);
            path.close();
            snapshot.push_fill(&path.to_path(), gsk::FillRule::Winding);
            // push_fill needs at least one node inside; an empty rect keeps it valid.
            snapshot.append_color(&gtk::gdk::RGBA::TRANSPARENT, &graphene::Rect::new(0.0, 0.0, 0.0, 0.0));
            self.parent_snapshot(snapshot);
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct CutBin(ObjectSubclass<imp::CutBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CutBin {
    /// Wraps `child`, cutting `size` px off its bottom-right corner.
    pub fn new(child: &impl IsA<gtk::Widget>, size: f32) -> Self {
        let bin: Self = glib::Object::new();
        bin.imp().size.set(size);
        child.set_parent(&bin);
        bin
    }

    pub fn child(&self) -> Option<gtk::Widget> {
        self.first_child()
    }
}
