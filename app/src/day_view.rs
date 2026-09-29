use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, NaiveTime, Timelike};
use gettextrs::gettext;
use gtk::{gio, glib};
use knotbook_core::{
    Block, BlockId, Day, DayFile, EditError, LocationKey, ProjectSlug, RemovedText, SaveError,
    Vault,
};

use crate::alert::show_error;
use crate::format::{
    DAY_KINDS, format_date, format_duration, format_full_date, format_span, format_time, kind_name,
};
use crate::markdown_view::MarkdownView;
use crate::project_picker::{project_markup, project_popover};
use crate::standup_dialog::StandupDialog;
use crate::task_list_view::TaskListView;
use crate::timeline::Timeline;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/day_view.ui")]
    pub struct DayView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub date: Cell<NaiveDate>,
        /// The day shown, if it has a file.
        pub file: RefCell<Option<DayFile>>,
        /// The block shown in the panel, kept there when the day is saved.
        pub shown_block: RefCell<Option<BlockId>>,
        /// The pending save of the texts being typed.
        pub text_save: RefCell<Option<glib::SourceId>>,
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
        pub work_hours_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub work_popover: TemplateChild<gtk::Popover>,
        #[template_child]
        pub work_start: TemplateChild<gtk::SpinButton>,
        #[template_child]
        pub work_end: TemplateChild<gtk::SpinButton>,
        #[template_child]
        pub day_scroll: TemplateChild<gtk::ScrolledWindow>,
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
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayView {
        const NAME: &'static str = "KnotbookDayView";
        type Type = super::DayView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            MarkdownView::ensure_type();
            TaskListView::ensure_type();
            Timeline::ensure_type();
            klass.bind_template();
            klass.install_action("day.close-block", None, |view, _, _| {
                view.imp().split_view.set_show_sidebar(false);
            });
            klass.install_action("day.create", None, |view, _, _| view.create_day());
            klass.install_action("day.set-work-hours", None, |view, _, _| {
                let imp = view.imp();
                let start = spin_time(&imp.work_start);
                let end = spin_time(&imp.work_end);
                imp.work_popover.popdown();
                view.update(|day| {
                    (day.work_start, day.work_end) = (Some(start), Some(end));
                    Ok(())
                });
            });
            klass.install_action("day.new-block", None, |view, _, _| view.new_block());
            klass.install_action("day.standup", None, |view, _, _| view.show_standup());
            klass.install_action("day.delete-block", None, |view, _, _| view.delete_block());
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
            klass.install_action("day.remove-work-hours", None, |view, _, _| {
                view.imp().work_popover.popdown();
                view.update(|day| {
                    (day.work_start, day.work_end) = (None, None);
                    Ok(())
                });
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
                    view.update(|day| day.move_block(&id, time_of(start), time_of(end)));
                }
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
        for spin in [
            &imp.work_start,
            &imp.work_end,
            &imp.block_start,
            &imp.block_end,
        ] {
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
        self.save_texts_now();
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
                self.action_set_enabled("day.new-block", false);
                imp.stack.set_visible_child_name("empty");
            }
            Err(err) => {
                imp.file.replace(None);
                self.action_set_enabled("day.new-block", false);
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
        self.action_set_enabled("day.new-block", true);
        self.show_details(&file.day);
        let is_today = file.day.date == Local::now().date_naive();
        imp.timeline.set_day(&self.vault(), &file.day, is_today);
        imp.file.replace(Some(file));
        imp.stack.set_visible_child_name("day");
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
        let hours = match (day.work_start, day.work_end) {
            (Some(start), Some(end)) => format_span(start, end),
            _ => gettext("No work hours"),
        };
        imp.work_hours_label.set_label(&hours);
        self.action_set_enabled("day.remove-work-hours", day.work_start.is_some());
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

    /// Applies `change` to the day shown and saves it.
    ///
    /// On failure the day is read again, so that the view shows what is in
    /// the file.
    fn update(&self, change: impl FnOnce(&mut Day) -> Result<(), EditError>) {
        self.save_texts_now();
        if let Err(err) = self.save(change) {
            self.show_save_error(&err);
            self.show_date(self.date());
        }
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
        if imp.text_save.borrow().is_some() {
            // Saving reads the changed file and keeps both changes.
            self.save_texts_now();
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

    /// Saves the texts a second after the last change, or when they are left.
    fn setup_text_editing(&self) {
        let imp = self.imp();
        for view in [&*imp.note_view, &*imp.block_text] {
            view.connect_edited(glib::clone!(
                #[weak(rename_to = day_view)]
                self,
                move |_| day_view.save_texts_later()
            ));
        }
        imp.block_title.connect_changed(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |entry| {
                // Filling in the title of the block shown is no edit.
                if view
                    .shown_block()
                    .is_some_and(|block| block.title != entry.text())
                {
                    view.save_texts_later();
                }
            }
        ));
        imp.block_title.connect_activate(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.save_texts_now()
        ));
        let editors: [&gtk::Widget; 3] = [
            imp.note_view.upcast_ref(),
            imp.block_text.upcast_ref(),
            imp.block_title.upcast_ref(),
        ];
        for editor in editors {
            let focus = gtk::EventControllerFocus::new();
            focus.connect_leave(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_| view.save_texts_now()
            ));
            editor.add_controller(focus);
        }
    }

    fn save_texts_later(&self) {
        let imp = self.imp();
        if let Some(source) = imp.text_save.take() {
            source.remove();
        }
        let source = glib::timeout_add_local_once(
            Duration::from_secs(1),
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move || {
                    view.imp().text_save.take();
                    view.save_texts();
                }
            ),
        );
        imp.text_save.replace(Some(source));
    }

    /// Saves the texts being typed, if there are unsaved changes.
    pub fn save_texts_now(&self) {
        if let Some(source) = self.imp().text_save.take() {
            source.remove();
            self.save_texts();
        }
    }

    /// Saves day note, title and text of the block shown as they are typed.
    /// Texts that were not edited are left alone, with any headings written
    /// outside the app.
    ///
    /// On failure the typed texts stay, so that nothing gets lost.
    fn save_texts(&self) {
        let imp = self.imp();
        let Some(note) = imp.file.borrow().as_ref().map(|file| file.day.note.clone()) else {
            return;
        };
        let edited =
            |view: &MarkdownView, saved: &str| (!view.shows(saved)).then(|| view.markdown());
        let note = edited(&imp.note_view, &note);
        let block = self.shown_block().map(|block| {
            let title = imp.block_title.text().to_string();
            (block.id, title, edited(&imp.block_text, &block.text))
        });
        let result = self.save(|day| {
            if let Some(note) = note {
                day.note = note;
            }
            if let Some((id, title, text)) = block {
                day.set_block_title(&id, &title)?;
                if let Some(text) = text {
                    day.set_block_text(&id, &text)?;
                }
            }
            Ok(())
        });
        if let Err(err) = result {
            self.show_save_error(&err);
        }
    }

    /// The block shown in the panel, as saved.
    fn shown_block(&self) -> Option<Block> {
        let imp = self.imp();
        let id = imp.shown_block.borrow();
        let file = imp.file.borrow();
        let blocks = &file.as_ref()?.day.blocks;
        blocks
            .iter()
            .find(|block| Some(&block.id) == id.as_ref())
            .cloned()
    }

    /// Adds a block of `project` from `start` to `end`, in minutes of the
    /// day, and shows it in the panel.
    fn add_block(&self, start: u32, end: u32, project: ProjectSlug) {
        let vault = self.vault();
        let mut id = None;
        self.update(|day| {
            id =
                Some(day.add_block(time_of(start), time_of(end), project, "", vault.projects())?);
            Ok(())
        });
        if let Some(id) = id {
            self.show_block_by_id(&id);
        }
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
    /// of work or of the grid.
    fn new_block(&self) {
        let imp = self.imp();
        let anchor = {
            let file = imp.file.borrow();
            let day = &file
                .as_ref()
                .expect("new blocks need a day with a file")
                .day;
            let start = day
                .work_start
                .unwrap_or(self.vault().config().grid.day_start);
            // A block into the next day leaves no room after it.
            day.blocks
                .iter()
                .map(|block| block.span().1)
                .filter(|&end| end < 24 * 60)
                .max()
                .unwrap_or(start.num_seconds_from_midnight() / 60)
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

    /// Deletes the block shown, asking what happens to its text.
    fn delete_block(&self) {
        let imp = self.imp();
        let id = imp
            .shown_block
            .borrow()
            .clone()
            .expect("the delete button belongs to the block shown");
        let has_text = imp
            .file
            .borrow()
            .as_ref()
            .and_then(|file| file.day.blocks.iter().find(|block| block.id == id))
            .is_some_and(|block| !block.text.is_empty());
        if !has_text {
            self.update(|day| day.remove_block(&id, RemovedText::Discard));
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&gettext("Delete Block?")),
            Some(&gettext(
                "The text of the block can be moved to the end of the day note",
            )),
        );
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("discard", &gettext("_Delete Text")),
            ("move", &gettext("_Move to Note")),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("move", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("move"));
        dialog.set_close_response("cancel");
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_, response| {
                    let text = match response {
                        "discard" => RemovedText::Discard,
                        "move" => RemovedText::MoveToNote,
                        _ => return,
                    };
                    view.update(|day| day.remove_block(&id, text));
                }
            ),
        );
        dialog.present(Some(self));
    }

    fn block_id(&self, index: usize) -> BlockId {
        let file = self.imp().file.borrow();
        let day = &file
            .as_ref()
            .expect("blocks are only shown with their day")
            .day;
        day.blocks[index].id.clone()
    }

    /// Shows the block `id` in the panel, or closes the panel if the day
    /// has no such block.
    pub fn show_block_by_id(&self, id: &BlockId) {
        let imp = self.imp();
        let index = imp
            .file
            .borrow()
            .as_ref()
            .and_then(|file| file.day.blocks.iter().position(|block| block.id == *id));
        match index {
            Some(index) => self.show_block(index),
            None => imp.split_view.set_show_sidebar(false),
        }
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

    /// Lets the spin buttons of work hours and block times show and take
    /// times, and fills them when their popover opens.
    fn setup_time_popovers(&self) {
        let imp = self.imp();
        for spin in [
            &imp.work_start,
            &imp.work_end,
            &imp.block_start,
            &imp.block_end,
        ] {
            spin.connect_output(|spin| {
                spin.set_text(&format_time(spin_time(spin)));
                glib::Propagation::Stop
            });
            spin.connect_input(|spin| {
                Some(
                    NaiveTime::parse_from_str(spin.text().trim(), "%H:%M")
                        .map(|time| f64::from(time.num_seconds_from_midnight() / 60))
                        .map_err(|_| ()),
                )
            });
        }
        // Without work hours, the popover suggests those of the blocks.
        imp.work_popover.connect_show(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| {
                let imp = view.imp();
                let file = imp.file.borrow();
                let day = &file.as_ref().expect("the popover belongs to a day").day;
                let vault = view.vault();
                let grid = &vault.config().grid;
                let start = day
                    .work_start
                    .or(day.blocks.first().map(|block| block.start))
                    .unwrap_or(grid.day_start);
                let end = day
                    .work_end
                    .or(day.blocks.last().map(|block| block.end))
                    .unwrap_or(grid.day_end);
                set_spin_time(&imp.work_start, start);
                set_spin_time(&imp.work_end, end);
            }
        ));
        imp.block_time_popover.connect_show(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| {
                let imp = view.imp();
                let id = imp.shown_block.borrow().clone();
                let file = imp.file.borrow();
                let day = &file.as_ref().expect("the popover belongs to a day").day;
                let block = day
                    .blocks
                    .iter()
                    .find(|block| Some(&block.id) == id.as_ref())
                    .expect("the popover belongs to the block shown");
                set_spin_time(&imp.block_start, block.start);
                set_spin_time(&imp.block_end, block.end);
            }
        ));
    }

    /// Shows title, time, project and text of the block `index` in the panel.
    fn show_block(&self, index: usize) {
        let imp = self.imp();
        // Saving texts does not reorder the blocks, so `index` stays valid.
        self.save_texts_now();
        imp.timeline.select(Some(index));
        let vault = self.vault();
        let file = imp.file.borrow();
        let block = &file
            .as_ref()
            .expect("blocks are only shown with their day")
            .day
            .blocks[index];
        let is_other = imp.shown_block.borrow().as_ref() != Some(&block.id);
        imp.shown_block.replace(Some(block.id.clone()));
        let project = vault.project(&block.project);
        let project_name = vault.project_name(&block.project);
        // Keeps what is being typed into the same block, see `show_details`.
        if is_other || imp.block_title.text() != block.title {
            imp.block_title.set_text(&block.title);
        }
        imp.block_title.set_placeholder_text(Some(project_name));
        imp.block_time_button
            .set_label(&format_span(block.start, block.end));
        imp.block_project_label.set_label(&project.map_or_else(
            || glib::markup_escape_text(project_name).to_string(),
            project_markup,
        ));
        let id = block.id.clone();
        imp.block_project_button.set_popover(Some(&project_popover(
            &vault,
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |project| {
                    let vault = view.vault();
                    view.update(|day| day.set_block_project(&id, project, vault.projects()));
                }
            ),
        )));
        if is_other || !imp.block_text.shows(&block.text) {
            imp.block_text.set_markdown(&block.text);
        }
        imp.split_view.set_show_sidebar(true);
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

/// The time `minute` minutes after midnight; the end of the day is midnight.
fn time_of(minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(minute / 60 % 24, minute % 60, 0)
        .expect("the timeline stays within a day")
}

/// The time of `spin`, whose value counts the minutes of the day.
fn spin_time(spin: &gtk::SpinButton) -> NaiveTime {
    let minutes = spin.value() as u32;
    NaiveTime::from_hms_opt(minutes / 60, minutes % 60, 0)
        .expect("the spin button stays within a day")
}

fn set_spin_time(spin: &gtk::SpinButton, time: NaiveTime) {
    spin.set_value(f64::from(time.num_seconds_from_midnight() / 60));
}

/// The weekday and the full date of `date`, in the user's language.
fn date_titles(date: NaiveDate) -> (String, String) {
    (format_date(date, "%A"), format_full_date(date))
}
