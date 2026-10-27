//! The rows of a task list view.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Task, TaskStatus};
use chrono::NaiveDate;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use super::{TaskKey, TaskListView, TaskRow, today};
use crate::format::{format_short_date, glib_date, naive_date};
use crate::widgets::project_picker::project_markup;
use crate::widgets::wrapping_entry::WrappingEntry;
use crate::widgets::{drag_handle, make_movable, set_class};

impl TaskListView {
    /// A check button that finishes the task of `key` when ticked and opens
    /// it again when unticked.
    pub(super) fn check_button(&self, key: TaskKey, task: &Task) -> gtk::CheckButton {
        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .active(!task.is_open())
            .build();
        check.update_property(&[gtk::accessible::Property::Label(&task.title)]);
        check.connect_toggled(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |check| {
                if check.is_active() {
                    view.finish_task(&key, TaskStatus::Done);
                } else {
                    view.reopen_task(&key, None);
                }
            }
        ));
        check
    }

    /// An open task: tick it off, edit its title and due date, or drop it.
    /// In one list, also drag or move it; among all due tasks, its project
    /// shows below its title instead.
    pub(super) fn task_row(&self, key: TaskKey, task: &Task, today: NaiveDate) -> TaskRow {
        let movable = !self.all_due();
        let title = WrappingEntry::new(&task.title);
        title.set_hexpand(true);
        title.set_valign(gtk::Align::Center);
        title.update_property(&[gtk::accessible::Property::Label(&gettext("Title"))]);

        let menu = gio::Menu::new();
        if movable {
            menu.append(
                Some(&gettext("Move _Up")),
                Some(&format!("tasks.move-up::{}", key.id)),
            );
            menu.append(
                Some(&gettext("Move _Down")),
                Some(&format!("tasks.move-down::{}", key.id)),
            );
        }
        menu.append(Some(&gettext("D_rop")), Some(&format!("tasks.drop::{key}")));
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
        let handle = drag_handle();
        if movable {
            content.append(&handle);
        }
        content.append(&self.check_button(key.clone(), task));
        match self.project_label(&key) {
            Some(label) => {
                let texts = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .hexpand(true)
                    .valign(gtk::Align::Center)
                    .build();
                texts.append(&title);
                texts.append(&label);
                content.append(&texts);
            }
            None => content.append(&title),
        }
        content.append(&self.due_button(key.clone(), task, today));
        content.append(&more);
        let row = gtk::ListBoxRow::builder()
            .activatable(false)
            .child(&content)
            .build();
        if movable {
            self.make_task_movable(&row, &handle, &key);
        }
        self.save_title_when_left(&title, &key, &task.title);
        TaskRow { key, row, title }
    }

    /// Among all due tasks, the project of the task of `key` as a dim line,
    /// if it has one.
    fn project_label(&self, key: &TaskKey) -> Option<gtk::Label> {
        if !self.all_due() {
            return None;
        }
        let vault = self.vault();
        let project = vault.project(key.project.as_ref()?)?;
        let label = gtk::Label::builder()
            .label(project_markup(project))
            .use_markup(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build();
        Some(label)
    }

    /// Saves the title of the task of `key`, `saved` as read, from `title`
    /// on Enter and when it is left.
    fn save_title_when_left(&self, title: &WrappingEntry, key: &TaskKey, saved: &str) {
        let save_title = glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[strong]
            key,
            #[to_owned]
            saved,
            move |text: glib::GString| {
                if text != saved {
                    view.update(&key.project, |tasks| tasks.set_title(&key.id, &text));
                }
            }
        );
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[weak]
            title,
            #[strong]
            save_title,
            move |_| {
                // Leaving rows as they are replaced is no edit: `show` puts
                // back what is typed.
                if view.imp().showing.get() {
                    return;
                }
                // Saving replaces the rows, so it waits until GTK has moved
                // the focus: rows freed while it does so leave it walking
                // freed widgets. The text is taken now, as the row may be
                // gone by then, replaced after a click on a check box.
                let text = title.text();
                let save_title = save_title.clone();
                glib::idle_add_local_once(move || save_title(text));
            }
        ));
        title.add_controller(focus);
        // Before the entry, which would insert a space.
        let keys = gtk::EventControllerKey::builder()
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        keys.connect_key_pressed(glib::clone!(
            #[weak]
            title,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if !matches!(
                    key,
                    gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter
                ) {
                    return glib::Propagation::Proceed;
                }
                save_title(title.text());
                glib::Propagation::Stop
            }
        ));
        title.add_controller(keys);
    }

    /// Lets the row of the task of `key` be moved within its list by
    /// dragging `handle` and with Alt+Up and Alt+Down.
    fn make_task_movable(&self, row: &gtk::ListBoxRow, handle: &gtk::Image, key: &TaskKey) {
        let shortcuts = gtk::ShortcutController::new();
        for (keyval, action) in [
            (gdk::Key::Up, "tasks.move-up"),
            (gdk::Key::Down, "tasks.move-down"),
        ] {
            shortcuts.add_shortcut(
                gtk::Shortcut::builder()
                    .trigger(&gtk::KeyvalTrigger::new(
                        keyval,
                        gdk::ModifierType::ALT_MASK,
                    ))
                    .action(&gtk::NamedAction::new(action))
                    .arguments(&key.id.as_str().to_variant())
                    .build(),
            );
        }
        row.add_controller(shortcuts);
        // Dragged with its list, as ids are only unique within one.
        let target = key.clone();
        let on_drop = glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |dragged: String, below| {
                if let Ok(dragged) = dragged.parse::<TaskKey>() {
                    view.drop_task(&dragged.id, &target.id, below);
                }
            }
        );
        let project = key.project.clone();
        let accepts = move |dragged: &str| {
            dragged
                .parse::<TaskKey>()
                .is_ok_and(|dragged| dragged.project == project)
        };
        make_movable(row, handle, key.to_string(), accepts, on_drop);
    }

    /// A button showing when the task of `key` is due, which lets the user
    /// change it.
    fn due_button(&self, key: TaskKey, task: &Task, today: NaiveDate) -> gtk::MenuButton {
        let button = gtk::MenuButton::builder()
            .tooltip_text(gettext("Due Date"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        show_due(&button, task.due, today);
        let due = task.due;
        // Built when needed, since each has a calendar.
        button.set_create_popup_func(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |button| {
                let key = key.clone();
                let popover = due_popover(
                    due,
                    glib::clone!(
                        #[weak]
                        view,
                        move |due| view.set_due(&key, due)
                    ),
                );
                button.set_popover(Some(&popover));
            }
        ));
        button
    }

    /// Lets the user choose when a task added now is due.
    pub(super) fn connect_new_due(&self) {
        let imp = self.imp();
        imp.new_due_button.set_create_popup_func(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |button| {
                let popover = due_popover(
                    view.imp().new_due.get(),
                    glib::clone!(
                        #[weak]
                        view,
                        move |due| {
                            view.imp().new_due.set(due);
                            view.show_new_due();
                        }
                    ),
                );
                button.set_popover(Some(&popover));
            }
        ));
    }

    /// A done or dropped task of the one list shown: untick it to open it
    /// again.
    pub(super) fn finished_row(&self, task: &Task) -> adw::ActionRow {
        let check = self.check_button(self.key(&task.id), task);
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
}

/// Shows `due` on `button`, red once it is past `today`, or a calendar
/// without a due date.
pub(super) fn show_due(button: &gtk::MenuButton, due: Option<NaiveDate>, today: NaiveDate) {
    match due {
        Some(due) => {
            button.set_label(&format_short_date(due));
            set_class(button, "error", due < today);
        }
        None => {
            button.set_icon_name("calendar-symbolic");
            button.remove_css_class("error");
        }
    }
}

/// A popover to choose a due date, starting at `due`, or to remove it.
/// Either closes it and calls `on_chosen`.
fn due_popover(
    due: Option<NaiveDate>,
    on_chosen: impl Fn(Option<NaiveDate>) + Clone + 'static,
) -> gtk::Popover {
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
        let on_chosen = on_chosen.clone();
        button.connect_clicked(glib::clone!(
            #[weak]
            popover,
            #[weak]
            calendar,
            move |_| {
                let due = set_date.then(|| naive_date(&calendar.date()));
                popover.popdown();
                on_chosen(due);
            }
        ));
    }
    popover
}
