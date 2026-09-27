use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{NaiveDate, NaiveTime, TimeDelta, Weekday};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gio, glib};
use knotbook_core::{EditError, LocationKey, Vault, VaultConfig};

use crate::format::{format_date, format_duration, format_time};

/// The block lengths offered, besides the one set.
const SLOT_MINUTES: [u32; 6] = [5, 10, 15, 20, 30, 60];

/// The steps of the times offered for the day view, besides those set.
const TIME_STEP_MINUTES: u32 = 30;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/preferences_dialog.ui")]
    pub struct PreferencesDialog {
        /// The vault folder, which note templates have to lie in.
        pub root: RefCell<PathBuf>,
        /// The settings as shown when the dialog opened.
        pub shown: RefCell<Option<VaultConfig>>,
        /// The choices of the combo rows, in the order shown.
        pub locations: RefCell<Vec<Option<LocationKey>>>,
        pub slots: RefCell<Vec<u32>>,
        pub day_starts: RefCell<Vec<NaiveTime>>,
        pub day_ends: RefCell<Vec<NaiveTime>>,
        /// The template as chosen, relative to the vault.
        pub template: RefCell<Option<PathBuf>>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub save_button: TemplateChild<gtk::Button>,
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
        const NAME: &'static str = "KnotbookPreferencesDialog";
        type Type = super::PreferencesDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PreferencesDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted when the user wants to save; the dialog stays open
            // until it is closed.
            SIGNALS.get_or_init(|| vec![Signal::builder("save").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            let days: Vec<String> = WEEKDAYS.into_iter().map(weekday_name).collect();
            let days: Vec<&str> = days.iter().map(String::as_str).collect();
            self.first_day_row
                .set_model(Some(&gtk::StringList::new(&days)));
            self.cancel_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| {
                    dialog.close();
                }
            ));
            self.save_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.emit_by_name::<()>("save", &[])
            ));
            self.name_row.connect_changed(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.update_save_button()
            ));
            for row in [&*self.day_start_row, &*self.day_end_row] {
                row.connect_selected_notify(glib::clone!(
                    #[weak]
                    dialog,
                    move |_| dialog.update_save_button()
                ));
            }
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
        }
    }

    impl WidgetImpl for PreferencesDialog {}
    impl AdwDialogImpl for PreferencesDialog {}
}

glib::wrapper! {
    /// Changes the settings of a vault, kept in its `knotbook.toml`.
    pub struct PreferencesDialog(ObjectSubclass<imp::PreferencesDialog>)
        @extends adw::Dialog, gtk::Widget,
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
    // September 21, 2026 is a Monday.
    let monday = NaiveDate::from_ymd_opt(2026, 9, 21).expect("valid date");
    let date = monday + TimeDelta::days(weekday.num_days_from_monday().into());
    format_date(date, "%A")
}

/// The times offered for the day view: every half hour, and `set`.
fn day_times(set: NaiveTime) -> Vec<NaiveTime> {
    let mut times: Vec<NaiveTime> = (0..24 * 60)
        .step_by(TIME_STEP_MINUTES as usize)
        .map(|minute| {
            NaiveTime::from_hms_opt(minute / 60, minute % 60, 0).expect("minutes of a day")
        })
        .collect();
    if let Err(index) = times.binary_search(&set) {
        times.insert(index, set);
    }
    times
}

/// Fills `row` with `choices` shown by `name` and selects `selected`.
fn set_choices<T: PartialEq>(
    row: &adw::ComboRow,
    choices: &[T],
    name: impl Fn(&T) -> String,
    selected: &T,
) {
    let names: Vec<String> = choices.iter().map(name).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    row.set_model(Some(&gtk::StringList::new(&names)));
    let index = choices
        .iter()
        .position(|choice| choice == selected)
        .expect("the value set is among the choices");
    row.set_selected(u32::try_from(index).expect("few choices"));
}

impl PreferencesDialog {
    /// A dialog showing the settings of `vault`.
    pub fn new(vault: &Vault) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        let config = vault.config();
        imp.root.replace(vault.root().to_owned());
        imp.shown.replace(Some(config.clone()));

        imp.name_row.set_text(&config.name);
        let locations: Vec<Option<LocationKey>> = [None]
            .into_iter()
            .chain(config.locations.keys().cloned().map(Some))
            .collect();
        set_choices(
            &imp.location_row,
            &locations,
            |key| match key {
                Some(key) => config.location_name(key).to_owned(),
                None => gettext("None"),
            },
            &config.defaults.location,
        );
        imp.locations.replace(locations);
        dialog.set_template(config.defaults.note_template.clone());

        set_choices(
            &imp.first_day_row,
            &WEEKDAYS,
            |day| weekday_name(*day),
            &config.week.first_day,
        );
        imp.target_hours_row.set_value(config.week.target_hours);

        let mut slots = SLOT_MINUTES.to_vec();
        if !slots.contains(&config.grid.slot_minutes) {
            slots.push(config.grid.slot_minutes);
            slots.sort_unstable();
        }
        set_choices(
            &imp.slot_row,
            &slots,
            |minutes| format_duration(TimeDelta::minutes((*minutes).into())),
            &config.grid.slot_minutes,
        );
        imp.slots.replace(slots);

        let starts = day_times(config.grid.day_start);
        let ends = day_times(config.grid.day_end);
        set_choices(
            &imp.day_start_row,
            &starts,
            |time| format_time(*time),
            &config.grid.day_start,
        );
        set_choices(
            &imp.day_end_row,
            &ends,
            |time| format_time(*time),
            &config.grid.day_end,
        );
        imp.day_starts.replace(starts);
        imp.day_ends.replace(ends);

        dialog.update_save_button();
        dialog
    }

    pub fn connect_save(&self, callback: impl Fn(&Self) + 'static) {
        self.connect_closure(
            "save",
            false,
            glib::closure_local!(move |dialog: &Self| callback(dialog)),
        );
    }

    /// Sets the settings of `config` that were changed in the dialog, so
    /// that changes made elsewhere meanwhile are kept. Blocks already there
    /// keep their times, whatever the block length.
    pub fn apply(&self, config: &mut VaultConfig) -> Result<(), EditError> {
        fn changed<T: PartialEq>(field: &mut T, shown: &T, entered: T) {
            if entered != *shown {
                *field = entered;
            }
        }

        let imp = self.imp();
        let shown = imp.shown.borrow();
        let shown = shown.as_ref().expect("the dialog shows settings");
        changed(&mut config.name, &shown.name, self.name());
        changed(
            &mut config.defaults.location,
            &shown.defaults.location,
            imp.locations.borrow()[imp.location_row.selected() as usize].clone(),
        );
        changed(
            &mut config.defaults.note_template,
            &shown.defaults.note_template,
            imp.template.borrow().clone(),
        );
        changed(
            &mut config.week.first_day,
            &shown.week.first_day,
            WEEKDAYS[imp.first_day_row.selected() as usize],
        );
        changed(
            &mut config.week.target_hours,
            &shown.week.target_hours,
            imp.target_hours_row.value(),
        );
        changed(
            &mut config.grid.slot_minutes,
            &shown.grid.slot_minutes,
            imp.slots.borrow()[imp.slot_row.selected() as usize],
        );
        let (start, end) = self.day_range();
        changed(&mut config.grid.day_start, &shown.grid.day_start, start);
        changed(&mut config.grid.day_end, &shown.grid.day_end, end);
        Ok(())
    }

    fn name(&self) -> String {
        self.imp().name_row.text().trim().to_owned()
    }

    /// The start and end of the day view as chosen.
    fn day_range(&self) -> (NaiveTime, NaiveTime) {
        let imp = self.imp();
        (
            imp.day_starts.borrow()[imp.day_start_row.selected() as usize],
            imp.day_ends.borrow()[imp.day_end_row.selected() as usize],
        )
    }

    fn set_template(&self, template: Option<PathBuf>) {
        let imp = self.imp();
        let shown = template
            .as_deref()
            .map_or_else(|| gettext("None"), |path| path.display().to_string());
        imp.template_row.set_subtitle(&shown);
        imp.clear_template_button.set_visible(template.is_some());
        imp.template.replace(template);
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
            None => {
                let alert = adw::AlertDialog::new(
                    Some(&gettext("Cannot Use File")),
                    Some(&gettext("The template has to lie in the vault folder.")),
                );
                alert.add_response("close", &gettext("_Close"));
                alert.present(Some(self));
            }
        }
    }

    /// Allows saving once there is a name and the day starts before it ends.
    fn update_save_button(&self) {
        let imp = self.imp();
        // Called while the rows are filled, before the choices are known.
        if imp.day_ends.borrow().is_empty() {
            return;
        }
        let (start, end) = self.day_range();
        let range_valid = start < end;
        if range_valid {
            imp.day_end_row.remove_css_class("error");
        } else {
            imp.day_end_row.add_css_class("error");
        }
        imp.save_button
            .set_sensitive(!self.name().is_empty() && range_valid);
    }
}
