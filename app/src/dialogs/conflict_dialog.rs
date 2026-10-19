use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::glib;

use crate::markdown_view::MarkdownView;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/conflict_dialog.ui")]
    pub struct ConflictDialog {
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub message: TemplateChild<gtk::Label>,
        #[template_child]
        pub mine_heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub theirs_heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub mine: TemplateChild<MarkdownView>,
        #[template_child]
        pub theirs: TemplateChild<MarkdownView>,
        #[template_child]
        pub keep_mine: TemplateChild<gtk::Button>,
        #[template_child]
        pub keep_theirs: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConflictDialog {
        const NAME: &'static str = "BitLogConflictDialog";
        type Type = super::ConflictDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            MarkdownView::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ConflictDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted with whether the user keeps their own version, after
            // the dialog closed.
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("chosen")
                        .param_types([bool::static_type()])
                        .build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            for (button, keep_mine) in [(&*self.keep_mine, true), (&*self.keep_theirs, false)] {
                button.connect_clicked(glib::clone!(
                    #[weak]
                    dialog,
                    move |_| {
                        dialog.force_close();
                        dialog.emit_by_name::<()>("chosen", &[&keep_mine]);
                    }
                ));
            }
        }
    }

    impl WidgetImpl for ConflictDialog {}
    impl AdwDialogImpl for ConflictDialog {}
}

glib::wrapper! {
    /// Shows two versions of a text side by side, to choose the one to
    /// keep: of a note that was changed elsewhere while it was being edited,
    /// or of a file and its sync conflict copy.
    pub struct ConflictDialog(ObjectSubclass<imp::ConflictDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ConflictDialog {
    pub fn new(name: &str, mine: &str, theirs: &str) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        imp.message.set_label(
            &gettext("“{name}” was changed elsewhere while you were editing it.")
                .replace("{name}", name),
        );
        imp.mine.set_markdown(mine);
        imp.theirs.set_markdown(theirs);
        dialog
    }

    /// Compares `original`, a text of a file, with `copy`, the same text of
    /// its sync conflict copy. It can be closed without choosing.
    pub fn compare(message: &str, original: &str, copy: &str) -> Self {
        let dialog = Self::new("", original, copy);
        let imp = dialog.imp();
        dialog.set_title(&gettext("Compare Versions"));
        dialog.set_can_close(true);
        imp.header_bar.set_show_end_title_buttons(true);
        imp.message.set_label(message);
        imp.mine_heading.set_label(&gettext("Original"));
        imp.theirs_heading.set_label(&gettext("Copy"));
        imp.keep_mine.set_label(&gettext("Keep _Original"));
        imp.keep_theirs.set_label(&gettext("Keep _Copy"));
        dialog
    }

    pub fn connect_chosen(&self, callback: impl Fn(bool) + 'static) {
        self.connect_closure(
            "chosen",
            false,
            glib::closure_local!(move |_: &Self, keep_mine: bool| callback(keep_mine)),
        );
    }
}
