use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{NotePath, Period, Project, ProjectSlug, SaveError, Vault};
use chrono::{Datelike, Local, NaiveDate, TimeDelta};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gdk, glib};

use crate::alert::show_error;
use crate::colors::color_dot;
use crate::format::{
    PROJECT_STATUSES, format_duration, format_full_date, format_short_date, status_name,
};
use crate::note_dialogs::{self, ask_note_name, note_param};
use crate::note_view::NoteView;
use crate::project_dialog::ProjectDialog;
use crate::project_view::ProjectView;
use crate::search_index::{ProjectsData, SearchIndex};

mod imp {
    use super::*;

    #[derive(Debug, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/projects_page.ui")]
    pub struct ProjectsPage {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// One group per status that has projects.
        pub groups: RefCell<Vec<adw::PreferencesGroup>>,
        /// What the list shows beside the projects, as last looked up.
        pub data: RefCell<ProjectsData>,
        /// Counts the lookups, so that one finishing after a newer one is
        /// dropped.
        pub lookups: Cell<u32>,
        /// Pushed on top of the list, owned here because they are only in
        /// the navigation view while shown.
        pub project_view: ProjectView,
        pub note_view: NoteView,
        /// Where the links to a note lie, before it is renamed.
        pub index: RefCell<SearchIndex>,
        #[template_child]
        pub nav: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub overview: TemplateChild<adw::NavigationPage>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub list: TemplateChild<adw::PreferencesPage>,
    }

    impl Default for ProjectsPage {
        fn default() -> Self {
            Self {
                vault: RefCell::default(),
                groups: RefCell::default(),
                data: RefCell::default(),
                lookups: Cell::default(),
                project_view: glib::Object::new(),
                note_view: glib::Object::new(),
                index: RefCell::default(),
                nav: TemplateChild::default(),
                overview: TemplateChild::default(),
                stack: TemplateChild::default(),
                list: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectsPage {
        const NAME: &'static str = "BitLogProjectsPage";
        type Type = super::ProjectsPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("projects.add", None, |page, _, _| page.edit(None));
            klass.install_action(
                "projects.edit",
                Some(glib::VariantTy::STRING),
                |page, _, slug| {
                    page.edit(Some(slug_param(slug)));
                },
            );
            klass.install_action(
                "projects.open",
                Some(glib::VariantTy::STRING),
                |page, _, slug| {
                    page.open_project(&slug_param(slug));
                },
            );
            klass.install_action_async(
                "notes.new",
                Some(glib::VariantTy::STRING),
                |page, _, slug| async move { page.new_note(slug_param(slug.as_ref())).await },
            );
            klass.install_action(
                "notes.open",
                Some(glib::VariantTy::STRING),
                |page, _, note| {
                    page.open_note(&note_param(note));
                },
            );
            klass.install_action_async(
                "notes.follow",
                Some(glib::VariantTy::STRING),
                |page, _, note| async move { page.follow_link(note_param(note.as_ref())).await },
            );
            klass.install_action_async(
                "notes.rename",
                Some(glib::VariantTy::STRING),
                |page, _, note| async move { page.rename_note(note_param(note.as_ref())).await },
            );
            klass.install_action_async(
                "notes.delete",
                Some(glib::VariantTy::STRING),
                |page, _, note| async move { page.delete_note(note_param(note.as_ref())).await },
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ProjectsPage {
        fn constructed(&self) {
            self.parent_constructed();
            // Back from a note, the previews show it as just typed.
            self.project_view.connect_showing(glib::clone!(
                #[weak(rename_to = note_view)]
                self.note_view,
                move |project_view| {
                    note_view.save_now();
                    project_view.update_previews();
                }
            ));
            // Back on the list, it shows the time and notes as they are now.
            self.overview.connect_showing(glib::clone!(
                #[weak(rename_to = page)]
                self.obj(),
                move |_| page.look_up()
            ));
            self.nav.connect_visible_page_notify(glib::clone!(
                #[weak(rename_to = page)]
                self.obj(),
                move |_| page.emit_by_name::<()>("shown-project-changed", &[])
            ));
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted after a project was saved, with the changed
                    // vault available through `vault()`.
                    Signal::builder("vault-changed").build(),
                    // Emitted after links in day files were changed, which
                    // watching the vault leaves out as own writes.
                    Signal::builder("days-changed").build(),
                    // Emitted when another project, or none, may be shown,
                    // see `shown_project()`.
                    Signal::builder("shown-project-changed").build(),
                ]
            })
        }
    }

    impl WidgetImpl for ProjectsPage {}
    impl NavigationPageImpl for ProjectsPage {}
}

glib::wrapper! {
    /// All projects of the vault, to add and edit, and to open one with its
    /// notes.
    pub struct ProjectsPage(ObjectSubclass<imp::ProjectsPage>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Whether the page with `tag` is in `nav`. Unlike `find_page()`, a page on
/// its way out is not, so that it is not built again while going back.
pub fn shows(nav: &adw::NavigationView, tag: &str) -> bool {
    let stack = nav.navigation_stack();
    (0..stack.n_items())
        .filter_map(|i| stack.item(i).and_downcast::<adw::NavigationPage>())
        .any(|page| page.tag().as_deref() == Some(tag))
}

fn slug_param(param: Option<&glib::Variant>) -> ProjectSlug {
    param
        .and_then(|param| param.str()?.parse().ok())
        .expect("project actions take a slug")
}

impl ProjectsPage {
    /// Shows the projects of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        imp.note_view.set_vault(vault.clone());
        imp.vault.replace(Some(vault));
    }

    /// Uses `index` to find the links to notes.
    pub fn set_index(&self, index: SearchIndex) {
        let imp = self.imp();
        imp.note_view.set_index(index.clone());
        imp.project_view.set_index(index.clone());
        imp.index.replace(index);
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

    pub fn connect_days_changed(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "days-changed",
            false,
            glib::closure_local!(move |page: &Self| callback(page)),
        );
    }

    pub fn connect_shown_project_changed(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "shown-project-changed",
            false,
            glib::closure_local!(move |page: &Self| callback(page)),
        );
    }

    /// The project open, alone or below one of its notes, if any.
    pub fn shown_project(&self) -> Option<ProjectSlug> {
        if shows(&self.imp().nav, "project") {
            self.imp().project_view.slug()
        } else {
            None
        }
    }

    /// Saves the note being typed, if there are unsaved changes.
    pub fn save_now(&self) {
        self.imp().note_view.save_now();
    }

    /// Shows the projects of the vault, grouped by status, and the project
    /// and note open as they are now.
    pub fn reload(&self) {
        let imp = self.imp();
        self.show_list();
        self.look_up();
        if shows(&imp.nav, "project") && !self.show_project_again() {
            imp.nav.pop_to_tag("projects");
        }
        if shows(&imp.nav, "note") && !imp.note_view.reload() {
            self.close_note();
        }
    }

    /// Goes back to the list of projects.
    pub fn show_overview(&self) {
        let imp = self.imp();
        imp.note_view.save_now();
        imp.nav.pop_to_tag("projects");
    }

    /// Shows the projects of the vault, grouped by status, with what was
    /// last looked up about them.
    fn show_list(&self) {
        let imp = self.imp();
        for group in imp.groups.take() {
            imp.list.remove(&group);
        }
        let vault = self.vault();
        let data = imp.data.borrow();
        let today = Local::now().date_naive();
        let mut groups = Vec::new();
        let find = |slug: &ProjectSlug, times: &[(ProjectSlug, TimeDelta)]| {
            times
                .iter()
                .find(|(other, _)| other == slug)
                .map(|(_, time)| *time)
        };
        let last_day = |slug: &ProjectSlug| {
            data.last_days
                .iter()
                .find(|(other, _)| other == slug)
                .map(|(_, date)| *date)
        };
        let mut has_projects = false;
        for status in PROJECT_STATUSES {
            let projects: Vec<&Project> = vault
                .projects()
                .iter()
                .filter(|project| project.status == status)
                .collect();
            if projects.is_empty() {
                continue;
            }
            let group = adw::PreferencesGroup::builder()
                .title(status_name(status))
                .build();
            let mut worked_this_week = false;
            for project in projects {
                let week = find(&project.slug, &data.week_times);
                worked_this_week |= week.is_some();
                let row = project_row(project, week, last_day(&project.slug), today);
                self.make_movable(&row, project);
                group.add(&row);
            }
            if worked_this_week {
                group.set_header_suffix(Some(
                    &gtk::Label::builder()
                        .label(gettext("This Week"))
                        .valign(gtk::Align::End)
                        .css_classes(["dim-label"])
                        .build(),
                ));
            }
            imp.list.add(&group);
            groups.push(group);
            has_projects = true;
        }
        imp.stack
            .set_visible_child_name(if has_projects { "list" } else { "empty" });
        imp.groups.replace(groups);
    }

    /// Lets `row` of `project` be dragged onto another project of the same
    /// status to move it there, marked by a handle at its end. On touch
    /// screens dragging starts at the handle or with a long press, so that
    /// swiping still scrolls and tapping still opens the project.
    fn make_movable(&self, row: &adw::ActionRow, project: &Project) {
        let slug = project.slug.clone();
        let status = project.status;
        let handle = gtk::Image::builder()
            .icon_name("list-drag-handle-symbolic")
            .css_classes(["dim-label"])
            .build();
        handle.set_cursor_from_name(Some("grab"));
        row.add_suffix(&handle);
        // Where the row was grabbed, and whether a finger holds it so that
        // it may be dragged.
        let grab = Rc::new(Cell::new((0.0, 0.0)));
        let held = Rc::new(Cell::new(false));

        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        // Held, the row neither scrolls nor opens when let go.
        let hold = |gesture: &gtk::Gesture, held: &Cell<bool>| {
            held.set(true);
            gesture.set_state(gtk::EventSequenceState::Claimed);
        };
        let touch = gtk::GestureClick::builder().touch_only(true).build();
        touch.group_with(&source);
        touch.connect_pressed(glib::clone!(
            #[weak]
            row,
            #[weak]
            handle,
            #[strong]
            held,
            move |gesture, _, x, _| {
                // The handle is small, so a finger may touch next to it.
                let on_handle = handle
                    .compute_bounds(&row)
                    .is_some_and(|bounds| x >= f64::from(bounds.x()) - 12.0);
                held.set(false);
                if on_handle {
                    hold(gesture.upcast_ref(), &held);
                }
            }
        ));
        let long_press = gtk::GestureLongPress::builder().touch_only(true).build();
        long_press.group_with(&source);
        long_press.connect_pressed(glib::clone!(
            #[strong]
            held,
            move |gesture, _, _| hold(gesture.upcast_ref(), &held)
        ));
        source.connect_prepare(glib::clone!(
            #[strong]
            held,
            #[strong]
            grab,
            #[strong]
            slug,
            move |source, x, y| {
                let touch = source
                    .current_event_device()
                    .is_some_and(|device| device.source() == gdk::InputSource::Touchscreen);
                if touch && !held.get() {
                    return None;
                }
                grab.set((x, y));
                Some(gdk::ContentProvider::for_value(
                    &slug.to_string().to_value(),
                ))
            }
        ));
        source.connect_drag_begin(glib::clone!(
            #[weak]
            row,
            #[strong]
            grab,
            move |_, drag| {
                // A copy of the row as it looks now, before it is dimmed.
                let picture = gtk::Picture::for_paintable(
                    &gtk::WidgetPaintable::new(Some(&row)).current_image(),
                );
                picture.set_size_request(row.width(), row.height());
                picture.add_css_class("dragged-project");
                gtk::DragIcon::for_drag(drag).set_child(Some(&picture));
                let (x, y) = grab.get();
                drag.set_hotspot(x as i32, y as i32);
                row.add_css_class("dragging");
            }
        ));
        source.connect_drag_end(glib::clone!(
            #[weak]
            row,
            #[strong]
            held,
            move |_, _, _| {
                held.set(false);
                row.remove_css_class("dragging");
            }
        ));
        row.add_controller(touch);
        row.add_controller(long_press);
        row.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        target.set_preload(true);
        // The project dragged, and whether it goes after this one, if it
        // may go here at all.
        let place = glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            row,
            #[strong]
            slug,
            #[upgrade_or]
            None,
            move |target: &gtk::DropTarget, y: f64| -> Option<(ProjectSlug, bool)> {
                let dragged: ProjectSlug = target.value()?.get::<String>().ok()?.parse().ok()?;
                let vault = page.vault();
                let same_group = vault.project(&dragged)?.status == status;
                (same_group && dragged != slug)
                    .then(|| (dragged, y > f64::from(row.height()) / 2.0))
            }
        );
        let place = Rc::new(place);
        target.connect_motion(glib::clone!(
            #[weak]
            row,
            #[strong]
            place,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |target, _, y| {
                let after = place(target, y).map(|(_, after)| after);
                set_class(&row, "drop-before", after == Some(false));
                set_class(&row, "drop-after", after == Some(true));
                if after.is_some() {
                    gdk::DragAction::MOVE
                } else {
                    gdk::DragAction::empty()
                }
            }
        ));
        target.connect_leave(glib::clone!(
            #[weak]
            row,
            move |_| {
                row.remove_css_class("drop-before");
                row.remove_css_class("drop-after");
            }
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            row,
            #[strong]
            slug,
            #[upgrade_or]
            false,
            move |target, _, _, y| {
                row.remove_css_class("drop-before");
                row.remove_css_class("drop-after");
                let Some((dragged, after)) = place(target, y) else {
                    return false;
                };
                // Moving rebuilds the list with this row, so it waits
                // until the drop is done.
                let slug = slug.clone();
                glib::idle_add_local_once(move || page.move_project(&dragged, &slug, after));
                true
            }
        ));
        row.add_controller(target);
    }

    /// Moves the project `slug` right before the project `target`, or right
    /// after it if `after`, and shows the new order.
    fn move_project(&self, slug: &ProjectSlug, target: &ProjectSlug, after: bool) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        match vault.move_project(slug, target, after) {
            Ok(()) => {
                self.set_vault(Rc::new(vault));
                self.show_list();
                self.emit_by_name::<()>("vault-changed", &[]);
            }
            Err(err) => show_error(self, &gettext("Cannot Move Project"), &err.to_string()),
        }
    }

    /// Looks up the time spent this week and the last days worked in the
    /// background, then shows them.
    fn look_up(&self) {
        let imp = self.imp();
        let lookup = imp.lookups.get() + 1;
        imp.lookups.set(lookup);
        let vault = self.vault();
        let week = Period::Week.range(Local::now().date_naive(), vault.config().week.first_day);
        let index = imp.index.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let data = index.projects(&vault, week).await;
                let imp = page.imp();
                if imp.lookups.get() != lookup {
                    return;
                }
                match data {
                    Ok(data) => {
                        imp.data.replace(data);
                        page.show_list();
                    }
                    Err(err) => glib::g_warning!("bitlog", "{err}"),
                }
            }
        ));
    }

    /// Shows the notes `changed`, which were changed elsewhere, as they are
    /// now.
    pub fn notes_changed(&self, changed: &[NotePath]) {
        let imp = self.imp();
        imp.note_view.update_links();
        let slug = imp.project_view.slug();
        if shows(&imp.nav, "project")
            && changed
                .iter()
                .any(|note| Some(note.project()) == slug.as_ref())
        {
            self.show_project_again();
        }
        let shown = imp.note_view.note();
        if shows(&imp.nav, "note")
            && changed.iter().any(|note| Some(note) == shown.as_ref())
            && !imp.note_view.reload()
        {
            self.close_note();
        }
    }

    /// Shows the assets of the project open as they are now, if they are
    /// among the assets of `projects`, which were changed elsewhere.
    pub fn assets_changed(&self, projects: &[ProjectSlug]) {
        let imp = self.imp();
        let shown = imp.project_view.slug();
        if shows(&imp.nav, "project") && shown.is_some_and(|slug| projects.contains(&slug)) {
            imp.project_view.show_assets();
        }
    }

    /// Shows the project open with its notes as they are now, and marks the
    /// links of the note open again. Returns whether the project is still
    /// there.
    fn show_project_again(&self) -> bool {
        let imp = self.imp();
        imp.note_view.update_links();
        let vault = self.vault();
        let project = imp
            .project_view
            .slug()
            .and_then(|slug| vault.project(&slug).cloned());
        if let Some(project) = &project {
            imp.project_view.show(&vault, project);
        }
        project.is_some()
    }

    /// Shows the note `note` below its project, from where going back leads
    /// to the project's notes.
    pub fn show_note(&self, note: &NotePath) {
        self.open_project(note.project());
        self.open_note(note);
    }

    /// Shows the project `slug` with its notes, from where going back leads
    /// to the list.
    pub fn open_project(&self, slug: &ProjectSlug) {
        let imp = self.imp();
        let vault = self.vault();
        let Some(project) = vault.project(slug) else {
            return;
        };
        imp.note_view.save_now();
        imp.project_view.show(&vault, project);
        let shown = imp.nav.visible_page();
        if shown.as_ref() != Some(imp.project_view.upcast_ref()) {
            imp.nav.pop_to_tag("projects");
            imp.nav.push(&imp.project_view);
        }
    }

    fn open_note(&self, note: &NotePath) {
        let imp = self.imp();
        // A link may lead to a note of another project, which going back
        // should show.
        if imp.project_view.slug().as_ref() != Some(note.project()) {
            let vault = self.vault();
            if let Some(project) = vault.project(note.project()) {
                imp.project_view.show(&vault, project);
            }
            self.emit_by_name::<()>("shown-project-changed", &[]);
        }
        match imp.note_view.show_note(note) {
            Ok(()) if !shows(&imp.nav, "note") => imp.nav.push(&imp.note_view),
            Ok(()) => {}
            Err(err) => show_error(self, &gettext("Cannot Open Note"), &err.to_string()),
        }
    }

    /// Goes back from the note, which is no longer there.
    fn close_note(&self) {
        let imp = self.imp();
        imp.note_view.forget();
        imp.nav.pop_to_page(&imp.project_view);
    }

    async fn new_note(&self, project: ProjectSlug) {
        let Some(name) = ask_note_name(
            self,
            &gettext("New Note"),
            &gettext("_Create"),
            &project,
            "",
        )
        .await
        else {
            return;
        };
        let created = self
            .vault()
            .create_note(&project, &name, Local::now().date_naive());
        match created {
            Ok(note) => {
                self.show_project_again();
                self.open_note(&note);
            }
            Err(err) => show_error(self, &gettext("Cannot Create Note"), &err.to_string()),
        }
    }

    /// Opens the note a wiki link points to, or offers to create it.
    async fn follow_link(&self, note: NotePath) {
        if let Some(note) = note_dialogs::follow_link(self, &self.vault(), &note).await {
            // It may be new.
            self.show_project_again();
            self.open_note(&note);
        }
    }

    async fn rename_note(&self, note: NotePath) {
        let imp = self.imp();
        // Renaming may change the links in the note shown, and the index
        // has to find what was just typed.
        imp.note_view.save_now();
        let index = imp.index.borrow().clone();
        let Some((renamed, updated_links)) =
            note_dialogs::rename_note(self, &self.vault(), &index, &note).await
        else {
            return;
        };
        if updated_links {
            self.emit_by_name::<()>("days-changed", &[]);
        }
        self.show_project_again();
        if imp.note_view.note() == Some(note) {
            imp.note_view.forget();
            self.open_note(&renamed);
        } else {
            imp.note_view.reload();
        }
    }

    async fn delete_note(&self, note: NotePath) {
        if !note_dialogs::delete_note(self, &self.vault(), &note).await {
            return;
        }
        if self.imp().note_view.note() == Some(note) {
            self.close_note();
        }
        self.show_project_again();
    }

    /// Asks for the details of the project `slug`, or of a new project.
    fn edit(&self, slug: Option<ProjectSlug>) {
        let vault = self.vault();
        let project = slug.as_ref().and_then(|slug| vault.project(slug));
        let dialog = ProjectDialog::new(project, vault.projects());
        // Read when the dialog opens, so that saving only writes a change.
        let repo = vault
            .repo_paths()
            .map(|mut repos| slug.as_ref().and_then(|slug| repos.remove(slug)))
            .map_err(|err| err.to_string());
        dialog.show_repo(repo.clone());
        dialog.connect_save(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |dialog| page.save(dialog, slug.as_ref(), &repo)
        ));
        dialog.present(Some(self));
    }

    /// Saves what `dialog` holds for the project `slug`, or as a new
    /// project, and closes it. On failure it stays open. `repo` is the
    /// repository shown when the dialog opened.
    fn save(
        &self,
        dialog: &ProjectDialog,
        slug: Option<&ProjectSlug>,
        repo: &Result<Option<PathBuf>, String>,
    ) {
        // A copy shares the record of own writes, so watching the vault
        // goes on as before.
        let mut vault = Vault::clone(&self.vault());
        let saved: Result<(), SaveError> = match slug {
            Some(slug) => match repo {
                Ok(repo) if *repo != dialog.repo() => {
                    vault.set_repo_path(slug, dialog.repo().as_deref())
                }
                _ => Ok(()),
            }
            .and_then(|()| vault.update_project(slug, |project| dialog.apply(project)))
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
        if let Err(err) = saved {
            show_error(dialog, &gettext("Cannot Save Project"), &err.to_string());
            return;
        }
        // The repository of a new project can only be set once the project
        // exists. Should that fail, the project is still added, so the
        // dialog closes anyway.
        let repo_saved = match (slug, dialog.repo()) {
            (None, Some(repo)) => {
                let slug = dialog.slug().expect("the project was added");
                vault.set_repo_path(&slug, Some(&repo))
            }
            _ => Ok(()),
        };
        dialog.close();
        self.set_vault(Rc::new(vault));
        self.reload();
        self.emit_by_name::<()>("vault-changed", &[]);
        if let Err(err) = repo_saved {
            show_error(self, &gettext("Cannot Save Repository"), &err.to_string());
        }
    }
}

/// A project in the list, with the time spent on it this week, `week`, and
/// the last day it was worked on, `last_day`, if any, as seen `today`.
fn project_row(
    project: &Project,
    week: Option<TimeDelta>,
    last_day: Option<NaiveDate>,
    today: NaiveDate,
) -> adw::ActionRow {
    let mut subtitle = vec![project.category.clone()];
    match last_day {
        Some(date) if date == today => subtitle.push(gettext("last worked today")),
        Some(date) => subtitle
            .push(gettext("last worked on {date}").replace("{date}", &format_day(date, today))),
        None => {}
    }
    subtitle.retain(|part| !part.is_empty());
    let row = adw::ActionRow::builder()
        .title(&project.name)
        .subtitle(subtitle.join(" · "))
        .use_markup(false)
        .activatable(true)
        .action_name("projects.open")
        .action_target(&project.slug.to_string().to_variant())
        .build();
    let dot = gtk::Label::builder()
        .label(color_dot(&project.color))
        .use_markup(true)
        .build();
    row.add_prefix(&dot);
    if let Some(week) = week {
        let time = gtk::Label::builder()
            .label(format_duration(week))
            .tooltip_text(gettext("Time spent this week"))
            .css_classes(["dim-label", "numeric"])
            .build();
        row.add_suffix(&time);
    }
    row
}

/// Adds the style class `class` to `widget`, or removes it.
fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// `date` without the year if it is the year of `today`.
fn format_day(date: NaiveDate, today: NaiveDate) -> String {
    if date.year() == today.year() {
        format_short_date(date)
    } else {
        format_full_date(date)
    }
}
