use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{
    Block, BlockId, ConflictCopy, ConflictVersions, Contradiction, Day, ReadError, Task, TaskId,
    TaskList, TaskStatus, Vault, VaultChange,
};
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::glib;

use crate::alert::show_error;
use crate::conflict_dialog::ConflictDialog;
use crate::format::{format_full_date, format_span, kind_name};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/sync_conflict_dialog.ui")]
    pub struct SyncConflictDialog {
        pub vault: RefCell<Option<Rc<Vault>>>,
        pub copy: RefCell<Option<ConflictCopy>>,
        /// Each contradiction with the toggles that decide it.
        pub choices: RefCell<Vec<(Contradiction, adw::ToggleGroup)>>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub rows: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub merge_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SyncConflictDialog {
        const NAME: &'static str = "BitLogSyncConflictDialog";
        type Type = super::SyncConflictDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SyncConflictDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted once the copy is merged and the dialog closed.
            SIGNALS.get_or_init(|| vec![Signal::builder("merged").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            self.merge_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.merge()
            ));
        }
    }

    impl WidgetImpl for SyncConflictDialog {}
    impl AdwDialogImpl for SyncConflictDialog {}
}

glib::wrapper! {
    /// Lets the user decide what a sync conflict copy and its original
    /// contradict each other in, then merges them.
    pub struct SyncConflictDialog(ObjectSubclass<imp::SyncConflictDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// How a contradiction is shown.
struct Row {
    title: String,
    subtitle: String,
    /// The texts of both sides, to compare them.
    texts: Option<(String, String)>,
}

impl SyncConflictDialog {
    pub fn new(vault: Rc<Vault>, copy: ConflictCopy) -> Result<Self, ReadError> {
        let contradictions = vault.contradictions(&copy)?;
        let versions = vault.conflict_versions(&copy)?;
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        imp.window_title.set_subtitle(&file_title(&vault, &copy.of));
        let mut choices = Vec::new();
        for contradiction in contradictions {
            let row = describe(&vault, &versions, &contradiction);
            choices.push((contradiction, dialog.add_row(row)));
        }
        imp.choices.replace(choices);
        imp.vault.replace(Some(vault));
        imp.copy.replace(Some(copy));
        Ok(dialog)
    }

    pub fn connect_merged(&self, callback: impl Fn() + 'static) {
        self.connect_closure(
            "merged",
            false,
            glib::closure_local!(move |_: &Self| callback()),
        );
    }

    /// Adds a row for `row` and returns the toggles that decide it.
    fn add_row(&self, row: Row) -> adw::ToggleGroup {
        let action_row = adw::ActionRow::builder()
            .title(&row.title)
            .subtitle(&row.subtitle)
            .use_markup(false)
            .build();
        let toggles = adw::ToggleGroup::builder()
            .valign(gtk::Align::Center)
            .build();
        for (name, label) in [("original", gettext("Original")), ("copy", gettext("Copy"))] {
            toggles.add(adw::Toggle::builder().name(name).label(label).build());
        }
        toggles.set_active_name(Some("original"));
        if let Some((original, copy)) = row.texts {
            let compare = gtk::Button::builder()
                .label(gettext("Compare"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            compare.connect_clicked(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                #[weak]
                toggles,
                move |_| {
                    let compare = ConflictDialog::compare(&row.title, &original, &copy);
                    compare.connect_chosen(move |keep_original| {
                        toggles.set_active_name(Some(if keep_original {
                            "original"
                        } else {
                            "copy"
                        }));
                    });
                    compare.present(Some(&dialog));
                }
            ));
            action_row.add_suffix(&compare);
        }
        action_row.add_suffix(&toggles);
        self.imp().rows.add(&action_row);
        toggles
    }

    fn merge(&self) {
        let imp = self.imp();
        let theirs: Vec<Contradiction> = imp
            .choices
            .borrow()
            .iter()
            .filter(|(_, toggles)| toggles.active_name().as_deref() == Some("copy"))
            .map(|(contradiction, _)| contradiction.clone())
            .collect();
        let vault = imp.vault.borrow().clone().expect("set up with a vault");
        let copy = imp.copy.borrow().clone().expect("set up with a copy");
        match vault.merge_conflict(&copy, &theirs) {
            Ok(()) => {
                self.close();
                self.emit_by_name::<()>("merged", &[]);
            }
            Err(err) => {
                show_error(self, &gettext("Cannot Merge"), &err.to_string());
            }
        }
    }
}

/// A name for the file of `of`, as in "September 22, 2026".
pub fn file_title(vault: &Vault, of: &VaultChange) -> String {
    match of {
        VaultChange::Day(date) => format_full_date(*date),
        VaultChange::Tasks => gettext("Tasks"),
        VaultChange::Note(note) => {
            format!("{} / {}", vault.project_name(note.project()), note.name())
        }
        VaultChange::Project(slug) => vault.project_name(slug).to_owned(),
        VaultChange::Config => gettext("Vault Settings"),
    }
}

fn describe(vault: &Vault, versions: &ConflictVersions, contradiction: &Contradiction) -> Row {
    let both = |original: String, copy: String| {
        gettext("Original: {original}\nCopy: {copy}")
            .replace("{original}", &original)
            .replace("{copy}", &copy)
    };
    match (versions, contradiction) {
        (ConflictVersions::Days(ours, theirs), Contradiction::DayField(name)) => Row {
            title: field_title(name),
            subtitle: both(
                field_value(vault, ours, name),
                field_value(vault, theirs, name),
            ),
            texts: None,
        },
        (ConflictVersions::Days(ours, theirs), Contradiction::Block(id)) => Row {
            title: gettext("Block"),
            subtitle: both(
                block_summary(vault, block(ours, id)),
                block_summary(vault, block(theirs, id)),
            ),
            texts: None,
        },
        (ConflictVersions::Days(ours, theirs), Contradiction::BlockText(id)) => Row {
            title: gettext("Text of Block “{name}”")
                .replace("{name}", &block_name(vault, block(ours, id))),
            subtitle: gettext("Compare the two texts to choose one"),
            texts: Some((block(ours, id).text.clone(), block(theirs, id).text.clone())),
        },
        (ConflictVersions::Days(_, theirs), Contradiction::Overlap(id)) => Row {
            title: gettext("Block Only in the Copy"),
            subtitle: format!(
                "{}\n{}",
                block_summary(vault, block(theirs, id)),
                gettext("Keeping it removes the blocks of the original it overlaps")
            ),
            texts: None,
        },
        (ConflictVersions::Days(ours, theirs), Contradiction::DayNote) => Row {
            title: gettext("Day Note"),
            subtitle: gettext("Compare the two texts to choose one"),
            texts: Some((ours.note.clone(), theirs.note.clone())),
        },
        (ConflictVersions::Tasks(ours, theirs), Contradiction::TaskField(id, name)) => {
            let (ours, theirs) = (task(ours, id), task(theirs, id));
            let value = |task: &Task| match name.as_str() {
                "title" => Some(task.title.clone()),
                "status" => Some(status_name(task.status)),
                "created" => Some(date_value(task.created)),
                "due" => Some(date_value(task.due)),
                "done" => Some(date_value(task.done)),
                _ => None,
            };
            let field = match name.as_str() {
                "title" => gettext("Title"),
                "status" => gettext("Status"),
                "created" => gettext("Created"),
                "due" => gettext("Due Date"),
                "done" => gettext("Finished"),
                _ => name.clone(),
            };
            let subtitle = match (value(ours), value(theirs)) {
                (Some(original), Some(copy)) => format!("{field}\n{}", both(original, copy)),
                _ => field,
            };
            Row {
                title: gettext("Task “{title}”").replace("{title}", &ours.title),
                subtitle,
                texts: None,
            }
        }
        (ConflictVersions::Texts(ours, theirs), Contradiction::Content) => Row {
            title: gettext("Whole File"),
            subtitle: gettext("These files are not merged, keep one of them"),
            texts: Some((ours.clone(), theirs.clone())),
        },
        _ => unreachable!("contradictions match the kind of file"),
    }
}

fn task<'a>(tasks: &'a TaskList, id: &TaskId) -> &'a Task {
    tasks
        .task(id)
        .expect("the contradiction names a task of this side")
}

fn block<'a>(day: &'a Day, id: &BlockId) -> &'a Block {
    day.blocks
        .iter()
        .find(|block| block.id == *id)
        .expect("the contradiction names a block of this side")
}

/// The title of `block`, or else the name of its project.
fn block_name(vault: &Vault, block: &Block) -> String {
    if block.title.is_empty() {
        vault.project_name(&block.project).to_owned()
    } else {
        block.title.clone()
    }
}

/// As in "09:00–10:30 · Webshop · Checkout".
fn block_summary(vault: &Vault, block: &Block) -> String {
    let mut parts = vec![
        format_span(block.start, block.end),
        vault.project_name(&block.project).to_owned(),
    ];
    if !block.title.is_empty() {
        parts.push(block.title.clone());
    }
    parts.join(" · ")
}

/// The name of the front matter field `name`.
fn field_title(name: &str) -> String {
    match name {
        "kind" => gettext("Kind of Day"),
        "location" => gettext("Location"),
        "tags" => gettext("Tags"),
        "energy" => gettext("Energy"),
        "work" => gettext("Working Hours"),
        _ => name.to_owned(),
    }
}

/// The front matter field `name` of `day`, as shown.
fn field_value(vault: &Vault, day: &Day, name: &str) -> String {
    let none = || gettext("None");
    match name {
        "kind" => kind_name(&day.kind),
        "location" => day
            .location
            .as_ref()
            .map_or_else(none, |key| vault.config().location_name(key).to_owned()),
        "tags" if day.tags.is_empty() => none(),
        "tags" => day.tags.join(", "),
        "energy" => day.energy.map_or_else(none, |energy| energy.to_string()),
        "work" => match (day.work_start, day.work_end) {
            (Some(start), Some(end)) => format_span(start, end),
            _ => none(),
        },
        _ => day
            .unknown_fields
            .get(name)
            .map_or_else(none, |value| value.to_string()),
    }
}

fn status_name(status: TaskStatus) -> String {
    match status {
        TaskStatus::Open => gettext("Open"),
        TaskStatus::Done => gettext("Done"),
        TaskStatus::Dropped => gettext("Dropped"),
    }
}

fn date_value(date: Option<chrono::NaiveDate>) -> String {
    date.map_or_else(|| gettext("None"), format_full_date)
}
