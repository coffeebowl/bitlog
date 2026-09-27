use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};
use knotbook_core::{Commit, NotePath, Period, Project, ProjectSlug, Vault, git_log};
use knotbook_index::{Found, ProjectBlock};

use crate::format::{format_duration, format_full_date, format_share, format_time};
use crate::heatmap::Heatmap;
use crate::markdown_view::MarkdownView;
use crate::search_index::{ProjectData, SearchIndex};
use crate::window::show_action;

/// Blocks the timeline shows at first and adds with "Load More".
const BLOCKS_AT_ONCE: u32 = 50;

/// Commits the log shows at first and adds with "Load More".
const COMMITS_AT_ONCE: usize = 50;

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
        /// The project's repository on this device, if it has one.
        pub repo: RefCell<Option<PathBuf>>,
        /// The time spent on the project on each day, to mark the days of
        /// commits.
        pub activity: RefCell<BTreeMap<NaiveDate, TimeDelta>>,
        /// How many commits the log shows.
        pub commits_shown: Cell<usize>,
        /// The last day shown in the log, which commits loaded later join.
        pub last_day: RefCell<Option<(NaiveDate, adw::PreferencesGroup)>>,
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
        #[template_child]
        pub git_page: TemplateChild<adw::ViewStackPage>,
        #[template_child]
        pub commits_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub commits_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub more_commits_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub commits_error_page: TemplateChild<adw::StatusPage>,
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
            self.more_commits_button.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.show_more_commits()
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
    /// spent on it, its blocks and its commits in the background.
    pub fn show(&self, vault: &Vault, project: &Project) {
        let imp = self.imp();
        let repo = vault
            .repo_paths()
            .map(|mut repos| repos.remove(&project.slug))
            .unwrap_or_else(|err| {
                glib::g_warning!("knotbook", "{err}");
                None
            });
        let git = imp.view_stack.visible_child_name().as_deref() == Some("git");
        if imp.slug.borrow().as_ref() != Some(&project.slug) || (git && repo.is_none()) {
            imp.view_stack.set_visible_child_name("overview");
        }
        // Projects without code have no repository, and then no Git page.
        imp.git_page.set_visible(repo.is_some());
        imp.repo.replace(repo);
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
        let month = Period::Month.range(today, vault.config().week.first_day);
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
                // After the activity, which marks the days of commits.
                view.clear_commits();
                view.show_more_commits();
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
        let first_day = vault.config().week.first_day;
        let (week_first, week_last) = Period::Week.range(today, first_day);
        let (month_first, month_last) = Period::Month.range(today, first_day);
        let month = sum_between(month_first, month_last);
        let work: TimeDelta = data
            .month_times
            .iter()
            .filter(|(slug, _)| !vault.is_break(slug))
            .map(|(_, time)| *time)
            .sum();
        let days = i32::try_from(activity.len()).unwrap_or(i32::MAX);
        let none = || "–".to_owned();

        imp.total_row.set_subtitle(&format_duration(total));
        imp.week_row
            .set_subtitle(&format_duration(sum_between(week_first, week_last)));
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
                let (action, target) = block_action(block);
                imp.longest_row.set_action_target_value(Some(&target));
                imp.longest_row.set_action_name(Some(action));
                imp.longest_row.set_activatable(true);
            }
            None => {
                imp.longest_row.set_subtitle(&none());
                imp.longest_row.set_action_name(None);
                imp.longest_row.set_activatable(false);
            }
        }
        imp.activity.replace(activity.iter().copied().collect());
        let color = gdk::RGBA::parse(project.color.as_str()).expect("project colors are valid");
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

impl ProjectView {
    fn clear_commits(&self) {
        let imp = self.imp();
        while let Some(child) = imp.commits_box.first_child() {
            imp.commits_box.remove(&child);
        }
        imp.commits_shown.set(0);
        imp.last_day.replace(None);
        imp.more_commits_button.set_visible(false);
    }

    /// Reads the next commits from the repository in the background and
    /// adds them to the log.
    fn show_more_commits(&self) {
        let imp = self.imp();
        let Some(repo) = imp.repo.borrow().clone() else {
            return;
        };
        let lookup = imp.lookups.get();
        let skip = imp.commits_shown.get();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let commits = gio::spawn_blocking(move || git_log(&repo, skip, COMMITS_AT_ONCE))
                    .await
                    .expect("reading the log does not panic");
                let imp = view.imp();
                if imp.lookups.get() != lookup {
                    return;
                }
                match commits {
                    Ok(commits) => view.add_commits(&commits),
                    Err(err) => {
                        imp.commits_error_page
                            .set_description(Some(&err.to_string()));
                        imp.commits_stack.set_visible_child_name("error");
                    }
                }
            }
        ));
    }

    /// Adds `commits`, newest first, to the end of the log, grouped by the
    /// local day they were made on.
    fn add_commits(&self, commits: &[Commit]) {
        let imp = self.imp();
        let today = Local::now().date_naive();
        for commit in commits {
            let date = commit.time.with_timezone(&Local).date_naive();
            let group = match &*imp.last_day.borrow() {
                Some((last, group)) if *last == date => group.clone(),
                _ => {
                    let group = self.day_group(date, today);
                    imp.commits_box.append(&group);
                    group
                }
            };
            group.add(&commit_row(commit, date));
            imp.last_day.replace(Some((date, group)));
        }
        let shown = imp.commits_shown.get() + commits.len();
        imp.commits_shown.set(shown);
        // A full batch may have more behind it.
        imp.more_commits_button
            .set_visible(commits.len() == COMMITS_AT_ONCE);
        imp.commits_stack
            .set_visible_child_name(if shown == 0 { "empty" } else { "list" });
    }

    /// The group of the commits made on `date`. It tells the time spent
    /// on the project that day, if any.
    fn day_group(&self, date: NaiveDate, today: NaiveDate) -> adw::PreferencesGroup {
        let title = if date == today {
            gettext("Today")
        } else if today.pred_opt() == Some(date) {
            gettext("Yesterday")
        } else {
            format_full_date(date)
        };
        let group = adw::PreferencesGroup::builder().title(title).build();
        if let Some(time) = self.imp().activity.borrow().get(&date) {
            // Translators: The time spent on a project on a day, as in
            // "2 h 30 min in blocks".
            let description =
                gettext("{time} in blocks").replace("{time}", &format_duration(*time));
            group.set_description(Some(&description));
        }
        group
    }
}

/// A commit in the log: its summary, then hash, author and time, opening
/// its day when activated.
fn commit_row(commit: &Commit, date: NaiveDate) -> adw::ActionRow {
    let time = commit.time.with_timezone(&Local).time();
    adw::ActionRow::builder()
        .title(&commit.summary)
        .subtitle(format!(
            "{} · {} · {}",
            commit.short_id(),
            commit.author,
            format_time(time)
        ))
        .use_markup(false)
        .activatable(true)
        .action_target(&date.to_string().to_variant())
        .action_name("win.show-day")
        .build()
}

/// The window action that shows `block`, with its target.
fn block_action(block: &ProjectBlock) -> (&'static str, glib::Variant) {
    show_action(&Found::Block {
        date: block.date,
        id: block.id.clone(),
    })
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
    let (action, target) = block_action(block);
    gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(true)
        .action_target(&target)
        .action_name(action)
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
