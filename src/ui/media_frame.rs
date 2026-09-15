//! GtkAspectFrame constrains allocation but does not reserve height in a list.
//! This small composite requests height from width before an image is decoded.
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct MediaFrame {
        pub child: RefCell<Option<gtk::Widget>>,
        pub ratio: Cell<f64>,
        pub max_width: Cell<i32>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for MediaFrame {
        const NAME: &'static str = "ViewfinderMediaFrame";
        type Type = super::MediaFrame;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for MediaFrame {
        fn dispose(&self) {
            if let Some(child) = self.child.borrow_mut().take() {
                child.unparent();
            }
        }
    }
    impl WidgetImpl for MediaFrame {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                let max = self.max_width.get();
                (0, if max > 0 { max.min(640) } else { 640 }, -1, -1)
            } else {
                // Boxes ask for height at the full row width, but the child is
                // only ever allocated up to its natural width — reporting height
                // for more than that reserves dead space under the media.
                let max = self.max_width.get();
                let width = if max > 0 { for_size.min(max) } else { for_size };
                let height = (width.max(1) as f64 / self.ratio.get().max(0.1)).round() as i32;
                (height.min(520), height.min(520), -1, -1)
            }
        }
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                child.allocate(width, height, baseline, None);
            }
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(child) = self.child.borrow().as_ref() {
                self.obj().snapshot_child(child, snapshot);
            }
        }
    }
}
glib::wrapper! {
    pub struct MediaFrame(ObjectSubclass<imp::MediaFrame>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
impl MediaFrame {
    pub fn new(child: &impl IsA<gtk::Widget>, ratio: f64) -> Self {
        let frame: Self = glib::Object::new();
        frame.imp().ratio.set(ratio);
        child.set_parent(&frame);
        frame.imp().child.replace(Some(child.clone().upcast()));
        frame
    }
    /// Cap the natural width so the surface stays compact in message rows.
    pub fn set_max_width(&self, width: i32) {
        self.imp().max_width.set(width);
    }
    /// Update the reserved aspect ratio — e.g. once the decoded image's true
    /// dimensions are known — and renegotiate size.
    pub fn set_ratio(&self, ratio: f64) {
        self.imp().ratio.set(ratio);
        self.queue_resize();
    }
}
