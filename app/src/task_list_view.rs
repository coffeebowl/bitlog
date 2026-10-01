use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{EditError, SaveError, Task, TaskId, TaskList, TaskStatus, Vault};
use chrono::{Local, NaiveDate};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::alert::show_error;
use crate::format::{format_short_date, glib_date, naive_date};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/dev/bitlog/BitLog/task_list_view.ui")]
    #[properties(wrapper_type = super::TaskListView)]
    pub struct TaskListView {
        /// Whether tasks can be edited and sorted, as on the task page, or
        /// only ticked off, as in the day view.
        #[property(get, set = Self::set_full)]
        pub full: Cell<bool>,
        /// Whether done and dropped tasks are shown below the open ones.
        #[property(get, set = Self::set_show_finished)]
        pub show_finished: Cell<bool>,
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The task list shown, `None` if it cannot be read.
        pub tasks: RefCell<Option<TaskList>>,
        /// One row per open task, in front of `new_task`.
        pub rows: RefCell<Vec<(TaskId, gtk::ListBoxRow)>>,
        /// Whether rows are being replaced.
        pub showing: Cell<bool>,
        #[template_child]
        pub heading: TemplateChild<gtk::Label>,
        #[template_child]
        pub error_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub new_task: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub finished_heading: TemplateChild<gtk::Box>,
        #[template_child]
        pub finished_list: TemplateChild<gtk::ListBox>,
    }

    impl TaskListView {
        fn set_full(&self, full: bool) {
            self.full.set(full);
            // The task page has the heading as its title.
            self.heading.set_visible(!full);
            self.obj().show_again();
        }

        fn set_show_finished(&self, show: bool) {
            self.show_finished.set(show);
            self.obj().show_again();
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
                view.move_task(&task_id(id), -1);
            });
            klass.install_action("tasks.move-down", id, |view, _, id| {
                view.move_task(&task_id(id), 1);
            });
            klass.install_action("tasks.drop", id, |view, _, id| {
                view.finish_task(&task_id(id), TaskStatus::Dropped);
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
            self.new_task.connect_entry_activated(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.add_task()
            ));
        }
    }
    impl WidgetImpl for TaskListView {}
    impl BinImpl for TaskListView {}
}

glib::wrapper! {
    /// The tasks of the global task list: the open ones to tick off and add
    /// to, and on the task page also to edit and sort, with the finished
    /// ones below.
    pub struct TaskListView(ObjectSubclass<imp::TaskListView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// The id a task action is called with.
fn task_id(value: Option<&glib::Variant>) -> TaskId {
    value
        .and_then(|value| value.str()?.parse().ok())
        .expect("task actions are called with a task id")
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

impl TaskListView {
    /// Shows the tasks of `vault` from the next call of `reload` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        self.imp().vault.replace(Some(vault));
    }

    /// Reads the task list and shows it.
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
                for part in [
                    imp.list.upcast_ref::<gtk::Widget>(),
                    imp.finished_heading.upcast_ref(),
                    imp.finished_list.upcast_ref(),
                ] {
                    part.set_visible(false);
                }
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

    /// Shows the task list shown once more, after a property changed.
    fn show_again(&self) {
        let tasks = self.imp().tasks.borrow().clone();
        if let Some(tasks) = tasks {
            self.show(tasks);
        }
    }

    fn show(&self, tasks: TaskList) {
        let imp = self.imp();
        imp.showing.set(true);
        self.remove_rows();
        imp.error_label.set_visible(false);
        imp.list.set_visible(true);
        let today = today();
        let mut rows = Vec::new();
        for task in tasks.tasks().iter().filter(|task| task.is_open()) {
            let row = if self.full() {
                self.editable_row(task, today)
            } else {
                self.compact_row(task, today).upcast()
            };
            let index =
                i32::try_from(rows.len()).expect("a task list is far shorter than i32::MAX");
            imp.list.insert(&row, index);
            rows.push((task.id.clone(), row));
        }
        imp.rows.replace(rows);

        let finished: Vec<&Task> = tasks
            .tasks()
            .iter()
            .filter(|task| !task.is_open())
            .collect();
        let show_finished = self.show_finished() && !finished.is_empty();
        if show_finished {
            for task in finished {
                imp.finished_list.append(&self.finished_row(task));
            }
        }
        imp.finished_heading.set_visible(show_finished);
        imp.finished_list.set_visible(show_finished);
        imp.tasks.replace(Some(tasks));
        imp.showing.set(false);
    }

    /// Moves the focus to the row of the open task `id`. Returns whether
    /// there is one.
    pub fn focus_task(&self, id: &TaskId) -> bool {
        let rows = self.imp().rows.borrow();
        let row = rows.iter().find(|(task, _)| task == id).map(|(_, row)| row);
        row.is_some_and(|row| row.grab_focus())
    }

    fn remove_rows(&self) {
        let imp = self.imp();
        for (_, row) in imp.rows.take() {
            imp.list.remove(&row);
        }
        imp.finished_list.remove_all();
    }

    /// A check button that finishes the task when ticked and opens it again
    /// when unticked.
    fn check_button(&self, task: &Task) -> gtk::CheckButton {
        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .active(!task.is_open())
            .build();
        check.update_property(&[gtk::accessible::Property::Label(&task.title)]);
        let id = task.id.clone();
        check.connect_toggled(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |check| {
                if check.is_active() {
                    view.finish_task(&id, TaskStatus::Done);
                } else {
                    view.reopen_task(&id, None);
                }
            }
        ));
        check
    }

    /// An open task in the day view: tick it off, see when it is due.
    fn compact_row(&self, task: &Task, today: NaiveDate) -> adw::ActionRow {
        let check = self.check_button(task);
        let row = adw::ActionRow::builder()
            .title(&task.title)
            .use_markup(false)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        if let Some(due) = task.due {
            let label = gtk::Label::builder()
                .label(format_short_date(due))
                .tooltip_text(gettext("Due Date"))
                .build();
            label.add_css_class(if due < today { "error" } else { "dim-label" });
            row.add_suffix(&label);
        }
        row
    }

    /// An open task on the task page: drag it, tick it off, edit its title
    /// and due date, move or drop it.
    fn editable_row(&self, task: &Task, today: NaiveDate) -> gtk::ListBoxRow {
        let handle = gtk::Image::builder()
            .icon_name("list-drag-handle-symbolic")
            .tooltip_text(gettext("Drag to Move"))
            .css_classes(["dim-label"])
            .build();
        let title = gtk::Entry::builder()
            .text(&task.title)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        title.update_property(&[gtk::accessible::Property::Label(&gettext("Title"))]);

        let target = |action: &str| format!("tasks.{action}::{}", task.id);
        let menu = gio::Menu::new();
        menu.append(Some(&gettext("Move _Up")), Some(&target("move-up")));
        menu.append(Some(&gettext("Move _Down")), Some(&target("move-down")));
        menu.append(Some(&gettext("D_rop")), Some(&target("drop")));
        let more = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(gettext("More"))
            .menu_model(&menu)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();

        let content = gtk::Box::builder()
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(6)
            .build();
        content.append(&handle);
        content.append(&self.check_button(task));
        content.append(&title);
        content.append(&self.due_button(task, today));
        content.append(&more);
        let row = gtk::ListBoxRow::builder()
            .activatable(false)
            .child(&content)
            .build();

        let shortcuts = gtk::ShortcutController::new();
        for (key, action) in [
            (gdk::Key::Up, "tasks.move-up"),
            (gdk::Key::Down, "tasks.move-down"),
        ] {
            shortcuts.add_shortcut(
                gtk::Shortcut::builder()
                    .trigger(&gtk::KeyvalTrigger::new(key, gdk::ModifierType::ALT_MASK))
                    .action(&gtk::NamedAction::new(action))
                    .arguments(&task.id.as_str().to_variant())
                    .build(),
            );
        }
        row.add_controller(shortcuts);

        // Saved on Enter and when the title is left.
        let save_title = glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[weak]
            title,
            #[strong(rename_to = id)]
            task.id,
            #[strong(rename_to = saved)]
            task.title,
            move || {
                // Leaving rows as they are replaced is no edit.
                if !view.imp().showing.get() && title.text() != saved {
                    view.update(|tasks| tasks.set_title(&id, &title.text()));
                }
            }
        );
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[strong]
            save_title,
            move |_| save_title()
        ));
        title.add_controller(focus);
        title.connect_activate(move |_| save_title());
        self.setup_drag(&row, &handle, &task.id);
        row
    }

    /// A button showing when the task is due, which lets the user change it.
    fn due_button(&self, task: &Task, today: NaiveDate) -> gtk::MenuButton {
        let button = gtk::MenuButton::builder()
            .tooltip_text(gettext("Due Date"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        match task.due {
            Some(due) => {
                button.set_label(&format_short_date(due));
                if due < today {
                    button.add_css_class("error");
                }
            }
            None => button.set_icon_name("x-office-calendar-symbolic"),
        }
        let (id, due) = (task.id.clone(), task.due);
        // Built when needed, since each has a calendar.
        button.set_create_popup_func(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |button| button.set_popover(Some(&view.due_popover(&id, due)))
        ));
        button
    }

    fn due_popover(&self, id: &TaskId, due: Option<NaiveDate>) -> gtk::Popover {
        let calendar = gtk::Calendar::new();
        calendar.set_date(&glib_date(due.unwrap_or_else(today)));
        let remove = gtk::Button::with_mnemonic(&gettext("_Remove"));
        remove.set_sensitive(due.is_some());
        let set = gtk::Button::with_mnemonic(&gettext("_Set"));
        set.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().homogeneous(true).spacing(6).build();
        buttons.append(&remove);
        buttons.append(&set);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        content.append(&calendar);
        content.append(&buttons);
        let popover = gtk::Popover::builder()
            .child(&content)
            .default_widget(&set)
            .build();

        for (button, set_date) in [(set, true), (remove, false)] {
            let id = id.clone();
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self,
                #[weak]
                popover,
                #[weak]
                calendar,
                move |_| {
                    let due = set_date.then(|| naive_date(&calendar.date()));
                    popover.popdown();
                    view.update(|tasks| tasks.set_due(&id, due));
                }
            ));
        }
        popover
    }

    /// A done or dropped task on the task page: untick it to open it again.
    fn finished_row(&self, task: &Task) -> adw::ActionRow {
        let check = self.check_button(task);
        let row = adw::ActionRow::builder()
            .title(&task.title)
            .use_markup(false)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        let date = task.done.map(format_short_date);
        let subtitle = match (task.status, date) {
            // Translators: When a task was done, as in "Done Sep 22".
            (TaskStatus::Done, Some(date)) => gettext("Done {date}").replace("{date}", &date),
            // Translators: When a task was dropped, as in "Dropped Sep 22".
            (_, Some(date)) => gettext("Dropped {date}").replace("{date}", &date),
            (TaskStatus::Done, None) => gettext("Done"),
            (_, None) => gettext("Dropped"),
        };
        row.set_subtitle(&subtitle);
        row
    }

    /// Lets the task of `row` be dragged by `handle` and other tasks be
    /// dropped on `row`, above or below it.
    fn setup_drag(&self, row: &gtk::ListBoxRow, handle: &gtk::Image, id: &TaskId) {
        let source = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .content(&gdk::ContentProvider::for_value(&id.as_str().to_value()))
            .build();
        source.connect_drag_begin(glib::clone!(
            #[weak]
            row,
            move |source, _| {
                source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&row))), 0, 0);
            }
        ));
        handle.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        target.connect_motion(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[weak]
            row,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |_, _, _| {
                view.imp().list.drag_highlight_row(&row);
                gdk::DragAction::MOVE
            }
        ));
        target.connect_leave(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.imp().list.drag_unhighlight_row()
        ));
        let id = id.clone();
        target.connect_drop(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[weak]
            row,
            #[upgrade_or]
            false,
            move |_, value, _, y| {
                view.imp().list.drag_unhighlight_row();
                let Some(dragged) = value
                    .get::<String>()
                    .ok()
                    .and_then(|text| text.parse::<TaskId>().ok())
                else {
                    return false;
                };
                let below = y > f64::from(row.height()) / 2.0;
                let id = id.clone();
                // Not while the row that takes the drop is being replaced.
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    view,
                    move || view.drop_task(&dragged, &id, below)
                ));
                true
            }
        ));
        row.add_controller(target);
    }

    /// Moves the task `dragged` above or below the task `target`.
    fn drop_task(&self, dragged: &TaskId, target: &TaskId, below: bool) {
        let positions = self.imp().tasks.borrow().as_ref().and_then(|tasks| {
            let position = |id: &TaskId| tasks.tasks().iter().position(|task| task.id == *id);
            Some((position(dragged)?, position(target)?))
        });
        let Some((from, to)) = positions else {
            return;
        };
        let mut to = to + usize::from(below);
        // The task leaves its place before it is put back.
        if from < to {
            to -= 1;
        }
        self.update(|tasks| tasks.move_task(dragged, to));
    }

    /// Moves the task `id` `steps` places up or down, keeping the focus on it.
    fn move_task(&self, id: &TaskId, steps: isize) {
        let position = self
            .imp()
            .tasks
            .borrow()
            .as_ref()
            .and_then(|tasks| tasks.tasks().iter().position(|task| task.id == *id));
        let Some(index) = position.and_then(|index| index.checked_add_signed(steps)) else {
            return;
        };
        if self.update(|tasks| tasks.move_task(id, index)) {
            let rows = self.imp().rows.borrow();
            if let Some((_, row)) = rows.iter().find(|(shown, _)| shown == id) {
                row.grab_focus();
            }
        }
    }

    fn add_task(&self) {
        let imp = self.imp();
        let title = imp.new_task.text();
        if title.trim().is_empty() {
            return;
        }
        if self.update(|tasks| tasks.add(&title, today()).map(|_| ())) {
            imp.new_task.set_text("");
        }
    }

    /// Marks the task `id` as done or dropped, offering to undo it.
    fn finish_task(&self, id: &TaskId, status: TaskStatus) {
        let index = self
            .imp()
            .tasks
            .borrow()
            .as_ref()
            .and_then(|tasks| tasks.tasks().iter().position(|task| task.id == *id))
            .expect("rows show tasks of the list shown");
        if !self.update(|tasks| tasks.set_status(id, status, today())) {
            return;
        }
        let title = match status {
            TaskStatus::Dropped => gettext("Task dropped"),
            _ => gettext("Task done"),
        };
        let toast = adw::Toast::builder()
            .title(title)
            .button_label(gettext("_Undo"))
            .build();
        let id = id.clone();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.reopen_task(&id, Some(index))
        ));
        self.add_toast(toast);
    }

    /// Opens the task `id` again, at `index` among the open tasks if given,
    /// else at their end.
    fn reopen_task(&self, id: &TaskId, index: Option<usize>) {
        self.update(|tasks| {
            tasks.set_status(id, TaskStatus::Open, today())?;
            match index {
                Some(index) => tasks.move_task(id, index),
                None => Ok(()),
            }
        });
    }

    fn archive(&self) {
        let Some(tasks) = self.imp().tasks.borrow().clone() else {
            return;
        };
        let archived = self.vault().archive_tasks(&tasks, today());
        if self.show_saved(archived) {
            // Archived tasks can no longer be opened again.
            self.toast_overlay().dismiss_all();
            self.add_toast(adw::Toast::new(&gettext("Finished tasks archived")));
        }
    }

    fn toast_overlay(&self) -> adw::ToastOverlay {
        self.ancestor(adw::ToastOverlay::static_type())
            .and_downcast()
            .expect("task lists lie in the window's toast overlay")
    }

    fn add_toast(&self, toast: adw::Toast) {
        self.toast_overlay().add_toast(toast);
    }

    /// Applies `change` to the task list shown, saves it and shows the
    /// result. Returns whether that worked; if not, the list is read again.
    fn update(&self, change: impl FnOnce(&mut TaskList) -> Result<(), EditError>) -> bool {
        let Some(tasks) = self.imp().tasks.borrow().clone() else {
            return false;
        };
        let saved = self.vault().update_tasks(&tasks, change);
        self.show_saved(saved)
    }

    /// Shows the task list as saved, or tells why it could not be saved and
    /// reads it again. Returns whether it was saved.
    fn show_saved(&self, saved: Result<TaskList, SaveError>) -> bool {
        match saved {
            Ok(saved) => {
                self.show(saved);
                true
            }
            Err(err) => {
                show_error(self, &gettext("Cannot Save Tasks"), &err.to_string());
                self.reload();
                false
            }
        }
    }
}
