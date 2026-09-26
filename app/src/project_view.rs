use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};
use knotbook_core::{NotePath, Project, ProjectSlug, Vault};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/project_view.ui")]
    pub struct ProjectView {
        /// The project shown.
        pub slug: RefCell<Option<ProjectSlug>>,
        pub rows: RefCell<Vec<adw::ActionRow>>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub new_note_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub edit_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub notes_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub empty_new_note_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectView {
        const NAME: &'static str = "KnotbookProjectView";
        type Type = super::ProjectView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ProjectView {}
    impl WidgetImpl for ProjectView {}
    impl NavigationPageImpl for ProjectView {}
}

glib::wrapper! {
    /// One project with its notes.
    pub struct ProjectView(ObjectSubclass<imp::ProjectView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ProjectView {
    /// The project shown, if any.
    pub fn slug(&self) -> Option<ProjectSlug> {
        self.imp().slug.borrow().clone()
    }

    /// Shows `project` of `vault` with its notes.
    pub fn show(&self, vault: &Vault, project: &Project) {
        let imp = self.imp();
        imp.slug.replace(Some(project.slug.clone()));
        self.set_title(&project.name);
        imp.window_title.set_title(&project.name);
        imp.window_title.set_subtitle(&project.category);
        let target = project.slug.to_string().to_variant();
        for button in [
            &*imp.new_note_button,
            &*imp.edit_button,
            &*imp.empty_new_note_button,
        ] {
            button.set_action_target_value(Some(&target));
        }

        for row in imp.rows.take() {
            imp.notes_group.remove(&row);
        }
        let notes = match vault.notes(&project.slug) {
            Ok(notes) => notes,
            Err(err) => {
                imp.error_page.set_description(Some(&err.to_string()));
                imp.stack.set_visible_child_name("error");
                return;
            }
        };
        let rows: Vec<adw::ActionRow> = notes.iter().map(note_row).collect();
        for row in &rows {
            imp.notes_group.add(row);
        }
        imp.stack
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        imp.rows.replace(rows);
    }
}

fn note_row(note: &NotePath) -> adw::ActionRow {
    let target = note.to_string().to_variant();
    let row = adw::ActionRow::builder()
        .title(note.name())
        .use_markup(false)
        .activatable(true)
        .action_name("notes.open")
        .action_target(&target)
        .build();
    let menu_button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(gettext("Note Menu"))
        .menu_model(&note_menu(note))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&menu_button);
    row
}

/// Renaming and deleting the note `note`.
pub fn note_menu(note: &NotePath) -> gio::Menu {
    let target = note.to_string().to_variant();
    let menu = gio::Menu::new();
    for (label, action) in [
        (gettext("_Rename…"), "notes.rename"),
        (gettext("_Delete"), "notes.delete"),
    ] {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&target));
        menu.append_item(&item);
    }
    menu
}
