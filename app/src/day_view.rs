use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, NaiveTime, Timelike};
use gettextrs::gettext;
use gtk::{gio, glib};
use knotbook_core::{Day, DayFile, EditError, LocationKey, SaveError, Vault};

use crate::format::{
    DAY_KINDS, format_date, format_duration, format_full_date, format_time, kind_name,
};
use crate::markdown_view::MarkdownView;
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
        pub note_view: TemplateChild<MarkdownView>,
        #[template_child]
        pub timeline: TemplateChild<Timeline>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub block_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub block_details: TemplateChild<gtk::Label>,
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
            // In the narrow layout the panel also closes by tapping beside it.
            self.split_view.connect_show_sidebar_notify(glib::clone!(
                #[weak]
                view,
                move |split_view| {
                    if !split_view.shows_sidebar() {
                        view.imp().timeline.select(None);
                    }
                }
            ));
            view.setup_detail_actions();
            view.setup_work_hours();
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
    /// Shows today of `vault`.
    pub fn set_vault(&self, vault: Rc<Vault>) {
        let imp = self.imp();
        imp.location_button
            .set_menu_model(Some(&location_menu(&vault)));
        let slot = vault.config().grid.slot_minutes.into();
        for spin in [&imp.work_start, &imp.work_end] {
            spin.adjustment().set_step_increment(slot);
        }
        imp.vault.replace(Some(vault));
        self.show_date(Local::now().date_naive());
    }

    pub fn date(&self) -> NaiveDate {
        self.imp().date.get()
    }

    pub fn show_date(&self, date: NaiveDate) {
        let imp = self.imp();
        imp.date.set(date);
        let (weekday, full_date) = date_titles(date);
        imp.window_title.set_title(&weekday);
        imp.window_title.set_subtitle(&full_date);
        self.set_title(&weekday);

        let loaded = self.vault().load_day(date);
        match loaded {
            Ok(Some(file)) => self.show_day(file),
            Ok(None) => {
                imp.split_view.set_show_sidebar(false);
                imp.file.replace(None);
                imp.stack.set_visible_child_name("empty");
            }
            Err(err) => {
                imp.split_view.set_show_sidebar(false);
                imp.file.replace(None);
                imp.error_page
                    .set_description(Some(&glib::markup_escape_text(&err.to_string())));
                imp.stack.set_visible_child_name("error");
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
        imp.split_view.set_show_sidebar(false);
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
            (Some(start), Some(end)) => format!("{}–{}", format_time(start), format_time(end)),
            _ => gettext("No work hours"),
        };
        imp.work_hours_label.set_label(&hours);
        self.action_set_enabled("day.remove-work-hours", day.work_start.is_some());
        imp.note_view.set_visible(!day.note.is_empty());
        imp.note_view.set_markdown(&day.note);
    }

    /// Creates the file of the day shown, which has none yet.
    fn create_day(&self) {
        let vault = self.vault();
        let file = vault.new_day(self.date());
        match vault.update_day(&file, |_| Ok(())) {
            Ok(saved) => self.show_day(saved),
            Err(err) => self.show_save_error(&err),
        }
    }

    /// Applies `change` to the day shown and saves it.
    ///
    /// On failure the day is read again, so that the view shows what is in
    /// the file.
    fn update(&self, change: impl FnOnce(&mut Day) -> Result<(), EditError>) {
        let imp = self.imp();
        let file = imp
            .file
            .borrow()
            .clone()
            .expect("only a day with a file can be changed");
        match self.vault().update_day(&file, change) {
            // The file may have been changed elsewhere in the meantime.
            Ok(saved) if saved.day.blocks != file.day.blocks => self.show_day(saved),
            Ok(saved) => {
                self.show_details(&saved.day);
                imp.file.replace(Some(saved));
            }
            Err(err) => {
                self.show_save_error(&err);
                self.show_date(self.date());
            }
        }
    }

    fn show_save_error(&self, err: &SaveError) {
        let dialog =
            adw::AlertDialog::new(Some(&gettext("Cannot Save Day")), Some(&err.to_string()));
        dialog.add_response("close", &gettext("_Close"));
        dialog.present(Some(self));
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

    /// Lets the spin buttons of the work hours show and take times.
    fn setup_work_hours(&self) {
        let imp = self.imp();
        for spin in [&imp.work_start, &imp.work_end] {
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
    }

    /// Shows title, time, project and text of the block `index` in the panel.
    fn show_block(&self, index: usize) {
        let imp = self.imp();
        let file = imp.file.borrow();
        let block = &file
            .as_ref()
            .expect("blocks are only shown with their day")
            .day
            .blocks[index];
        let project = self
            .vault()
            .project(&block.project)
            .map_or_else(|| block.project.to_string(), |project| project.name.clone());
        let title = if block.title.is_empty() {
            &project
        } else {
            &block.title
        };
        imp.block_title.set_label(title);
        imp.block_details.set_label(&format!(
            "{}–{} · {project}",
            format_time(block.start),
            format_time(block.end)
        ));
        imp.block_text.set_visible(!block.text.is_empty());
        imp.block_text.set_markdown(&block.text);
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
