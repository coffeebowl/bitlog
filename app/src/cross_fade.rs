use std::cell::{OnceCell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::widgets::redraw_animation;

const FADE_MS: u32 = 200;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct CrossFade {
        /// A picture of what the child showed before.
        pub before: RefCell<Option<gdk::Paintable>>,
        pub fade: OnceCell<adw::TimedAnimation>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CrossFade {
        const NAME: &'static str = "BitLogCrossFade";
        type Type = super::CrossFade;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for CrossFade {
        fn constructed(&self) {
            self.parent_constructed();
            let cross_fade = self.obj();
            let fade = redraw_animation(&*cross_fade, FADE_MS);
            fade.connect_done(glib::clone!(
                #[weak]
                cross_fade,
                move |_| {
                    cross_fade.imp().before.take();
                    cross_fade.queue_draw();
                }
            ));
            self.fade.set(fade).expect("constructed runs once");
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for CrossFade {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let Some(child) = widget.first_child() else {
                return;
            };
            let before = self.before.borrow();
            let (Some(before), Some(fade)) = (before.as_ref(), self.fade.get()) else {
                widget.snapshot_child(&child, snapshot);
                return;
            };
            // Mixed pixel by pixel, so that what both show stays as it is.
            snapshot.push_cross_fade(fade.value());
            before.snapshot(snapshot, widget.width().into(), widget.height().into());
            snapshot.pop();
            widget.snapshot_child(&child, snapshot);
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// Its child, which fades over from what it showed before on `fade`.
    pub struct CrossFade(ObjectSubclass<imp::CrossFade>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CrossFade {
    /// Takes a picture of the child, to fade over from it once the child
    /// has changed; call it before changing anything.
    pub fn fade(&self) {
        let Some(child) = self.first_child() else {
            return;
        };
        let picture = gtk::WidgetPaintable::new(Some(&child)).current_image();
        self.imp().before.replace(Some(picture));
        if let Some(fade) = self.imp().fade.get() {
            fade.play();
        }
    }
}
