use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Datelike, Days, Local, Months, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};
use knotbook_core::{NotePath, Project, ProjectSlug, Vault};
use knotbook_index::ProjectBlock;

use crate::calendar_view::week_start;
use crate::format::{format_duration, format_full_date, format_share};
use crate::heatmap::Heatmap;
use crate::markdown_view::MarkdownView;
use crate::search_index::{ProjectData, SearchIndex};

/// Blocks the timeline shows at first and adds with "Load More".
const BLOCKS_AT_ONCE: u32 = 50;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/project_view.ui")]
    pub struct ProjectView {
        /// The project shown.
        pub slug: RefCell<Option<ProjectSlug>>,
        pub rows: RefCell<Vec<adw::ActionRow>>,
        pub vault: RefCell<Option<Vault>>,
        pub index: RefCell<SearchIndex>,
        /// Counts the lookups in the index, so that one finishing after a
        /// newer one is dropped.
        pub lookups: Cell<u32>,
        /// How many blocks the timeline shows.
        pub blocks_shown: Cell<u32>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub new_note_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub edit_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub notes_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub empty_new_note_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub view_stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub total_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub week_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub month_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub share_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub days_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub average_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub first_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub last_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub longest_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub heatmap: TemplateChild<Heatmap>,
        #[template_child]
        pub timeline_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub timeline_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub more_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectView {
        const NAME: &'static str = "KnotbookProjectView";
        type Type = super::ProjectView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Heatmap::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ProjectView {
        fn constructed(&self) {
            self.parent_constructed();
            self.more_button.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.show_more_blocks()
            ));
        }
    }
    impl WidgetImpl for ProjectView {}
    impl NavigationPageImpl for ProjectView {}
}

glib::wrapper! {
    /// One project with its notes.
    pub struct ProjectView(ObjectSubclass<imp::ProjectView>)
        @extends adw::NavigationPage, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ProjectView {
    /// The project shown, if any.
    pub fn slug(&self) -> Option<ProjectSlug> {
        self.imp().slug.borrow().clone()
    }

    /// Uses `index` for the time spent on projects and their blocks.
    pub fn set_index(&self, index: SearchIndex) {
        self.imp().index.replace(index);
    }

    /// Shows `project` of `vault` with its notes, and looks up the time
    /// spent on it and its blocks in the background.
    pub fn show(&self, vault: &Vault, project: &Project) {
        let imp = self.imp();
        if imp.slug.borrow().as_ref() != Some(&project.slug) {
            imp.view_stack.set_visible_child_name("overview");
        }
        imp.slug.replace(Some(project.slug.clone()));
        imp.vault.replace(Some(vault.clone()));
        self.look_up(vault, project);
        self.set_title(&project.name);
        imp.window_title.set_title(&project.name);
        imp.window_title.set_subtitle(&project.category);
        let target = project.slug.to_string().to_variant();
        for button in [
            &*imp.new_note_button,
            &*imp.edit_button,
            &*imp.empty_new_note_button,
        ] {
            button.set_action_target_value(Some(&target));
        }

        for row in imp.rows.take() {
            imp.notes_group.remove(&row);
        }
        let notes = match vault.notes(&project.slug) {
            Ok(notes) => notes,
            Err(err) => {
                imp.error_page.set_description(Some(&err.to_string()));
                imp.stack.set_visible_child_name("error");
                return;
            }
        };
        let rows: Vec<adw::ActionRow> = notes.iter().map(note_row).collect();
        for row in &rows {
            imp.notes_group.add(row);
        }
        imp.stack
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        imp.rows.replace(rows);
    }
}

impl ProjectView {
    /// Looks up the time spent on `project` and its newest blocks, then
    /// shows them.
    fn look_up(&self, vault: &Vault, project: &Project) {
        let imp = self.imp();
        let lookup = imp.lookups.get() + 1;
        imp.lookups.set(lookup);
        let today = Local::now().date_naive();
        let month = month_of(today);
        let index = imp.index.borrow().clone();
        let (vault, project) = (vault.clone(), project.clone());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let data = index
                    .project(&vault, project.slug.clone(), month, BLOCKS_AT_ONCE)
                    .await;
                if view.imp().lookups.get() != lookup {
                    return;
                }
                match data {
                    Ok(data) => view.show_data(&vault, &project, data, today),
                    Err(err) => glib::g_warning!("knotbook", "{err}"),
                }
            }
        ));
    }

    fn show_data(&self, vault: &Vault, project: &Project, data: ProjectData, today: NaiveDate) {
        let imp = self.imp();
        let activity = &data.activity;
        let sum_between = |first: NaiveDate, last: NaiveDate| -> TimeDelta {
            activity
                .iter()
                .filter(|(date, _)| (first..=last).contains(date))
                .map(|(_, time)| *time)
                .sum()
        };
        let total: TimeDelta = activity.iter().map(|(_, time)| *time).sum();
        let week = week_start(today, vault.config().week.first_day);
        let (month_first, month_last) = month_of(today);
        let month = sum_between(month_first, month_last);
        let work: TimeDelta = data
            .month_times
            .iter()
            .filter(|(slug, _)| !vault.project(slug).is_some_and(Project::is_break))
            .map(|(_, time)| *time)
            .sum();
        let days = i32::try_from(activity.len()).unwrap_or(i32::MAX);
        let none = || "–".to_owned();

        imp.total_row.set_subtitle(&format_duration(total));
        imp.week_row
            .set_subtitle(&format_duration(sum_between(week, week + Days::new(6))));
        imp.month_row.set_subtitle(&format_duration(month));
        imp.share_row.set_subtitle(&if work.is_zero() {
            none()
        } else {
            format_share(month, work)
        });
        imp.days_row.set_subtitle(&days.to_string());
        imp.average_row.set_subtitle(&if days == 0 {
            none()
        } else {
            format_duration(total / days)
        });
        let date = |day: Option<&(NaiveDate, TimeDelta)>| {
            day.map_or_else(none, |(date, _)| format_full_date(*date))
        };
        imp.first_row.set_subtitle(&date(activity.first()));
        imp.last_row.set_subtitle(&date(activity.last()));
        match &data.longest {
            Some(block) => {
                imp.longest_row.set_subtitle(&format!(
                    "{} · {}",
                    format_duration(block.duration()),
                    format_full_date(block.date)
                ));
                // The target first, which the action needs.
                imp.longest_row
                    .set_action_target_value(Some(&block_target(block)));
                imp.longest_row.set_action_name(Some("win.show-block"));
                imp.longest_row.set_activatable(true);
            }
            None => {
                imp.longest_row.set_subtitle(&none());
                imp.longest_row.set_action_name(None);
                imp.longest_row.set_activatable(false);
            }
        }
        let color = gdk::RGBA::parse(project.color.as_str()).expect("project colors are valid");
        let first_day = vault.config().week.first_day;
        imp.heatmap.show(
            activity,
            color,
            Heatmap::last_12_months(today, first_day),
            first_day,
        );

        imp.timeline_list.remove_all();
        imp.blocks_shown.set(0);
        self.add_blocks(data.blocks);
    }

    /// Adds `blocks` to the end of the timeline.
    fn add_blocks(&self, blocks: Vec<ProjectBlock>) {
        let imp = self.imp();
        let added = u32::try_from(blocks.len()).expect("at most BLOCKS_AT_ONCE blocks come");
        for block in &blocks {
            imp.timeline_list.append(&block_row(block));
        }
        imp.blocks_shown.set(imp.blocks_shown.get() + added);
        // A full batch may have more behind it.
        imp.more_button.set_visible(added == BLOCKS_AT_ONCE);
        let empty = imp.blocks_shown.get() == 0;
        imp.timeline_stack
            .set_visible_child_name(if empty { "empty" } else { "list" });
    }

    fn show_more_blocks(&self) {
        let imp = self.imp();
        let (Some(slug), Some(vault)) = (self.slug(), imp.vault.borrow().clone()) else {
            return;
        };
        let lookup = imp.lookups.get();
        let skip = imp.blocks_shown.get();
        let index = imp.index.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let blocks = index
                    .project_blocks(&vault, slug, skip, BLOCKS_AT_ONCE)
                    .await;
                if view.imp().lookups.get() != lookup {
                    return;
                }
                match blocks {
                    Ok(blocks) => view.add_blocks(blocks),
                    Err(err) => glib::g_warning!("knotbook", "{err}"),
                }
            }
        ));
    }
}

/// The first and last day of the month `date` lies in.
fn month_of(date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = date.with_day(1).expect("every month has a first day");
    let last = (first + Months::new(1))
        .pred_opt()
        .expect("the day before exists");
    (first, last)
}

/// The target of `win.show-block` for `block`.
fn block_target(block: &ProjectBlock) -> glib::Variant {
    (block.date.to_string(), block.id.to_string()).to_variant()
}

/// A block in the timeline: its title, when it was and its text, opening
/// its day when activated.
fn block_row(block: &ProjectBlock) -> gtk::ListBoxRow {
    let title = if block.title.is_empty() {
        gettext("Untitled Block")
    } else {
        block.title.clone()
    };
    let minute = |minute: u32| format!("{:02}:{:02}", minute / 60 % 24, minute % 60);
    let when = format!(
        "{} · {}–{} · {}",
        format_full_date(block.date),
        minute(block.start_minute),
        minute(block.end_minute),
        format_duration(block.duration())
    );
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(
        &gtk::Label::builder()
            .label(&title)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["heading"])
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(&when)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    if !block.text.is_empty() {
        let text: MarkdownView = glib::Object::new();
        text.set_markdown(&block.text);
        // Clicks go to the row, which opens the block.
        text.set_can_target(false);
        text.set_margin_top(6);
        content.append(&text);
    }
    gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(true)
        .action_target(&block_target(block))
        .action_name("win.show-block")
        .build()
}

fn note_row(note: &NotePath) -> adw::ActionRow {
    let target = note.to_string().to_variant();
    let row = adw::ActionRow::builder()
        .title(note.name())
        .use_markup(false)
        .activatable(true)
        .action_name("notes.open")
        .action_target(&target)
        .build();
    let menu_button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(gettext("Note Menu"))
        .menu_model(&note_menu(note))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    row.add_suffix(&menu_button);
    row
}

/// Renaming and deleting the note `note`.
pub fn note_menu(note: &NotePath) -> gio::Menu {
    let target = note.to_string().to_variant();
    let menu = gio::Menu::new();
    for (label, action) in [
        (gettext("_Rename…"), "notes.rename"),
        (gettext("_Delete"), "notes.delete"),
    ] {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&target));
        menu.append_item(&item);
    }
    menu
}
