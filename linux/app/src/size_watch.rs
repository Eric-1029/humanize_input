//! A one-child container that reports its width whenever it is allocated,
//! so the layout can follow the window without polling every frame.

use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

type Callback = Box<dyn Fn(i32)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SizeWatch {
        pub callback: RefCell<Option<Callback>>,
        pub last: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SizeWatch {
        const NAME: &'static str = "HumanizeSizeWatch";
        type Type = super::SizeWatch;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SizeWatch {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    // No layout manager: GTK would then skip size_allocate below.
    impl WidgetImpl for SizeWatch {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match self.obj().first_child() {
                Some(child) => child.measure(orientation, for_size),
                None => (0, 0, -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
            if width != self.last.replace(width) {
                // Changing the layout during allocation would re-enter it;
                // let the callback run right after this pass.
                let watch = self.obj().clone();
                glib::idle_add_local_once(move || {
                    if let Some(cb) = watch.imp().callback.borrow().as_ref() {
                        cb(watch.imp().last.get());
                    }
                });
            }
        }
    }
}

glib::wrapper! {
    pub struct SizeWatch(ObjectSubclass<imp::SizeWatch>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SizeWatch {
    /// Wraps `child`; `on_width` runs whenever the allocated width changes.
    pub fn new(child: &impl IsA<gtk::Widget>, on_width: impl Fn(i32) + 'static) -> Self {
        let watch: Self = glib::Object::new();
        *watch.imp().callback.borrow_mut() = Some(Box::new(on_width));
        child.set_parent(&watch);
        watch
    }
}
