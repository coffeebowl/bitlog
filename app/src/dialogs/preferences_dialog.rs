use std::cell::RefCell;
use std::path::PathBuf;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{LocationKey, Vault, VaultConfig, time_at_minute};
use chrono::{NaiveDate, NaiveTime, TimeDelta, Weekday};
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::alert::show_error;
use crate::format::{format_date, format_duration, format_time};
use crate::widgets::{Choices, set_class};
use crate::window::Window;

/// The block lengths offered, besides the one set.
const SLOT_MINUTES: [u32; 6] = [5, 10, 15, 20, 30, 60];

/// The steps of the times offered for the day view, besides those set.
const TIME_STEP_MINUTES: u32 = 30;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/preferences_dialog.ui")]
    pub struct PreferencesDialog {
        /// The vault folder, which note templates have to lie in.
        pub root: RefCell<PathBuf>,
        /// What the combo rows offer.
        pub locations: Choices<Option<LocationKey>>,
        pub first_days: Choices<Weekday>,
        pub slots: Choices<u32>,
        pub day_starts: Choices<NaiveTime>,
        pub day_ends: Choices<NaiveTime>,
        /// One button per day of `WEEKDAYS`.
        pub workday_buttons: RefCell<Vec<gtk::ToggleButton>>,
        /// The template as chosen, relative to the vault.
        pub template: RefCell<Option<PathBuf>>,
        #[template_child]
        pub name_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub location_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub template_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub clear_template_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub choose_template_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub first_day_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub workdays_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub target_hours_row: TemplateChild<adw::SpinRow>,
        #[template_child]
        pub slot_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub day_start_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub day_end_row: TemplateChild<adw::ComboRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PreferencesDialog {
        const NAME: &'static str = "BitLogPreferencesDialog";
        type Type = super::PreferencesDialog;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PreferencesDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            let buttons: Vec<gtk::ToggleButton> = WEEKDAYS
                .into_iter()
                .map(|day| {
                    let button = gtk::ToggleButton::builder()
                        .label(weekday_label(day, "%a"))
                        .tooltip_text(weekday_name(day))
                        .build();
                    button.connect_toggled(glib::clone!(
                        #[weak]
                        dialog,
                        move |button| dialog.workday_toggled(button)
                    ));
                    self.workdays_box.append(&button);
                    button
                })
                .collect();
            self.workday_buttons.replace(buttons);
            self.name_row
                .connect_changed(|row| set_class(row, "error", row.text().trim().is_empty()));
            self.name_row.connect_apply(glib::clone!(
                #[weak]
                dialog,
                move |row| {
                    let name = row.text().trim().to_owned();
                    if !name.is_empty() {
                        dialog.change(move |config| config.name = name);
                    }
                }
            ));
            self.location_row.connect_selected_notify(glib::clone!(
                #[weak]
                dialog,
                move |row| {
                    if let Some(location) = dialog.imp().locations.chosen(row) {
                        dialog.change(move |config| config.defaults.location = location);
                    }
                }
            ));
            self.choose_template_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    glib::spawn_future_local(async move { dialog.choose_template().await });
                }
            ));
            self.clear_template_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.set_template(None)
            ));
            self.first_day_row.connect_selected_notify(glib::clone!(
                #[weak]
                dialog,
                move |row| {
                    if let Some(day) = dialog.imp().first_days.chosen(row) {
                        dialog.change(move |config| config.week.first_day = day);
                    }
                }
            ));
            self.target_hours_row.connect_value_notify(glib::clone!(
                #[weak]
                dialog,
                move |row| {
                    let hours = row.value();
                    dialog.change(move |config| config.week.target_hours = hours);
                }
            ));
            self.slot_row.connect_selected_notify(glib::clone!(
                #[weak]
                dialog,
                move |row| {
                    if let Some(minutes) = dialog.imp().slots.chosen(row) {
                        dialog.change(move |config| config.grid.slot_minutes = minutes);
                    }
                }
            ));
            for row in [&*self.day_start_row, &*self.day_end_row] {
                row.connect_selected_notify(glib::clone!(
                    #[weak]
                    dialog,
                    move |_| dialog.day_range_chosen()
                ));
            }
        }
    }

    impl WidgetImpl for PreferencesDialog {}
    impl AdwDialogImpl for PreferencesDialog {}
    impl PreferencesDialogImpl for PreferencesDialog {}
}

glib::wrapper! {
    /// Changes the settings of a vault, kept in its `bitlog.toml`, each as
    /// soon as it is chosen.
    pub struct PreferencesDialog(ObjectSubclass<imp::PreferencesDialog>)
        @extends adw::PreferencesDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

/// The name of `weekday` in the user's language.
fn weekday_name(weekday: Weekday) -> String {
    weekday_label(weekday, "%A")
}

/// `weekday` in the user's language, written as the strftime `format` says.
fn weekday_label(weekday: Weekday, format: &str) -> String {
    // September 21, 2026 is a Monday.
    let monday = NaiveDate::from_ymd_opt(2026, 9, 21).expect("valid date");
    let date = monday + TimeDelta::days(weekday.num_days_from_monday().into());
    format_date(date, format)
}

/// The times offered for the day view: every half hour, and `set`.
fn day_times(set: NaiveTime) -> Vec<NaiveTime> {
    let mut times: Vec<NaiveTime> = (0..24 * 60)
        .step_by(TIME_STEP_MINUTES as usize)
        .map(time_at_minute)
        .collect();
    if let Err(index) = times.binary_search(&set) {
        times.insert(index, set);
    }
    times
}

impl PreferencesDialog {
    /// A dialog showing the settings of `vault`.
    pub fn new(vault: &Vault) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        let config = vault.config();
        imp.root.replace(vault.root().to_owned());

        imp.name_row.set_text(&config.name);
        let locations = [None]
            .into_iter()
            .chain(config.locations.keys().cloned().map(Some))
            .collect();
        imp.locations.fill(
            &*imp.location_row,
            locations,
            &config.defaults.location,
            |key| match key {
                Some(key) => config.location_name(key).to_owned(),
                None => gettext("None"),
            },
        );
        dialog.set_template(config.defaults.note_template.clone());

        imp.first_days.fill(
            &*imp.first_day_row,
            WEEKDAYS.to_vec(),
            &config.week.first_day,
            |day| weekday_name(*day),
        );
        for (day, button) in WEEKDAYS.iter().zip(imp.workday_buttons.borrow().iter()) {
            button.set_active(config.week.workdays.contains(day));
        }
        imp.target_hours_row.set_value(config.week.target_hours);

        let mut slots = SLOT_MINUTES.to_vec();
        if !slots.contains(&config.grid.slot_minutes) {
            slots.push(config.grid.slot_minutes);
            slots.sort_unstable();
        }
        imp.slots.fill(
            &*imp.slot_row,
            slots,
            &config.grid.slot_minutes,
            |minutes| format_duration(TimeDelta::minutes((*minutes).into())),
        );
        dialog.show_day_range(config.grid.day_start, config.grid.day_end);
        dialog
    }

    /// Saves a setting as soon as the user chose it. Not before the dialog
    /// is shown, while its rows are filled.
    fn change(&self, change: impl FnOnce(&mut VaultConfig)) {
        let Some(window) = self.root().and_downcast::<Window>() else {
            return;
        };
        if let Err(err) = window.change_config(change) {
            show_error(self, &gettext("Cannot Save Preferences"), &err.to_string());
        }
    }

    /// Saves the workdays chosen. The last one stays, a week has one.
    fn workday_toggled(&self, button: &gtk::ToggleButton) {
        let workdays: Vec<Weekday> = WEEKDAYS
            .into_iter()
            .zip(self.imp().workday_buttons.borrow().iter())
            .filter(|(_, button)| button.is_active())
            .map(|(day, _)| day)
            .collect();
        if workdays.is_empty() {
            button.set_active(true);
        } else {
            self.change(move |config| config.week.workdays = workdays);
        }
    }

    /// Offers the times before `end` for the start of the day view and
    /// those after `start` for its end, so that it cannot end before it
    /// starts.
    fn show_day_range(&self, start: NaiveTime, end: NaiveTime) {
        let imp = self.imp();
        let starts = day_times(start).into_iter().filter(|time| *time < end);
        let ends = day_times(end).into_iter().filter(|time| *time > start);
        let name = |time: &NaiveTime| format_time(*time);
        imp.day_starts
            .fill(&*imp.day_start_row, starts.collect(), &start, name);
        imp.day_ends
            .fill(&*imp.day_end_row, ends.collect(), &end, name);
    }

    /// Saves the start and end of the day view as chosen, both, as they
    /// only hold together.
    fn day_range_chosen(&self) {
        let imp = self.imp();
        let start = imp.day_starts.chosen(&*imp.day_start_row);
        let end = imp.day_ends.chosen(&*imp.day_end_row);
        let (Some(start), Some(end)) = (start, end) else {
            return;
        };
        self.change(move |config| {
            config.grid.day_start = start;
            config.grid.day_end = end;
        });
        self.show_day_range(start, end);
    }

    /// Shows `template` as the one for new notes, or none, and saves it.
    fn set_template(&self, template: Option<PathBuf>) {
        let imp = self.imp();
        let shown = template
            .as_deref()
            .map_or_else(|| gettext("None"), |path| path.display().to_string());
        imp.template_row.set_subtitle(&shown);
        imp.clear_template_button.set_visible(template.is_some());
        imp.template.replace(template.clone());
        self.change(move |config| config.defaults.note_template = template);
    }

    async fn choose_template(&self) {
        let root = self.imp().root.borrow().clone();
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose Note Template"))
            .modal(true)
            .build();
        let folder = match self.imp().template.borrow().as_deref() {
            Some(template) => root.join(template).parent().map(ToOwned::to_owned),
            None => Some(root.clone()),
        };
        if let Some(folder) = folder {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        let window = self.root().and_downcast::<gtk::Window>();
        // Dismissing the dialog is reported as an error, too.
        let Ok(file) = dialog.open_future(window.as_ref()).await else {
            return;
        };
        // The vault may have been opened through a symbolic link.
        let root = root.canonicalize().unwrap_or(root);
        let relative = file
            .path()
            .and_then(|path| path.canonicalize().ok())
            .and_then(|path| path.strip_prefix(&root).ok().map(ToOwned::to_owned));
        match relative {
            Some(relative) => self.set_template(Some(relative)),
            None => show_error(
                self,
                &gettext("Cannot Use File"),
                &gettext("The template has to lie in the vault folder."),
            ),
        }
    }
}
