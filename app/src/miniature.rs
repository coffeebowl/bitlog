use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, gsk};

/// The width the child is laid out at, as the width of a page.
const PAGE_WIDTH: f32 = 480.0;
/// Of a page in portrait, as A4 has.
const PAGE_RATIO: f32 = std::f32::consts::SQRT_2;
/// The width a miniature asks for.
const NATURAL_WIDTH: i32 = 150;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Miniature;

    #[glib::object_subclass]
    impl ObjectSubclass for Miniature {
        const NAME: &'static str = "KnotbookMiniature";
        type Type = super::Miniature;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Miniature {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_overflow(gtk::Overflow::Hidden);
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Miniature {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        /// Any width, and as high as a page of that width.
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                return (0, NATURAL_WIDTH, -1, -1);
            }
            let width = if for_size < 0 {
                NATURAL_WIDTH
            } else {
                for_size
            };
            let height = (width as f32 * PAGE_RATIO).round() as i32;
            (height, height, -1, -1)
        }

        /// Lays the child out on a page and scales it down to fit the
        /// width, cut off at the bottom of the page.
        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(child) = self.obj().first_child().filter(|_| width > 0) else {
                return;
            };
            let scale = width as f32 / PAGE_WIDTH;
            let page_width = PAGE_WIDTH.round() as i32;
            let page_height = (height as f32 / scale).ceil() as i32;
            let (_, natural, _, _) = child.measure(gtk::Orientation::Vertical, page_width);
            let transform = gsk::Transform::new().scale(scale, scale);
            child.allocate(page_width, natural.max(page_height), -1, Some(transform));
        }
    }
}

glib::wrapper! {
    /// Its child as on a page in portrait, scaled down to a miniature of
    /// the page, as documents show in file managers.
    pub struct Miniature(ObjectSubclass<imp::Miniature>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Miniature {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let miniature: Self = glib::Object::new();
        child.set_parent(&miniature);
        miniature
    }
}
