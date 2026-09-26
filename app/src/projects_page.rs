use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::Local;
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::glib;
use knotbook_core::{Project, ProjectSlug, SaveError, Vault};

use crate::format::{PROJECT_STATUSES, status_name};
use crate::project_dialog::ProjectDialog;
use crate::project_picker::color_dot;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/projects_page.ui")]
    pub struct ProjectsPage {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// One group per status that has projects.
        pub groups: RefCell<Vec<adw::PreferencesGroup>>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub list: TemplateChild<adw::PreferencesPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectsPage {
        const NAME: &'static str = "KnotbookProjectsPage";
        type Type = super::ProjectsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("projects.add", None, |page, _, _| page.edit(None));
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ProjectsPage {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted after a project was saved, with the changed vault
            // available through `vault()`.
            SIGNALS.get_or_init(|| vec![Signal::builder("vault-changed").build()])
        }
    }

    impl WidgetImpl for ProjectsPage {}
    impl NavigationPageImpl for ProjectsPage {}
}

glib::wrapper! {
    /// All projects of the vault, to add and edit.
    pub struct ProjectsPage(ObjectSubclass<imp::ProjectsPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ProjectsPage {
    /// Shows the projects of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    pub fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("the page is only shown with a vault")
    }

    pub fn connect_vault_changed(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "vault-changed",
            false,
            glib::closure_local!(move |page: &Self| callback(page)),
        );
    }

    /// Shows the projects of the vault, grouped by status.
    pub fn reload(&self) {
        let imp = self.imp();
        for group in imp.groups.take() {
            imp.list.remove(&group);
        }
        let vault = self.vault();
        let mut groups = Vec::new();
        for status in PROJECT_STATUSES {
            let mut projects: Vec<&Project> = vault
                .projects()
                .iter()
                .filter(|project| project.status == status)
                .collect();
            if projects.is_empty() {
                continue;
            }
            projects.sort_by_key(|project| project.name.to_lowercase());
            let group = adw::PreferencesGroup::builder()
                .title(status_name(status))
                .build();
            for project in projects {
                group.add(&self.project_row(project));
            }
            imp.list.add(&group);
            groups.push(group);
        }
        imp.stack
            .set_visible_child_name(if groups.is_empty() { "empty" } else { "list" });
        imp.groups.replace(groups);
    }

    fn project_row(&self, project: &Project) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&project.name)
            .subtitle(&project.category)
            .use_markup(false)
            .activatable(true)
            .build();
        let dot = gtk::Label::builder()
            .label(color_dot(project))
            .use_markup(true)
            .build();
        row.add_prefix(&dot);
        if project.pinned {
            let pin = gtk::Image::builder()
                .icon_name("view-pin-symbolic")
                .tooltip_text(gettext("Pinned"))
                .build();
            row.add_suffix(&pin);
        }
        let slug = project.slug.clone();
        row.connect_activated(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.edit(Some(slug.clone()))
        ));
        row
    }

    /// Asks for the details of the project `slug`, or of a new project.
    fn edit(&self, slug: Option<ProjectSlug>) {
        let vault = self.vault();
        let project = slug.as_ref().and_then(|slug| vault.project(slug));
        let dialog = ProjectDialog::new(project);
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |dialog| page.save(dialog, slug.as_ref())
        ));
        dialog.present(Some(self));
    }

    /// Saves what `dialog` holds for the project `slug`, or as a new
    /// project, and closes it. On failure it stays open.
    fn save(&self, dialog: &ProjectDialog, slug: Option<&ProjectSlug>) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        let saved: Result<(), SaveError> = match slug {
            Some(slug) => vault
                .update_project(slug, |project| dialog.apply(project))
                .map(|_| ()),
            None => {
                let slug = dialog
                    .slug()
                    .expect("new projects are saved with a valid ID");
                let mut project = Project::new(slug, &dialog.name(), Local::now().date_naive());
                dialog
                    .apply(&mut project)
                    .map_err(SaveError::from)
                    .and_then(|()| vault.add_project(project).map(|_| ()))
            }
        };
        match saved {
            Ok(()) => {
                dialog.close();
                self.set_vault(Rc::new(vault));
                self.reload();
                self.emit_by_name::<()>("vault-changed", &[]);
            }
            Err(err) => {
                let alert = adw::AlertDialog::new(
                    Some(&gettext("Cannot Save Project")),
                    Some(&err.to_string()),
                );
                alert.add_response("close", &gettext("_Close"));
                alert.present(Some(dialog));
            }
        }
    }
}
