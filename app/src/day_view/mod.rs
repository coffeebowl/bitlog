//! One day: its details, timeline, day note and tasks, with the block
//! chosen shown in a panel beside them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{
    BlockId, Day, DayFile, EditError, LocationKey, ProjectSlug, SaveError, Vault, minute_of_day,
    time_at_minute,
};
use chrono::{Local, NaiveDate, NaiveTime};
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::alert::{show_error, toast_overlay};
use crate::cross_fade::CrossFade;
use crate::format::{DAY_KINDS, format_date, format_duration, format_full_date, kind_name};
use crate::launch;
use crate::markdown_view::MarkdownView;
use crate::note_dialogs::confirm_delete;
use crate::project_picker::project_popover;
use crate::standup_dialog::StandupDialog;
use crate::task_list_view::TaskListView;
use crate::timeline::Timeline;
use crate::widgets::{SaveTimer, param};
use block_panel::spin_time;

mod block_panel;
mod editing;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/day_view.ui")]
    pub struct DayView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub date: Cell<NaiveDate>,
        /// The day shown, if it has a file.
        pub file: RefCell<Option<DayFile>>,
        /// The block shown in the panel, kept there when the day is saved.
        pub shown_block: RefCell<Option<BlockId>>,
        /// The pending save of the texts being typed.
        pub text_save: SaveTimer,
        /// The toast offering to undo reordered blocks, which belongs to
        /// the day shown.
        pub undo_toast: RefCell<Option<adw::Toast>>,
        /// The stateful actions `day.kind` and `day.location`, whose state
        /// is the value of the day shown.
        pub actions: gio::SimpleActionGroup,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub kind_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub kind_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub location_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub location_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub working_time_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub day_scroll: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub cross_fade: TemplateChild<CrossFade>,
        #[template_child]
        pub note_view: TemplateChild<MarkdownView>,
        #[template_child]
        pub tasks: TemplateChild<TaskListView>,
        #[template_child]
        pub timeline: TemplateChild<Timeline>,
        #[template_child]
        pub empty_tasks: TemplateChild<TaskListView>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub block_title: TemplateChild<gtk::Entry>,
        #[template_child]
        pub block_time_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub block_time_popover: TemplateChild<gtk::Popover>,
        #[template_child]
        pub block_start: TemplateChild<gtk::SpinButton>,
        #[template_child]
        pub block_end: TemplateChild<gtk::SpinButton>,
        #[template_child]
        pub block_project_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub block_project_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub block_text: TemplateChild<MarkdownView>,
        #[template_child]
        pub panel_handle: TemplateChild<gtk::Box>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayView {
        const NAME: &'static str = "BitLogDayView";
        type Type = super::DayView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            CrossFade::ensure_type();
            MarkdownView::ensure_type();
            TaskListView::ensure_type();
            Timeline::ensure_type();
            klass.bind_template();
            klass.install_action("day.close-block", None, |view, _, _| {
                view.imp().split_view.set_show_sidebar(false);
            });
            klass.install_action("day.create", None, |view, _, _| view.create_day());
            klass.install_action("day.new-block", None, |view, _, _| view.new_block());
            klass.install_action("day.standup", None, |view, _, _| view.show_standup());
            klass.install_action_async(
                "day.follow",
                Some(glib::VariantTy::STRING),
                |view, _, note| async move { view.follow_link(param(note.as_ref(), "notes")).await },
            );
            klass.install_action_async("day.delete-block", None, |view, _, _| async move {
                view.delete_block().await;
            });
            klass.install_action_async("day.delete", None, |view, _, _| async move {
                view.delete_day().await;
            });
            // With what is being typed saved, for the other app to see.
            klass.install_action_async("day.open-file", None, |view, _, _| async move {
                view.save_texts_now();
                launch::open_file(&view, &view.vault().day_path(view.date())).await;
            });
            klass.install_action_async("day.show-file", None, |view, _, _| async move {
                view.save_texts_now();
                launch::show_in_folder(&view, &view.vault().day_path(view.date())).await;
            });
            klass.install_action("day.set-block-time", None, |view, _, _| {
                let imp = view.imp();
                let id = imp
                    .shown_block
                    .borrow()
                    .clone()
                    .expect("the time popover belongs to the block shown");
                let start = spin_time(&imp.block_start);
                let end = spin_time(&imp.block_end);
                imp.block_time_popover.popdown();
                view.update(|day| day.move_block(&id, start, end));
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DayView {
        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            self.timeline.connect_block_activated(glib::clone!(
                #[weak]
                view,
                move |_, index| view.show_block(index)
            ));
            self.timeline.connect_span_selected(glib::clone!(
                #[weak]
                view,
                move |_, start, end| view.choose_project(start, end)
            ));
            self.timeline.connect_block_moved(glib::clone!(
                #[weak]
                view,
                move |_, index, start, end| {
                    let id = view.block_id(index);
                    view.update(|day| {
                        day.move_block(&id, time_at_minute(start), time_at_minute(end))
                    });
                }
            ));
            self.timeline.connect_block_reordered(glib::clone!(
                #[weak]
                view,
                move |_, index, place| view.reorder_block(index, place)
            ));
            // In the narrow layout the panel also closes by tapping beside it.
            self.split_view.connect_show_sidebar_notify(glib::clone!(
                #[weak]
                view,
                move |split_view| {
                    if !split_view.shows_sidebar() {
                        view.save_texts_now();
                        view.imp().shown_block.replace(None);
                        view.imp().timeline.select(None);
                    }
                }
            ));
            view.setup_detail_actions();
            view.setup_time_popovers();
            view.setup_text_editing();
            view.setup_panel_resizing();
        }
    }
    impl WidgetImpl for DayView {}
    impl NavigationPageImpl for DayView {}
}

glib::wrapper! {
    /// One day of the vault: its details and its blocks.
    pub struct DayView(ObjectSubclass<imp::DayView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DayView {
    /// Shows the days of `vault` from the next call of `show_date` on.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        if imp.vault.borrow().is_some() {
            self.save_texts_now();
        }
        imp.location_button
            .set_menu_model(Some(&location_menu(&vault)));
        let slot = vault.config().grid.slot_minutes.into();
        for spin in [&imp.block_start, &imp.block_end] {
            spin.adjustment().set_step_increment(slot);
        }
        imp.tasks.set_vault(vault.clone());
        imp.empty_tasks.set_vault(vault.clone());
        imp.vault.replace(Some(vault));
    }

    pub fn date(&self) -> NaiveDate {
        self.imp().date.get()
    }

    pub fn show_date(&self, date: NaiveDate) {
        let imp = self.imp();
        // Before anything on the page changes, which the fade starts from.
        if date != imp.date.get() {
            imp.cross_fade.fade();
        }
        self.save_texts_now();
        if date != imp.date.get()
            && let Some(toast) = imp.undo_toast.take()
        {
            toast.dismiss();
        }
        imp.date.set(date);
        let (weekday, full_date) = date_titles(date);
        imp.window_title.set_title(&weekday);
        imp.window_title.set_subtitle(&full_date);
        self.set_title(&weekday);

        imp.split_view.set_show_sidebar(false);
        let loaded = self.vault().load_day(date);
        match loaded {
            Ok(Some(file)) => self.show_day(file),
            Ok(None) => {
                imp.file.replace(None);
                self.enable_actions(false, false);
                imp.stack.set_visible_child_name("empty");
            }
            Err(err) => {
                imp.file.replace(None);
                // Another editor may be the way to repair the file.
                self.enable_actions(true, false);
                imp.error_page
                    .set_description(Some(&glib::markup_escape_text(&err.to_string())));
                imp.stack.set_visible_child_name("error");
            }
        }
        self.show_tasks();
    }

    /// Shows the open tasks, read again, on today only: they belong to no
    /// day, and past days stay a plain log.
    pub fn show_tasks(&self) {
        let imp = self.imp();
        let is_today = self.date() == Local::now().date_naive();
        // One list is on the page of a day, the other on that of a day
        // without a file; only the one on the page shown is read.
        let page = imp.stack.visible_child_name();
        for (tasks, on) in [(&*imp.tasks, "day"), (&*imp.empty_tasks, "empty")] {
            let shown = is_today && page.as_deref() == Some(on);
            tasks.set_visible(shown);
            if shown {
                tasks.reload();
            }
        }
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("a day is only shown once a vault is open")
    }

    /// Shows the day of `file` with its blocks, none of them selected.
    fn show_day(&self, file: DayFile) {
        let imp = self.imp();
        self.enable_actions(true, true);
        self.update_links();
        let vault = self.vault();
        let path = vault.day_path(file.day.date);
        for view in [&*imp.note_view, &*imp.block_text] {
            view.set_location(vault.root(), &path);
        }
        self.show_details(&file.day);
        let is_today = file.day.date == Local::now().date_naive();
        imp.timeline.set_day(&self.vault(), &file.day, is_today);
        imp.file.replace(Some(file));
        imp.stack.set_visible_child_name("day");
    }

    /// Enables the actions that need a file of the day shown, and those
    /// that need it readable.
    fn enable_actions(&self, exists: bool, readable: bool) {
        for action in ["day.open-file", "day.show-file"] {
            self.action_set_enabled(action, exists);
        }
        for action in ["day.new-block", "day.delete"] {
            self.action_set_enabled(action, readable);
        }
    }

    /// Shows everything of `day` but its blocks.
    fn show_details(&self, day: &Day) {
        let imp = self.imp();
        let vault = self.vault();
        imp.kind_label.set_label(&kind_name(&day.kind));
        imp.kind_button.set_menu_model(Some(&kind_menu(&day.kind)));
        imp.actions
            .lookup_action("kind")
            .and_downcast::<gio::SimpleAction>()
            .expect("the kind action is set up with the view")
            .set_state(&day.kind.to_variant());
        let location = day
            .location
            .as_ref()
            .map(|key| vault.config().location_name(key));
        imp.location_label.set_label(location.unwrap_or("–"));
        let location_key = day.location.as_ref().map_or("", LocationKey::as_str);
        imp.actions
            .lookup_action("location")
            .and_downcast::<gio::SimpleAction>()
            .expect("the location action is set up with the view")
            .set_state(&location_key.to_variant());
        imp.working_time_label
            .set_label(&format_duration(day.working_time(vault.projects())));
        // Keeps what is being typed.
        if !imp.note_view.shows(&day.note) {
            imp.note_view.set_markdown(&day.note);
        }
    }

    /// Creates the file of the day shown, which has none yet.
    fn create_day(&self) {
        let vault = self.vault();
        let file = vault.new_day(self.date());
        match vault.update_day(&file, |_| Ok(())) {
            Ok(saved) => {
                self.show_day(saved);
                // Tasks may have changed on the page without a file.
                self.show_tasks();
            }
            Err(err) => self.show_save_error(&err),
        }
    }

    /// Applies `change` to the day shown and saves it. Returns whether that
    /// worked.
    ///
    /// On failure the day is read again, so that the view shows what is in
    /// the file.
    fn update(&self, change: impl FnOnce(&mut Day) -> Result<(), EditError>) -> bool {
        self.save_texts_now();
        let saved = self.save(change);
        if let Err(err) = &saved {
            self.show_save_error(err);
            self.show_date(self.date());
        }
        saved.is_ok()
    }

    /// Applies `change` to the day shown, saves it and shows the result.
    fn save(
        &self,
        change: impl FnOnce(&mut Day) -> Result<(), EditError>,
    ) -> Result<(), SaveError> {
        let imp = self.imp();
        let file = imp
            .file
            .borrow()
            .clone()
            .expect("only a day with a file can be changed");
        let saved = self.vault().update_day(&file, change)?;
        self.show_file(saved);
        Ok(())
    }

    /// Shows the day again as its file is now, after it was changed
    /// elsewhere, keeping what is being typed.
    pub fn reload(&self) {
        let imp = self.imp();
        if imp.text_save.cancel() {
            // Saving reads the changed file and keeps both changes.
            self.save_texts();
            return;
        }
        let loaded = self.vault().load_day(self.date());
        match loaded {
            Ok(Some(file)) if imp.file.borrow().is_some() => self.show_file(file),
            _ => self.show_date(self.date()),
        }
    }

    /// Shows `file`, a new state of the day shown, with the same block in
    /// the panel if it is still there.
    fn show_file(&self, file: DayFile) {
        let imp = self.imp();
        let same_blocks = imp
            .file
            .borrow()
            .as_ref()
            .is_some_and(|shown| shown.day.blocks == file.day.blocks);
        if same_blocks {
            self.show_details(&file.day);
            imp.file.replace(Some(file));
            return;
        }
        self.show_day(file);
        let shown = imp.shown_block.borrow().clone();
        match shown {
            Some(id) if imp.split_view.shows_sidebar() => self.show_block_by_id(&id),
            _ => imp.split_view.set_show_sidebar(false),
        }
    }

    /// Adds a block of `project` from `start` to `end`, in minutes of the
    /// day, and shows it in the panel.
    fn add_block(&self, start: u32, end: u32, project: ProjectSlug) {
        let vault = self.vault();
        let mut id = None;
        self.update(|day| {
            id = Some(day.add_block(
                time_at_minute(start),
                time_at_minute(end),
                project,
                "",
                vault.projects(),
            )?);
            Ok(())
        });
        if let Some(id) = id {
            self.show_block_by_id(&id);
        }
    }

    /// Lets the block `index` take the place of the block `place`, offering
    /// to undo it.
    fn reorder_block(&self, index: usize, place: usize) {
        let (id, place) = (self.block_id(index), self.block_id(place));
        let before = self.block_times();
        if !self.update(|day| day.reorder_block(&id, &place)) {
            return;
        }
        let after = self.block_times();
        let moved: Vec<_> = before
            .into_iter()
            .filter(|times| !after.contains(times))
            .collect();
        let toast = adw::Toast::builder()
            .title(gettext("Blocks reordered"))
            .button_label(gettext("_Undo"))
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| {
                view.update(|day| day.move_blocks(&moved));
            }
        ));
        toast_overlay(self).add_toast(toast.clone());
        self.imp().undo_toast.replace(Some(toast));
    }

    /// Id, start and end of each block of the day shown.
    fn block_times(&self) -> Vec<(BlockId, NaiveTime, NaiveTime)> {
        let file = self.imp().file.borrow();
        let blocks = file.as_ref().map_or(&[][..], |file| &file.day.blocks);
        blocks
            .iter()
            .map(|block| (block.id.clone(), block.start, block.end))
            .collect()
    }

    /// Asks for the project of a new block from `start` to `end`, in minutes
    /// of the day.
    fn choose_project(&self, start: u32, end: u32) {
        let popover = project_popover(
            &self.vault(),
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |project| view.add_block(start, end, project)
            ),
        );
        self.imp().timeline.show_popover(&popover);
    }

    /// Shows the standup summary for the day shown.
    fn show_standup(&self) {
        // The summary is read from the files.
        self.save_texts_now();
        let vault = Vault::clone(&self.vault());
        let date = self.date();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                // It reads the projects' repositories too.
                let standup = gio::spawn_blocking(move || vault.standup(date))
                    .await
                    .expect("summarizing does not panic");
                match standup {
                    Ok(text) => StandupDialog::new(&text).present(Some(&view)),
                    Err(err) => show_error(&view, &gettext("Cannot Summarize"), &err.to_string()),
                }
            }
        ));
    }

    /// Offers one slot for a new block after the last one, or at the start
    /// of the grid.
    fn new_block(&self) {
        let imp = self.imp();
        let anchor = {
            let file = imp.file.borrow();
            let day = &file
                .as_ref()
                .expect("new blocks need a day with a file")
                .day;
            let start = self.vault().config().grid.day_start;
            // A block into the next day leaves no room after it.
            day.blocks
                .iter()
                .map(|block| block.span().1)
                .filter(|&end| end < 24 * 60)
                .max()
                .unwrap_or(minute_of_day(start))
        };
        let Some((start, end)) = imp.timeline.select_free_span(anchor) else {
            self.error_bell();
            return;
        };
        // Once the grid has its size, scroll to the span and ask there.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || {
                let imp = view.imp();
                let area = imp
                    .timeline
                    .pending_area()
                    .expect("the span is still marked");
                let point = gtk::graphene::Point::new(area.x(), area.y());
                if let Some(point) = imp.timeline.compute_point(&*imp.day_scroll, &point) {
                    let adjustment = imp.day_scroll.vadjustment();
                    let top = adjustment.value() + f64::from(point.y());
                    adjustment.clamp_page(top, top + f64::from(area.height()));
                }
                view.choose_project(start, end);
            }
        ));
    }

    /// Deletes the file of the day shown, after asking.
    async fn delete_day(&self) {
        let date = self.date();
        let body = gettext("{date} will be permanently deleted, with its blocks and note")
            .replace("{date}", &format_full_date(date));
        if !confirm_delete(self, &gettext("Delete Day?"), &body).await {
            return;
        }
        // What is being typed goes with the day.
        self.imp().text_save.cancel();
        if let Err(err) = self.vault().delete_day(date) {
            show_error(self, &gettext("Cannot Delete Day"), &err.to_string());
        }
        self.show_date(date);
    }

    fn show_save_error(&self, err: &SaveError) {
        show_error(self, &gettext("Cannot Save Day"), &err.to_string());
    }

    /// Adds the actions behind the kind and location menus, which save the
    /// value chosen.
    fn setup_detail_actions(&self) {
        let imp = self.imp();
        let kind = gio::SimpleAction::new_stateful(
            "kind",
            Some(glib::VariantTy::STRING),
            &"".to_variant(),
        );
        kind.connect_change_state(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, value| {
                let kind = variant_string(value);
                view.update(|day| {
                    day.kind = kind;
                    Ok(())
                });
            }
        ));
        let location = gio::SimpleAction::new_stateful(
            "location",
            Some(glib::VariantTy::STRING),
            &"".to_variant(),
        );
        location.connect_change_state(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, value| {
                let key = variant_string(value);
                let vault = view.vault();
                view.update(|day| {
                    let key = (!key.is_empty())
                        .then(|| key.parse())
                        .transpose()
                        .expect("the menu offers the keys of the vault");
                    day.set_location(key, vault.config())
                });
            }
        ));
        imp.actions.add_action(&kind);
        imp.actions.add_action(&location);
        self.insert_action_group("day", Some(&imp.actions));
    }
}

/// The kinds of day to choose from, with `current` among them.
fn kind_menu(current: &str) -> gio::Menu {
    let menu = gio::Menu::new();
    let other = (!DAY_KINDS.contains(&current)).then_some(current);
    for kind in DAY_KINDS.into_iter().chain(other) {
        let item = gio::MenuItem::new(Some(&kind_name(kind)), None);
        item.set_action_and_target_value(Some("day.kind"), Some(&kind.to_variant()));
        menu.append_item(&item);
    }
    menu
}

/// The locations of `vault` to choose from, and none.
fn location_menu(vault: &Vault) -> gio::Menu {
    let menu = gio::Menu::new();
    let none = gettext("None");
    let locations = vault.config().locations.iter();
    let items = [("", none.as_str())]
        .into_iter()
        .chain(locations.map(|(key, name)| (key.as_str(), name.as_str())));
    for (key, name) in items {
        let item = gio::MenuItem::new(Some(name), None);
        item.set_action_and_target_value(Some("day.location"), Some(&key.to_variant()));
        menu.append_item(&item);
    }
    menu
}

fn variant_string(value: Option<&glib::Variant>) -> String {
    value
        .and_then(glib::Variant::get)
        .expect("the menus pass strings")
}

/// The weekday and the full date of `date`, in the user's language.
fn date_titles(date: NaiveDate) -> (String, String) {
    (format_date(date, "%A"), format_full_date(date))
}
