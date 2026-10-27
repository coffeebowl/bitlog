//! Changing the tasks shown: adding, editing, moving and finishing them.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{EditError, ProjectSlug, ProjectStatus, SaveError, TaskId, TaskList, TaskStatus};
use chrono::NaiveDate;
use gettextrs::gettext;
use gtk::glib;

use super::{TaskKey, TaskListView, is_due, rows, today};
use crate::alert::{show_error, toast_overlay};
use crate::format::format_short_date;
use crate::widgets::project_picker::{none_markup, optional_project_popover, project_markup};

impl TaskListView {
    /// Moves the task `dragged` above or below the task `target`, both of
    /// the one list shown.
    pub(super) fn drop_task(&self, dragged: &TaskId, target: &TaskId, below: bool) {
        let positions = self
            .position(&self.key(dragged))
            .zip(self.position(&self.key(target)));
        let Some((from, to)) = positions else {
            return;
        };
        let mut to = to + usize::from(below);
        // The task leaves its place before it is put back.
        if from < to {
            to -= 1;
        }
        let project = self.imp().project.borrow().clone();
        self.update(&project, |tasks| tasks.move_task(dragged, to));
    }

    /// Moves the task `id` of the one list shown `steps` places up or down,
    /// keeping the focus on it.
    pub(super) fn move_task(&self, id: &TaskId, steps: isize) {
        let key = self.key(id);
        let Some(index) = self
            .position(&key)
            .and_then(|index| index.checked_add_signed(steps))
        else {
            return;
        };
        // `show` keeps the focus in the row, as when moving with Alt+Up
        // while typing the title; from the menu it comes back here.
        if self.update(&key.project, |tasks| tasks.move_task(id, index)) {
            self.focus_row(&key);
        }
    }

    /// Lets the user choose the project a task added now goes to.
    pub(super) fn connect_new_project(&self) {
        self.imp()
            .new_project_button
            .set_create_popup_func(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |button| {
                    let popover = optional_project_popover(
                        &view.vault(),
                        &gettext("None"),
                        glib::clone!(
                            #[weak]
                            view,
                            move |project| {
                                view.imp().new_project.replace(project);
                                view.show_new_project();
                            }
                        ),
                    );
                    button.set_popover(Some(&popover));
                }
            ));
    }

    /// The project a task added now goes to, `None` for the global list or
    /// a project no longer offered.
    fn new_project_slug(&self) -> Option<ProjectSlug> {
        let slug = self.imp().new_project.borrow().clone()?;
        let archived = self.vault().project(&slug)?.status() == ProjectStatus::Archived;
        (!archived).then_some(slug)
    }

    /// Shows the project a task added now goes to.
    pub(super) fn show_new_project(&self) {
        let vault = self.vault();
        let project = self
            .new_project_slug()
            .and_then(|slug| vault.project(&slug));
        let markup = match project {
            Some(project) => project_markup(project),
            None => none_markup(&gettext("None")),
        };
        self.imp().new_project_label.set_markup(&markup);
    }

    /// Shows when a task added now is due.
    pub(super) fn show_new_due(&self) {
        let imp = self.imp();
        rows::show_due(&imp.new_due_button, imp.new_due.get(), today());
    }

    pub(super) fn add_task(&self) {
        let imp = self.imp();
        let title = imp.new_task.text();
        if title.trim().is_empty() {
            return;
        }
        let (project, due) = if self.all_due() {
            (self.new_project_slug(), imp.new_due.get())
        } else {
            (imp.project.borrow().clone(), None)
        };
        let added = self.update(&project, |tasks| {
            let id = tasks.add(&title, today())?;
            tasks.set_due(&id, due)
        });
        if !added {
            return;
        }
        imp.new_task.set_text("");
        if self.all_due() {
            if !is_due(due, today()) {
                toast_overlay(self).add_toast(adw::Toast::new(&not_due_title(due)));
            }
            imp.new_due.set(Some(today()));
            self.show_new_due();
        }
    }

    /// Sets when the task of `key` is due. If it is no longer shown then,
    /// offers to undo it.
    pub(super) fn set_due(&self, key: &TaskKey, due: Option<NaiveDate>) {
        let before = self.task(key).and_then(|task| task.due);
        if !self.update(&key.project, |tasks| tasks.set_due(&key.id, due)) {
            return;
        }
        if !self.all_due() || is_due(due, today()) {
            return;
        }
        let toast = adw::Toast::builder()
            .title(not_due_title(due))
            .button_label(gettext("_Undo"))
            .build();
        let key = key.clone();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.set_due(&key, before)
        ));
        toast_overlay(self).add_toast(toast);
    }

    /// Marks the task of `key` as done or dropped, offering to undo it.
    pub(super) fn finish_task(&self, key: &TaskKey, status: TaskStatus) {
        let index = self
            .position(key)
            .expect("rows show tasks of the lists shown");
        if !self.update(&key.project, |tasks| {
            tasks.set_status(&key.id, status, today())
        }) {
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
        let key = key.clone();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.reopen_task(&key, Some(index))
        ));
        toast_overlay(self).add_toast(toast);
    }

    /// Opens the task of `key` again, at `index` among the open tasks of its
    /// list if given, else at their end.
    pub(super) fn reopen_task(&self, key: &TaskKey, index: Option<usize>) {
        self.update(&key.project, |tasks| {
            tasks.set_status(&key.id, TaskStatus::Open, today())?;
            match index {
                Some(index) => tasks.move_task(&key.id, index),
                None => Ok(()),
            }
        });
    }

    /// Archives the finished tasks of the one list shown.
    pub(super) fn archive(&self) {
        let project = self.imp().project.borrow().clone();
        let Some(tasks) = self.list(&project) else {
            return;
        };
        let archived = self
            .vault()
            .archive_tasks(project.as_ref(), &tasks, today());
        if self.show_saved(&project, archived) {
            // Archived tasks can no longer be opened again.
            let overlay = toast_overlay(self);
            overlay.dismiss_all();
            overlay.add_toast(adw::Toast::new(&gettext("Finished tasks archived")));
        }
    }

    /// Applies `change` to the task list of `project`, or the global one,
    /// saves it and shows the result. Returns whether that worked; if not,
    /// the lists are read again.
    pub(super) fn update(
        &self,
        project: &Option<ProjectSlug>,
        change: impl FnOnce(&mut TaskList) -> Result<(), EditError>,
    ) -> bool {
        // A list that could not be read is read again when saving.
        let tasks = self.list(project).unwrap_or_default();
        let saved = self.vault().update_tasks(project.as_ref(), &tasks, change);
        self.show_saved(project, saved)
    }

    /// Shows the task list of `project` as saved, or tells why it could not
    /// be saved and reads the lists again. Returns whether it was saved.
    fn show_saved(
        &self,
        project: &Option<ProjectSlug>,
        saved: Result<TaskList, SaveError>,
    ) -> bool {
        match saved {
            Ok(saved) => {
                let mut lists = self.imp().lists.borrow_mut();
                match lists.iter_mut().find(|(listed, _)| listed == project) {
                    Some((_, tasks)) => *tasks = saved,
                    None => lists.push((project.clone(), saved)),
                }
                drop(lists);
                self.show();
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

/// What a toast says about a task that is not due by today, so that it is
/// not shown among the tasks due.
fn not_due_title(due: Option<NaiveDate>) -> String {
    match due {
        // Translators: A task no longer shown on the day page, as in "Task due Nov 3".
        Some(due) => gettext("Task due {date}").replace("{date}", &format_short_date(due)),
        None => gettext("Task without due date"),
    }
}
