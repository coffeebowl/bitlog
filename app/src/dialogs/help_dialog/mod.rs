//! The help: a page per topic, listed on the first page together with the
//! way to the keyboard shortcuts.

mod figures;
mod topics;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use self::figures::Figure;
use self::topics::{Block, Row, Topic};

/// The topic explaining Markdown.
pub const MARKDOWN: &str = "markdown";
/// The tag of the first page.
const OVERVIEW: &str = "overview";

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/help_dialog.ui")]
    pub struct HelpDialog {
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub topics: TemplateChild<adw::PreferencesGroup>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HelpDialog {
        const NAME: &'static str = "BitLogHelpDialog";
        type Type = super::HelpDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for HelpDialog {
        fn constructed(&self) {
            self.parent_constructed();
            for topic in topics::all() {
                self.obj().add_topic(topic);
            }
        }
    }

    impl WidgetImpl for HelpDialog {}
    impl AdwDialogImpl for HelpDialog {}
}

glib::wrapper! {
    /// Explains the app, topic by topic.
    pub struct HelpDialog(ObjectSubclass<imp::HelpDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl HelpDialog {
    /// Opens on the page of `topic`, with the way back to the others, or
    /// on the list of topics.
    pub fn new(topic: Option<&str>) -> Self {
        let dialog: Self = glib::Object::new();
        if let Some(topic) = topic {
            dialog
                .imp()
                .navigation_view
                .replace_with_tags(&[OVERVIEW, topic]);
        }
        dialog
    }

    /// Lists `topic` on the first page and adds its own page.
    fn add_topic(&self, topic: Topic) {
        let imp = self.imp();
        let entry = adw::ActionRow::builder()
            .title(&topic.title)
            .activatable(true)
            .action_name("navigation.push")
            .action_target(&topic.tag.to_variant())
            .build();
        entry.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        imp.topics.add(&entry);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&page_content(topic.blocks)));
        let page = adw::NavigationPage::with_tag(&toolbar, &topic.title, topic.tag);
        imp.navigation_view.add(&page);
    }
}

/// The blocks of a topic, one below the other, as wide as text reads well.
fn page_content(blocks: Vec<Block>) -> gtk::ScrolledWindow {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    for (index, block) in blocks.into_iter().enumerate() {
        let widget = block_widget(block);
        // The figure atop a page stands apart from the text.
        if index == 0 {
            widget.set_margin_bottom(12);
        }
        column.append(&widget);
    }
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .child(&column)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build()
}

fn block_widget(block: Block) -> gtk::Widget {
    match block {
        Block::Heading(heading) => {
            let label = text_label(&heading, false);
            label.add_css_class("title-4");
            label.set_margin_top(12);
            label.upcast()
        }
        Block::Text(text) => {
            let label = text_label(&text, true);
            // libadwaita's style for running text.
            label.add_css_class("body");
            label.upcast()
        }
        Block::Rows(rows) => {
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();
            for row in rows {
                list.append(&row_widget(row));
            }
            list.upcast()
        }
        Block::Code(code) => {
            let label = gtk::Label::builder()
                .label(code)
                .xalign(0.0)
                .margin_top(12)
                .margin_bottom(12)
                .margin_start(12)
                .margin_end(12)
                .css_classes(["monospace"])
                .build();
            // Lines are kept whole: narrow windows scroll them instead. Not
            // focusable, or a page opening with it would ring it.
            let scrolled = gtk::ScrolledWindow::builder()
                .vscrollbar_policy(gtk::PolicyType::Never)
                .focusable(false)
                .propagate_natural_height(true)
                .child(&label)
                .css_classes(["card"])
                .build();
            scrolled.upcast()
        }
        Block::Figure(kind) => Figure::new(kind).upcast(),
    }
}

fn text_label(text: &str, markup: bool) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .use_markup(markup)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .build()
}

fn row_widget(row: Row) -> adw::ActionRow {
    // Paths and Markdown are no Pango markup.
    let widget = adw::ActionRow::builder()
        .title(&row.title)
        .subtitle(row.subtitle.unwrap_or_default())
        .use_markup(false)
        .build();
    if let Some(example) = row.example {
        let label = gtk::Label::new(Some(&example));
        label.add_css_class("monospace");
        widget.add_suffix(&label);
    }
    if let Some(keys) = row.keys {
        let label = adw::ShortcutLabel::new(keys);
        label.set_valign(gtk::Align::Center);
        widget.add_suffix(&label);
    }
    widget
}
