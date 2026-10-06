use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/markdown_help_dialog.ui")]
    pub struct MarkdownHelpDialog {}

    #[glib::object_subclass]
    impl ObjectSubclass for MarkdownHelpDialog {
        const NAME: &'static str = "BitLogMarkdownHelpDialog";
        type Type = super::MarkdownHelpDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MarkdownHelpDialog {}
    impl WidgetImpl for MarkdownHelpDialog {}
    impl AdwDialogImpl for MarkdownHelpDialog {}
}

glib::wrapper! {
    /// Explains the Markdown the texts understand, with its shortcuts.
    pub struct MarkdownHelpDialog(ObjectSubclass<imp::MarkdownHelpDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MarkdownHelpDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }
}
