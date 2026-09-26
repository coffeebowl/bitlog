use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate};
use gettextrs::gettext;
use gtk::glib;
use knotbook_core::{EditError, Task, TaskId, TaskList, TaskStatus, Vault};

use crate::format::format_date;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/open_tasks.ui")]
    pub struct OpenTasks {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The task list shown, `None` if it cannot be read.
        pub tasks: RefCell<Option<TaskList>>,
        /// One row per open task, in front of `new_task`.
        pub rows: RefCell<Vec<adw::ActionRow>>,
        #[template_child]
        pub error_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub new_task: TemplateChild<adw::EntryRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OpenTasks {
        const NAME: &'static str = "KnotbookOpenTasks";
        type Type = super::OpenTasks;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for OpenTasks {
        fn constructed(&self) {
            self.parent_constructed();
            self.new_task.connect_entry_activated(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.add_task()
            ));
        }
    }
    impl WidgetImpl for OpenTasks {}
    impl BinImpl for OpenTasks {}
}

glib::wrapper! {
    /// The open tasks of the global task list, to tick off and add to.
    pub struct OpenTasks(ObjectSubclass<imp::OpenTasks>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl OpenTasks {
    /// Shows the tasks of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    /// Reads the task list and shows its open tasks.
    pub fn reload(&self) {
        let imp = self.imp();
        let loaded = self.vault().load_tasks();
        match loaded {
            Ok(tasks) => self.show(tasks),
            Err(err) => {
                imp.tasks.replace(None);
                self.remove_rows();
                imp.error_label.set_label(&err.to_string());
                imp.error_label.set_visible(true);
                imp.list.set_visible(false);
            }
        }
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("tasks are only shown once a vault is open")
    }

    fn show(&self, tasks: TaskList) {
        let imp = self.imp();
        imp.error_label.set_visible(false);
        imp.list.set_visible(true);
        self.remove_rows();
        let today = Local::now().date_naive();
        let rows: Vec<adw::ActionRow> = tasks
            .tasks()
            .iter()
            .filter(|task| task.is_open())
            .map(|task| self.task_row(task, today))
            .collect();
        for (index, row) in rows.iter().enumerate() {
            let index = i32::try_from(index).expect("a task list is far shorter than i32::MAX");
            imp.list.insert(row, index);
        }
        imp.rows.replace(rows);
        imp.tasks.replace(Some(tasks));
    }

    fn remove_rows(&self) {
        let imp = self.imp();
        for row in imp.rows.take() {
            imp.list.remove(&row);
        }
    }

    fn task_row(&self, task: &Task, today: NaiveDate) -> adw::ActionRow {
        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .build();
        check.update_property(&[gtk::accessible::Property::Label(&task.title)]);
        let row = adw::ActionRow::builder()
            .title(&task.title)
            .use_markup(false)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        if let Some(due) = task.due {
            // Translators: A due date without the year, as in "Sep 30". See
            // the GLib documentation of g_date_time_format() for the codes.
            let label = gtk::Label::builder()
                .label(format_date(due, &gettext("%b %-d")))
                .tooltip_text(gettext("Due Date"))
                .build();
            label.add_css_class(if due < today { "error" } else { "dim-label" });
            row.add_suffix(&label);
        }
        let id = task.id.clone();
        check.connect_toggled(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |check| {
                if check.is_active() {
                    view.finish_task(&id);
                }
            }
        ));
        row
    }

    fn add_task(&self) {
        let imp = self.imp();
        let title = imp.new_task.text();
        if title.trim().is_empty() {
            return;
        }
        let today = Local::now().date_naive();
        if self.update(|tasks| tasks.add(&title, today).map(|_| ())) {
            imp.new_task.set_text("");
        }
    }

    /// Marks the task `id` as done, offering to undo it.
    fn finish_task(&self, id: &TaskId) {
        let index = self
            .imp()
            .tasks
            .borrow()
            .as_ref()
            .and_then(|tasks| tasks.tasks().iter().position(|task| task.id == *id))
            .expect("rows show tasks of the list shown");
        let today = Local::now().date_naive();
        if !self.update(|tasks| tasks.set_status(id, TaskStatus::Done, today)) {
            return;
        }
        let toast = adw::Toast::builder()
            .title(gettext("Task done"))
            .button_label(gettext("_Undo"))
            .build();
        let id = id.clone();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.reopen_task(&id, index)
        ));
        self.ancestor(adw::ToastOverlay::static_type())
            .and_downcast::<adw::ToastOverlay>()
            .expect("task lists lie in the window's toast overlay")
            .add_toast(toast);
    }

    /// Opens the task `id` again, back at `index` among the open tasks.
    fn reopen_task(&self, id: &TaskId, index: usize) {
        let today = Local::now().date_naive();
        self.update(|tasks| {
            tasks.set_status(id, TaskStatus::Open, today)?;
            tasks.move_task(id, index)
        });
    }

    /// Applies `change` to the task list shown, saves it and shows the
    /// result. Returns whether that worked; if not, the list is read again.
    fn update(&self, change: impl FnOnce(&mut TaskList) -> Result<(), EditError>) -> bool {
        let Some(tasks) = self.imp().tasks.borrow().clone() else {
            return false;
        };
        let saved = self.vault().update_tasks(&tasks, change);
        match saved {
            Ok(saved) => {
                self.show(saved);
                true
            }
            Err(err) => {
                let dialog = adw::AlertDialog::new(
                    Some(&gettext("Cannot Save Tasks")),
                    Some(&err.to_string()),
                );
                dialog.add_response("close", &gettext("_Close"));
                dialog.present(Some(self));
                self.reload();
                false
            }
        }
    }
}
