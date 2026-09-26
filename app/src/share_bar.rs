use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

const HEIGHT: i32 = 10;
const WIDTH: i32 = 120;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct ShareBar {
        /// Colored parts from the left, each a share of the whole width.
        pub parts: RefCell<Vec<(gdk::RGBA, f32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ShareBar {
        const NAME: &'static str = "KnotbookShareBar";
        type Type = super::ShareBar;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            // The numbers beside it say the same.
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for ShareBar {}

    impl WidgetImpl for ShareBar {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Vertical => (HEIGHT, HEIGHT, -1, -1),
                _ => (WIDTH / 2, WIDTH, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            let whole = gsk::RoundedRect::from_rect(
                graphene::Rect::new(0.0, 0.0, width, height),
                height / 2.0,
            );
            snapshot.push_rounded_clip(&whole);
            let track = widget.color();
            snapshot.append_color(
                &gdk::RGBA::new(track.red(), track.green(), track.blue(), 0.1),
                whole.bounds(),
            );
            let mut x = 0.0;
            for (color, share) in self.parts.borrow().iter() {
                let part = width * share.clamp(0.0, 1.0);
                snapshot.append_color(color, &graphene::Rect::new(x, 0.0, part, height));
                x += part;
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// A thin bar filled from the left with colored parts, for shares of a
    /// whole.
    pub struct ShareBar(ObjectSubclass<imp::ShareBar>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ShareBar {
    /// A bar with `parts`, each a color and a share from 0 to 1.
    pub fn new(parts: Vec<(gdk::RGBA, f32)>) -> Self {
        let bar: Self = glib::Object::builder()
            .property("valign", gtk::Align::Center)
            .build();
        bar.set_parts(parts);
        bar
    }

    pub fn set_parts(&self, parts: Vec<(gdk::RGBA, f32)>) {
        self.imp().parts.replace(parts);
        self.queue_draw();
    }
}
