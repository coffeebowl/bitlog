use std::cell::{Cell, RefCell};
use std::fmt;
use std::iter;
use std::rc::Rc;
use std::str::FromStr;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{InvalidId, ProjectSlug, Task, TaskId, TaskList, TaskStatus, Vault};
use chrono::{Local, NaiveDate};
use gtk::glib;

use crate::widgets::param;
use crate::widgets::wrapping_entry::WrappingEntry;

mod editing;
mod rows;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/dev/bitlog/BitLog/task_list_view.ui")]
    #[properties(wrapper_type = super::TaskListView)]
    pub struct TaskListView {
        /// Whether done and dropped tasks are shown below the open ones.
        #[property(get, set = Self::set_show_finished)]
        pub show_finished: Cell<bool>,
        /// Whether the open tasks of all lists are shown that are due by
        /// today, by their due dates, as on the day page, instead of the
        /// tasks of one list.
        #[property(get, set = Self::set_all_due)]
        pub all_due: Cell<bool>,
        /// Whether the list hides while it shows no tasks, as the lists of
        /// projects do among all tasks.
        #[property(get, set)]
        pub hide_empty: Cell<bool>,
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The project whose tasks are shown, `None` for the global list.
        /// Not used while showing all due tasks.
        pub project: RefCell<Option<ProjectSlug>>,
        /// The task lists shown that could be read, with their projects.
        pub lists: RefCell<Vec<(Option<ProjectSlug>, TaskList)>>,
        /// One row per open task, in front of `new_task`.
        pub rows: RefCell<Vec<TaskRow>>,
        /// Whether rows are being replaced.
        pub showing: Cell<bool>,
        /// The project a task added while showing all due tasks goes to,
        /// `None` for the global list.
        pub new_project: RefCell<Option<ProjectSlug>>,
        /// When a task added while showing all due tasks is due.
        pub new_due: Cell<Option<NaiveDate>>,
        /// Whether the heading is shown; not on the task page, which has it
        /// as its title.
        #[property(
            name = "show-heading",
            type = bool,
            get = |imp: &Self| imp.heading.is_visible(),
            set = |imp: &Self, show| imp.heading.set_visible(show),
            default = true
        )]
        #[template_child]
        pub heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub error_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub new_task: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub new_project_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub new_project_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub new_due_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub finished_heading: TemplateChild<gtk::Box>,
        #[template_child]
        pub finished_list: TemplateChild<gtk::ListBox>,
    }

    impl TaskListView {
        fn set_show_finished(&self, show: bool) {
            self.show_finished.set(show);
            self.obj().show();
        }

        fn set_all_due(&self, all_due: bool) {
            self.all_due.set(all_due);
            self.new_project_button.set_visible(all_due);
            self.new_due_button.set_visible(all_due);
            self.obj().show();
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TaskListView {
        const NAME: &'static str = "BitLogTaskListView";
        type Type = super::TaskListView;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("tasks.archive", None, |view, _, _| view.archive());
            let id = Some(glib::VariantTy::STRING);
            klass.install_action("tasks.move-up", id, |view, _, id| {
                view.move_task(&param(id, "tasks"), -1);
            });
            klass.install_action("tasks.move-down", id, |view, _, id| {
                view.move_task(&param(id, "tasks"), 1);
            });
            klass.install_action("tasks.drop", id, |view, _, key| {
                view.finish_task(&param(key, "tasks"), TaskStatus::Dropped);
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for TaskListView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.new_task.connect_entry_activated(glib::clone!(
                #[weak]
                obj,
                move |_| obj.add_task()
            ));
            self.new_due.set(Some(today()));
            obj.show_new_due();
            obj.connect_new_due();
            obj.connect_new_project();
        }
    }
    impl WidgetImpl for TaskListView {}
    impl BinImpl for TaskListView {}
}

glib::wrapper! {
    /// The tasks of the global task list or of a project: the open ones to
    /// tick off, edit, sort and add to, with the finished ones below. Or the
    /// open tasks of all lists that are due by today, to tick off, edit and
    /// add to.
    pub struct TaskListView(ObjectSubclass<imp::TaskListView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// A task and the list it is on, written `slug/id` for the list of a
/// project and `id` for the global one, as actions take it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskKey {
    project: Option<ProjectSlug>,
    id: TaskId,
}

impl fmt::Display for TaskKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.project {
            Some(project) => write!(f, "{project}/{}", self.id),
            None => write!(f, "{}", self.id),
        }
    }
}

impl FromStr for TaskKey {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, InvalidId> {
        Ok(match value.split_once('/') {
            Some((project, id)) => Self {
                project: Some(project.parse()?),
                id: id.parse()?,
            },
            None => Self {
                project: None,
                id: value.parse()?,
            },
        })
    }
}

/// The row of an open task, with the entry its title is edited in.
#[derive(Debug)]
pub struct TaskRow {
    key: TaskKey,
    row: gtk::ListBoxRow,
    title: WrappingEntry,
}

impl TaskRow {
    fn has_focus(&self) -> bool {
        self.row
            .state_flags()
            .contains(gtk::StateFlags::FOCUS_WITHIN)
    }
}

/// Where the focus was in the rows of open tasks, so that it can be put
/// back once they are replaced.
#[derive(Debug)]
struct Focus {
    task: TaskKey,
    /// Where the cursor was in the title, if the focus was there.
    cursor: Option<i32>,
    /// The title typed but not saved yet, which is saved when the title is
    /// left.
    typed: Option<glib::GString>,
}

/// The date tasks are added, done or dropped on.
fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// Whether an open task due on `due` is shown among the tasks due by `today`.
fn is_due(due: Option<NaiveDate>, today: NaiveDate) -> bool {
    due.is_some_and(|due| due <= today)
}

impl TaskListView {
    /// Shows the tasks of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    /// Shows the tasks of `project`, or the global ones, from the next call
    /// of `reload` on.
    pub fn set_project(&self, project: Option<ProjectSlug>) {
        self.imp().project.replace(project);
    }

    /// Shows `markup` as the heading instead of "Tasks".
    pub fn set_heading_markup(&self, markup: &str) {
        self.imp().heading.set_markup(markup);
    }

    /// Hides the list if it shows no tasks and should hide then, and shows
    /// it again once it does.
    fn update_visibility(&self) {
        let imp = self.imp();
        let shows_tasks = !imp.rows.borrow().is_empty()
            || imp.finished_list.is_visible()
            || imp.error_label.is_visible();
        self.set_visible(shows_tasks || !self.hide_empty());
    }

    /// Reads the task lists and shows them. Those that cannot be read are
    /// left out, with why.
    pub fn reload(&self) {
        let imp = self.imp();
        let vault = self.vault();
        let projects: Vec<Option<ProjectSlug>> = if self.all_due() {
            let slugs = vault.projects().iter().map(|project| &project.slug);
            iter::once(None).chain(slugs.cloned().map(Some)).collect()
        } else {
            vec![imp.project.borrow().clone()]
        };
        let mut lists = Vec::new();
        let mut errors = Vec::new();
        for project in projects {
            match vault.load_tasks(project.as_ref()) {
                Ok(tasks) => lists.push((project, tasks)),
                Err(err) => errors.push(err.to_string()),
            }
        }
        imp.error_label.set_label(&errors.join("\n"));
        imp.error_label.set_visible(!errors.is_empty());
        imp.lists.replace(lists);
        if self.all_due() {
            // A day page left open overnight would offer yesterday.
            if imp.new_due.get().is_some_and(|due| due < today()) {
                imp.new_due.set(Some(today()));
            }
            self.show_new_due();
            self.show_new_project();
        }
        self.show();
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("tasks are only shown once a vault is open")
    }

    /// Shows the task lists read in new rows. They are shown anew whenever
    /// a file may have changed, also while a title is being typed in: the
    /// focus and what is typed then stay with their task.
    fn show(&self) {
        let imp = self.imp();
        let focus = self.focus();
        imp.showing.set(true);
        self.remove_rows();
        let today = today();
        let all_due = self.all_due();
        let lists = imp.lists.borrow();
        imp.list.set_visible(!lists.is_empty());
        let mut open: Vec<(&Option<ProjectSlug>, &Task)> = lists
            .iter()
            .flat_map(|(project, tasks)| tasks.tasks().iter().map(move |task| (project, task)))
            .filter(|(_, task)| task.is_open())
            .collect();
        if all_due {
            open.retain(|(_, task)| is_due(task.due, today));
            // Stable, so that tasks due on the same day keep the order of
            // their lists, and of the lists.
            open.sort_by_key(|(_, task)| task.due);
        }
        let mut rows = Vec::new();
        for (project, task) in open {
            let key = TaskKey {
                project: project.clone(),
                id: task.id.clone(),
            };
            let row = self.task_row(key, task, today);
            let index =
                i32::try_from(rows.len()).expect("a task list is far shorter than i32::MAX");
            imp.list.insert(&row.row, index);
            rows.push(row);
        }
        imp.rows.replace(rows);

        let finished: Vec<&Task> = lists
            .iter()
            .flat_map(|(_, tasks)| tasks.tasks())
            .filter(|task| !task.is_open())
            .collect();
        let show_finished = self.show_finished() && !all_due && !finished.is_empty();
        if show_finished {
            for task in finished {
                imp.finished_list.append(&self.finished_row(task));
            }
        }
        drop(lists);
        imp.finished_heading.set_visible(show_finished);
        imp.finished_list.set_visible(show_finished);
        if let Some(focus) = focus {
            self.restore_focus(focus);
        }
        imp.showing.set(false);
        self.update_visibility();
    }

    /// Moves the focus to the row of the open task `id`, unless it is in
    /// there already. Returns whether there is such a row. For the tasks of
    /// one list only, where ids are unique.
    pub fn focus_task(&self, id: &TaskId) -> bool {
        let rows = self.imp().rows.borrow();
        let key = rows
            .iter()
            .find(|row| row.key.id == *id)
            .map(|row| row.key.clone());
        drop(rows);
        key.is_some_and(|key| self.focus_row(&key))
    }

    /// Moves the focus to the row of `key`, unless it is in there already.
    /// Returns whether there is such a row.
    fn focus_row(&self, key: &TaskKey) -> bool {
        // Not borrowed while the focus moves, which runs handlers that may
        // replace the rows.
        let row = self
            .imp()
            .rows
            .borrow()
            .iter()
            .find(|row| row.key == *key)
            .map(|row| (row.row.clone(), row.has_focus()));
        match row {
            Some((row, has_focus)) => has_focus || row.grab_focus(),
            None => false,
        }
    }

    /// Where the focus is in the rows of open tasks, if it is there.
    fn focus(&self) -> Option<Focus> {
        let imp = self.imp();
        let rows = imp.rows.borrow();
        let row = rows.iter().find(|row| row.has_focus())?;
        let in_title = row
            .title
            .state_flags()
            .contains(gtk::StateFlags::FOCUS_WITHIN);
        let text = row.title.text();
        let saved = self.task(&row.key).is_some_and(|task| task.title == text);
        Some(Focus {
            task: row.key.clone(),
            cursor: in_title.then(|| row.title.buffer().cursor_position()),
            // Only what was typed: a title changed elsewhere is shown as
            // it is now unless it was being edited.
            typed: (!saved).then_some(text),
        })
    }

    /// Puts `focus`, taken from the rows replaced, back into the row of its
    /// task, if that is still shown.
    fn restore_focus(&self, focus: Focus) {
        let title = self
            .imp()
            .rows
            .borrow()
            .iter()
            .find(|row| row.key == focus.task)
            .map(|row| row.title.clone());
        let Some(title) = title else {
            return;
        };
        // The row keeps the title as saved, so the text typed still counts
        // as a change and is saved when the title is left, onto the file as
        // it is now.
        let buffer = title.buffer();
        if let Some(typed) = &focus.typed {
            buffer.set_text(typed);
        }
        match focus.cursor {
            Some(cursor) => {
                title.grab_focus();
                buffer.place_cursor(&buffer.iter_at_offset(cursor));
            }
            None => {
                self.focus_row(&focus.task);
            }
        }
    }

    fn remove_rows(&self) {
        let imp = self.imp();
        // A widget removed with the focus in it is freed while the window
        // still points to it as its focus. The next key goes to freed
        // memory, the next click makes `Root::focus` panic and the app
        // abort. This happened when a sync changed the task list while a
        // title was being typed in, so the focus is taken out first. Leaving
        // a title in `show` saves nothing, as it puts back what is typed.
        let has_focus = imp.rows.borrow().iter().any(TaskRow::has_focus)
            || imp
                .finished_list
                .state_flags()
                .contains(gtk::StateFlags::FOCUS_WITHIN);
        if let Some(root) = self.root().filter(|_| has_focus) {
            root.set_focus(None::<&gtk::Widget>);
        }
        for row in imp.rows.take() {
            imp.list.remove(&row.row);
        }
        imp.finished_list.remove_all();
    }

    /// The task list of `project`, or the global one, as read, if it could
    /// be read.
    fn list(&self, project: &Option<ProjectSlug>) -> Option<TaskList> {
        self.imp()
            .lists
            .borrow()
            .iter()
            .find(|(listed, _)| listed == project)
            .map(|(_, tasks)| tasks.clone())
    }

    /// The task of `key` as read.
    fn task(&self, key: &TaskKey) -> Option<Task> {
        self.list(&key.project)?.task(&key.id).cloned()
    }

    /// Where the task of `key` is in its list.
    fn position(&self, key: &TaskKey) -> Option<usize> {
        self.list(&key.project)?.position(&key.id)
    }

    /// The key of the task `id` of the one list shown.
    fn key(&self, id: &TaskId) -> TaskKey {
        TaskKey {
            project: self.imp().project.borrow().clone(),
            id: id.clone(),
        }
    }
}
