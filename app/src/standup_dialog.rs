use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::glib;

use crate::markdown_view::MarkdownView;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/standup_dialog.ui")]
    pub struct StandupDialog {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub text: TemplateChild<MarkdownView>,
        #[template_child]
        pub copy_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StandupDialog {
        const NAME: &'static str = "BitLogStandupDialog";
        type Type = super::StandupDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            MarkdownView::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for StandupDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            self.copy_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.copy()
            ));
        }
    }

    impl WidgetImpl for StandupDialog {}
    impl AdwDialogImpl for StandupDialog {}
}

glib::wrapper! {
    /// Shows a standup summary to adjust and copy.
    pub struct StandupDialog(ObjectSubclass<imp::StandupDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl StandupDialog {
    pub fn new(text: &str) -> Self {
        let dialog: Self = glib::Object::new();
        dialog.imp().text.set_markdown(text);
        dialog
    }

    /// Copies the text as it is now, with the user's changes.
    fn copy(&self) {
        let imp = self.imp();
        self.clipboard().set_text(&imp.text.markdown());
        imp.toast_overlay
            .add_toast(adw::Toast::new(&gettext("Copied to clipboard")));
    }
}
