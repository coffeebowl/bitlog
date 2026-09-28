use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::{gettext, ngettext};
use gtk::{gdk, gio, glib};
use knotbook_core::{Commit, NotePath, Period, Project, ProjectSlug, Vault, git_log};
use knotbook_index::{Found, ProjectBlock};

use crate::format::{
    format_duration, format_full_date, format_share, format_time, format_weekday_date,
};
use crate::heatmap::Heatmap;
use crate::markdown_view::MarkdownView;
use crate::miniature::Miniature;
use crate::search_index::{ProjectData, SearchIndex};
use crate::window::show_action;

/// Blocks the timeline shows at first and adds with "Load More".
const BLOCKS_AT_ONCE: u32 = 50;

/// The days the activity shows, up to today.
const ACTIVITY_DAYS: u64 = 30;

/// Commits the log shows at first and adds with "Load More".
const COMMITS_AT_ONCE: usize = 50;

/// The lines of a note its miniature formats at most, more than fit.
const PREVIEW_LINES: usize = 50;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/project_view.ui")]
    pub struct ProjectView {
        /// The project shown.
        pub slug: RefCell<Option<ProjectSlug>>,
        /// The notes in the grid, in its order, with their previews.
        pub notes: RefCell<Vec<(NotePath, MarkdownView)>>,
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
        pub switcher: TemplateChild<adw::InlineViewSwitcher>,
        #[template_child]
        pub view_stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub scrolled: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub total_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub recent_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub span_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub longest_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub heatmap: TemplateChild<Heatmap>,
        #[template_child]
        pub notes_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub notes_grid: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub empty_new_note_button: TemplateChild<adw::ButtonRow>,
        #[template_child]
        pub notes_error_row: TemplateChild<adw::ActionRow>,
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
            self.notes_grid.connect_child_activated(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_, card| {
                    let index = usize::try_from(card.index()).expect("cards are in the grid");
                    let target = view.imp().notes.borrow()[index].0.to_string().to_variant();
                    let _ = WidgetExt::activate_action(&view, "notes.open", Some(&target));
                }
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
        let other = imp.slug.borrow().as_ref() != Some(&project.slug);
        if other || (git && repo.is_none()) {
            imp.view_stack.set_visible_child_name("project");
        }
        if other {
            imp.scrolled.vadjustment().set_value(0.0);
        }
        // Projects without code have no repository, and then no Git page
        // to switch to.
        imp.git_page.set_visible(repo.is_some());
        imp.switcher.set_visible(repo.is_some());
        imp.repo.replace(repo);
        imp.slug.replace(Some(project.slug.clone()));
        imp.vault.replace(Some(vault.clone()));
        self.look_up(vault, project);
        self.set_title(&project.name);
        imp.window_title.set_title(&project.name);
        imp.window_title.set_subtitle(&project.category);
        let target = project.slug.to_string().to_variant();
        imp.new_note_button.set_action_target_value(Some(&target));
        imp.edit_button.set_action_target_value(Some(&target));
        imp.empty_new_note_button
            .set_action_target_value(Some(&target));

        imp.notes_grid.remove_all();
        imp.notes.take();
        let notes = match vault.notes(&project.slug) {
            Ok(notes) => notes,
            Err(err) => {
                imp.notes_error_row.set_subtitle(&err.to_string());
                imp.notes_stack.set_visible_child_name("error");
                return;
            }
        };
        let notes: Vec<_> = notes
            .into_iter()
            .map(|note| {
                let preview = note_preview();
                show_preview(&preview, &note_text(vault, &note));
                imp.notes_grid.append(&note_card(&note, &preview));
                (note, preview)
            })
            .collect();
        imp.notes_stack
            .set_visible_child_name(if notes.is_empty() { "empty" } else { "list" });
        imp.notes.replace(notes);
    }

    /// Shows the notes in the grid as they are now, as after editing one.
    pub fn update_previews(&self) {
        let imp = self.imp();
        let Some(vault) = imp.vault.borrow().clone() else {
            return;
        };
        for (note, preview) in &*imp.notes.borrow() {
            show_preview(preview, &note_text(&vault, note));
        }
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

        imp.total_row.set_subtitle(&if days == 0 {
            none()
        } else {
            // Translators: The time spent on a project, the days it was
            // worked on and the average per day, as in
            // "42 h on 17 days · 2 h 28 min per day".
            ngettext(
                "{total} on {days} day · {average} per day",
                "{total} on {days} days · {average} per day",
                days.unsigned_abs(),
            )
            .replace("{total}", &format_duration(total))
            .replace("{days}", &days.to_string())
            .replace("{average}", &format_duration(total / days))
        });
        let share = if work.is_zero() {
            String::new()
        } else {
            // Translators: The share of a project in the time spent on
            // all projects this month, as in " (35 % of all work)".
            gettext(" ({share} of all work)").replace("{share}", &format_share(month, work))
        };
        // Translators: The time spent on a project this week and month, as
        // in "3 h this week · 12 h this month (35 % of all work)".
        imp.recent_row.set_subtitle(
            &gettext("{week} this week · {month} this month{share}")
                .replace(
                    "{week}",
                    &format_duration(sum_between(week_first, week_last)),
                )
                .replace("{month}", &format_duration(month))
                .replace("{share}", &share),
        );
        imp.span_row
            .set_subtitle(&match (activity.first(), activity.last()) {
                (Some((first, _)), Some((last, _))) if first != last => {
                    format!("{} – {}", format_full_date(*first), format_full_date(*last))
                }
                (Some((first, _)), _) => format_full_date(*first),
                _ => none(),
            });
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
            Heatmap::last_days(today, ACTIVITY_DAYS),
            first_day,
        );

        imp.timeline_list.remove_all();
        imp.blocks_shown.set(0);
        self.add_blocks(data.blocks, today);
    }

    /// Adds `blocks` to the end of the timeline.
    fn add_blocks(&self, blocks: Vec<ProjectBlock>, today: NaiveDate) {
        let imp = self.imp();
        let added = u32::try_from(blocks.len()).expect("at most BLOCKS_AT_ONCE blocks come");
        for block in &blocks {
            imp.timeline_list.append(&block_row(block, today));
        }
        imp.blocks_shown.set(imp.blocks_shown.get() + added);
        // A full batch may have more behind it.
        imp.more_button.set_visible(added == BLOCKS_AT_ONCE);
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
                    Ok(blocks) => view.add_blocks(blocks, Local::now().date_naive()),
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

/// A block in the timeline: its title, its day and time and how long it
/// took, opening it when activated.
fn block_row(block: &ProjectBlock, today: NaiveDate) -> adw::ActionRow {
    let title = if block.title.is_empty() {
        gettext("Untitled Block")
    } else {
        block.title.clone()
    };
    let minute = |minute: u32| format!("{:02}:{:02}", minute / 60 % 24, minute % 60);
    let row = adw::ActionRow::builder()
        .title(&title)
        .subtitle(format!(
            "{} · {}–{}",
            format_weekday_date(block.date, today),
            minute(block.start_minute),
            minute(block.end_minute)
        ))
        .use_markup(false)
        .activatable(true)
        .build();
    row.add_suffix(
        &gtk::Label::builder()
            .label(format_duration(block.duration()))
            .css_classes(["dim-label", "numeric"])
            .build(),
    );
    let (action, target) = block_action(block);
    // The target first, which the action needs.
    row.set_action_target_value(Some(&target));
    row.set_action_name(Some(action));
    row
}

/// The text of `note`, or none if it cannot be read.
fn note_text(vault: &Vault, note: &NotePath) -> String {
    vault.load_note(note).map_or_else(
        |err| {
            glib::g_warning!("knotbook", "{err}");
            String::new()
        },
        |file| file.text,
    )
}

/// Shows the start of a note's text, its Markdown formatted, laid out as on
/// a page.
fn note_preview() -> MarkdownView {
    let preview: MarkdownView = glib::Object::new();
    preview.set_full(true);
    preview.set_top_margin(48);
    preview.set_bottom_margin(48);
    preview.set_left_margin(56);
    preview.set_right_margin(56);
    // Clicks go to the card, which opens the note.
    preview.set_can_target(false);
    preview.set_focusable(false);
    preview
}

/// Shows the start of `text` in `preview`, unless it shows it already.
fn show_preview(preview: &MarkdownView, text: &str) {
    let text = preview_text(without_front_matter(text));
    if !preview.shows(text) {
        preview.set_markdown(text);
    }
}

/// A note in the grid: `preview` as the miniature of a page, above its
/// name and menu. The grid opens it when activated.
fn note_card(note: &NotePath, preview: &MarkdownView) -> gtk::FlowBoxChild {
    let page = Miniature::new(preview);
    page.add_css_class("note-page");
    // Takes up rounding, so that the names line up.
    page.set_vexpand(true);

    let footer = gtk::Box::builder().spacing(3).build();
    footer.append(
        &gtk::Label::builder()
            .label(note.name())
            .tooltip_text(note.name())
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            // Leaves the width of the card to the page.
            .max_width_chars(1)
            .margin_start(3)
            .build(),
    );
    footer.append(
        &gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(gettext("Note Menu"))
            .menu_model(&note_menu(note))
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build(),
    );

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    content.append(&page);
    content.append(&footer);
    gtk::FlowBoxChild::builder()
        .child(&content)
        .width_request(120)
        .css_classes(["note-card"])
        .build()
}

/// `text` without its front matter, if it starts with one.
fn without_front_matter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    let mut end = 0;
    for line in rest.split_inclusive('\n') {
        end += line.len();
        if line.trim_end() == "---" {
            return rest[end..].trim_start();
        }
    }
    text
}

/// The start of `text`, as much as a preview formats.
fn preview_text(text: &str) -> &str {
    let end = text
        .match_indices('\n')
        .nth(PREVIEW_LINES)
        .map_or(text.len(), |(end, _)| end);
    &text[..end]
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
