use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{ProjectSlug, TaskId, Vault};
use gtk::glib;

use crate::task_list_view::TaskListView;
use crate::widgets::project_picker::project_markup;

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::TaskListsView)]
    pub struct TaskListsView {
        /// Whether done and dropped tasks are shown below the open ones.
        #[property(get, set = Self::set_show_finished)]
        pub show_finished: Cell<bool>,
        pub global: TaskListView,
        /// The lists of the projects, in their order.
        pub projects: RefCell<Vec<(ProjectSlug, TaskListView)>>,
        pub lists: gtk::Box,
        pub vault: RefCell<Option<Rc<Vault>>>,
    }

    impl Default for TaskListsView {
        fn default() -> Self {
            Self {
                show_finished: Cell::default(),
                global: glib::Object::new(),
                projects: RefCell::default(),
                lists: gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(24)
                    .build(),
                vault: RefCell::default(),
            }
        }
    }

    impl TaskListsView {
        fn set_show_finished(&self, show: bool) {
            self.show_finished.set(show);
            self.global.set_show_finished(show);
            for (_, view) in &*self.projects.borrow() {
                view.set_show_finished(show);
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TaskListsView {
        const NAME: &'static str = "BitLogTaskListsView";
        type Type = super::TaskListsView;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for TaskListsView {
        fn constructed(&self) {
            self.parent_constructed();
            // The task page has it as its title.
            self.global.set_show_heading(false);
            self.lists.append(&self.global);
            self.obj().set_child(Some(&self.lists));
        }
    }
    impl WidgetImpl for TaskListsView {}
    impl BinImpl for TaskListsView {}
}

glib::wrapper! {
    /// The global task list, with the lists of the projects that have tasks
    /// to show below it.
    pub struct TaskListsView(ObjectSubclass<imp::TaskListsView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TaskListsView {
    /// Shows the tasks of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        imp.global.set_vault(vault.clone());
        imp.vault.replace(Some(vault));
    }

    /// Reads all task lists and shows them, those of projects only while
    /// they have tasks to show. The lists of projects still there are kept,
    /// with the focus in them.
    pub fn reload(&self) {
        let imp = self.imp();
        imp.global.reload();
        let vault = imp
            .vault
            .borrow()
            .clone()
            .expect("tasks are only shown once a vault is open");
        let mut old = imp.projects.take();
        let mut views = Vec::new();
        let mut previous = imp.global.clone().upcast::<gtk::Widget>();
        for project in vault.projects() {
            let view = match old.iter().position(|(slug, _)| *slug == project.slug) {
                Some(index) => old.remove(index).1,
                None => self.add_list(&project.slug),
            };
            view.set_vault(vault.clone());
            view.set_heading_markup(&project_markup(project));
            imp.lists.reorder_child_after(&view, Some(&previous));
            view.reload();
            previous = view.clone().upcast();
            views.push((project.slug.clone(), view));
        }
        for (_, view) in old {
            // The focus must not stay in a widget that is freed, see
            // `TaskListView::remove_rows`.
            if view.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN)
                && let Some(root) = self.root()
            {
                root.set_focus(None::<&gtk::Widget>);
            }
            imp.lists.remove(&view);
        }
        imp.projects.replace(views);
    }

    fn add_list(&self, slug: &ProjectSlug) -> TaskListView {
        let view: TaskListView = glib::Object::builder()
            .property("show-finished", self.show_finished())
            .property("hide-empty", true)
            .build();
        view.set_project(Some(slug.clone()));
        self.imp().lists.append(&view);
        view
    }

    /// Moves the focus to the open task `id` of the list of `project`, or of
    /// the global one. Returns whether it is shown.
    pub fn focus_task(&self, project: Option<&ProjectSlug>, id: &TaskId) -> bool {
        let imp = self.imp();
        let Some(project) = project else {
            return imp.global.focus_task(id);
        };
        let view = imp
            .projects
            .borrow()
            .iter()
            .find(|(slug, _)| slug == project)
            .map(|(_, view)| view.clone());
        view.is_some_and(|view| view.focus_task(id))
    }
}
