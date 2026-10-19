use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct WrappingEntry;

    #[glib::object_subclass]
    impl ObjectSubclass for WrappingEntry {
        const NAME: &'static str = "BitLogWrappingEntry";
        type Type = super::WrappingEntry;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for WrappingEntry {
        fn constructed(&self) {
            self.parent_constructed();
            let entry = self.obj();
            entry.set_wrap_mode(gtk::WrapMode::WordChar);
            entry.set_accepts_tab(false);
            entry.add_css_class("inline");
            let buffer = entry.buffer();
            buffer.connect_insert_text(|buffer, iter, text| {
                if text.contains(['\n', '\r']) {
                    buffer.stop_signal_emission_by_name("insert-text");
                    buffer.insert(iter, &text.replace("\r\n", " ").replace(['\n', '\r'], " "));
                }
            });
            buffer.connect_changed(glib::clone!(
                #[weak]
                entry,
                move |_| entry.queue_resize()
            ));
        }
    }

    impl WidgetImpl for WrappingEntry {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        // A text view reports the height of its text at the width it had
        // last, not at the width offered, and the resize it asks for once
        // it knows better gets lost while it is allocated. So the height is
        // measured here, as a label measures it.
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                return self.parent_measure(orientation, for_size);
            }
            let entry = self.obj();
            let layout = entry.create_pango_layout(Some(&entry.text()));
            if for_size >= 0 {
                // The text view keeps a pixel for the cursor.
                let width = for_size - entry.left_margin() - entry.right_margin() - 1;
                layout.set_width(width.max(1) * pango::SCALE);
                layout.set_wrap(pango::WrapMode::WordChar);
            }
            let height = layout.pixel_size().1 + entry.top_margin() + entry.bottom_margin();
            (height, height, -1, -1)
        }
    }

    impl TextViewImpl for WrappingEntry {}
}

glib::wrapper! {
    /// A field for one line of text that wraps where an entry would scroll
    /// it out of view, growing with it. Line breaks pasted become spaces;
    /// Enter is left to the caller.
    pub struct WrappingEntry(ObjectSubclass<imp::WrappingEntry>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl WrappingEntry {
    pub fn new(text: &str) -> Self {
        let entry: Self = glib::Object::new();
        let buffer = entry.buffer();
        // Undo should not empty the field.
        buffer.begin_irreversible_action();
        buffer.set_text(text);
        buffer.end_irreversible_action();
        entry
    }

    pub fn text(&self) -> glib::GString {
        let buffer = self.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
    }
}
