use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::{gettext, ngettext};
use gtk::{gdk, gio, glib};
use knotbook_core::{
    Branch, Branches, Commit, GitLogError, NotePath, Period, Project, ProjectSlug, RepoWatcher,
    Uncommitted, Upstream, Vault, git_branches, git_commit, git_log, git_uncommitted, git_upstream,
    watch_repo,
};
use knotbook_index::{Found, ProjectBlock};

use crate::alert::show_error;
use crate::commit_dialog::CommitDialog;
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
        /// Watches the repository, so that the log shows new commits.
        pub repo_watcher: RefCell<Option<RepoWatcher>>,
        /// The branch the log shows, `None` for the one checked out.
        pub branch: RefCell<Option<Branch>>,
        /// The branches of the repository as last read.
        pub branches: RefCell<Branches>,
        /// The branches in the dropdown, in its order, `None` for a
        /// detached HEAD.
        pub branch_items: RefCell<Vec<Option<Branch>>>,
        /// Set while the dropdown is filled, so that choosing reads nothing.
        pub filling_branches: Cell<bool>,
        /// Counts the reads of the log, so that one finishing after a
        /// newer one is dropped.
        pub log_reads: Cell<u32>,
        /// The time spent on the project on each day, to mark the days of
        /// commits.
        pub activity: RefCell<BTreeMap<NaiveDate, TimeDelta>>,
        /// The commits the log shows.
        pub commits: RefCell<Vec<Commit>>,
        /// The window whose activation reads the repository again, with
        /// the handler.
        pub activation: RefCell<Option<(gtk::Window, glib::SignalHandlerId)>>,
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
        pub branch_dropdown: TemplateChild<gtk::DropDown>,
        #[template_child]
        pub status_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub upstream_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub uncommitted_row: TemplateChild<adw::ActionRow>,
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
            self.branch_dropdown.connect_selected_notify(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.choose_branch()
            ));
            // The working tree is not watched, so it is read again when
            // the user may have changed it: when coming to the Git page or
            // back to the window.
            self.view_stack
                .connect_visible_child_name_notify(glib::clone!(
                    #[weak(rename_to = view)]
                    self.obj(),
                    move |_| view.refresh_git_page()
                ));
            self.obj().connect_realize(|view| {
                let Some(window) = view.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let handler = window.connect_is_active_notify(glib::clone!(
                    #[weak]
                    view,
                    move |window| {
                        if window.is_active() {
                            view.refresh_git_page();
                        }
                    }
                ));
                view.imp().activation.replace(Some((window, handler)));
            });
            self.obj().connect_unrealize(|view| {
                if let Some((window, handler)) = view.imp().activation.take() {
                    window.disconnect(handler);
                }
            });
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
            imp.branch.replace(None);
        }
        // Projects without code have no repository, and then no Git page
        // to switch to.
        imp.git_page.set_visible(repo.is_some());
        imp.switcher.set_visible(repo.is_some());
        if *imp.repo.borrow() != repo {
            self.watch_repo(repo.as_deref());
        }
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
                imp.notes_grid.append(&note_card(&note, &preview, None));
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

        // Row by row, as removing all would take the placeholder as well.
        while let Some(row) = imp.timeline_list.row_at_index(0) {
            imp.timeline_list.remove(&row);
        }
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
        imp.commits.take();
        imp.last_day.replace(None);
        imp.more_commits_button.set_visible(false);
    }

    /// Watches `repo`, if any, and reads it again after changes.
    fn watch_repo(&self, repo: Option<&Path>) {
        let imp = self.imp();
        imp.repo_watcher.replace(None);
        let Some(repo) = repo else {
            return;
        };
        let view: glib::SendWeakRef<Self> = self.downgrade().into();
        let watcher = watch_repo(repo, move || {
            let view = view.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(view) = view.upgrade() {
                    view.reload_commits();
                }
            });
        });
        match watcher {
            Ok(watcher) => {
                imp.repo_watcher.replace(Some(watcher));
            }
            Err(err) => glib::g_warning!("knotbook", "{err}"),
        }
    }

    /// Reads the repository again if the Git page is shown.
    fn refresh_git_page(&self) {
        let imp = self.imp();
        if self.is_mapped() && imp.view_stack.visible_child_name().as_deref() == Some("git") {
            self.reload_commits();
        }
    }

    /// Reads the next commits from the repository in the background and
    /// adds them to the log.
    fn show_more_commits(&self) {
        self.read_log(self.imp().commits.borrow().len(), COMMITS_AT_ONCE);
    }

    /// Reads the log again from the newest commit, as many commits as it
    /// shows, and replaces it.
    fn reload_commits(&self) {
        let shown = self.imp().commits.borrow().len();
        self.read_log(0, shown.max(COMMITS_AT_ONCE));
    }

    /// Reads at most `limit` commits after the first `skip` in the
    /// background, then shows them after the first `skip` of the log.
    /// Reading from the start also reads where the branch stands.
    fn read_log(&self, skip: usize, limit: usize) {
        let imp = self.imp();
        let Some(repo) = imp.repo.borrow().clone() else {
            return;
        };
        let branch = imp.branch.borrow().clone();
        let lookup = imp.lookups.get();
        let read = imp.log_reads.get() + 1;
        imp.log_reads.set(read);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let log = gio::spawn_blocking(move || read_repo(&repo, branch, skip, limit))
                    .await
                    .expect("reading the log does not panic");
                let imp = view.imp();
                if imp.lookups.get() != lookup || imp.log_reads.get() != read {
                    return;
                }
                match log {
                    Ok(log) => {
                        imp.branch.replace(log.branch);
                        view.fill_branches(log.branches);
                        if let Some(status) = log.status {
                            view.show_status(status);
                        }
                        // Unchanged, as after staging, it is left as it is.
                        if skip == 0 && *imp.commits.borrow() != log.commits {
                            view.clear_commits();
                        }
                        if skip > 0 || imp.commits.borrow().is_empty() {
                            view.add_commits(log.commits, limit);
                        }
                    }
                    Err(err) => {
                        view.clear_commits();
                        imp.branch_dropdown.set_visible(false);
                        imp.commits_error_page
                            .set_description(Some(&err.to_string()));
                        imp.commits_stack.set_visible_child_name("error");
                    }
                }
            }
        ));
    }

    /// Offers `branches` in the dropdown, with the one shown chosen.
    fn fill_branches(&self, branches: Branches) {
        let imp = self.imp();
        let mut items: Vec<Option<Branch>> = branches.all.iter().cloned().map(Some).collect();
        if branches.current.is_none() {
            items.insert(0, None);
        }
        let shown = imp
            .branch
            .borrow()
            .clone()
            .or_else(|| branches.current.clone().map(Branch::Local));
        let selected = items.iter().position(|item| *item == shown).unwrap_or(0);
        imp.filling_branches.set(true);
        if *imp.branch_items.borrow() != items {
            let labels: Vec<String> = items
                .iter()
                .map(|item| match item {
                    Some(branch) => branch.name().to_owned(),
                    None => gettext("Detached HEAD"),
                })
                .collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            imp.branch_dropdown
                .set_model(Some(&gtk::StringList::new(&labels)));
            imp.branch_items.replace(items);
        }
        imp.branch_dropdown
            .set_selected(u32::try_from(selected).expect("branches fit in a list"));
        imp.filling_branches.set(false);
        imp.branch_dropdown.set_visible(true);
        imp.branches.replace(branches);
    }

    /// Shows the log of the branch chosen in the dropdown.
    fn choose_branch(&self) {
        let imp = self.imp();
        if imp.filling_branches.get() {
            return;
        }
        let index = usize::try_from(imp.branch_dropdown.selected()).expect("u32 fits in usize");
        let Some(item) = imp.branch_items.borrow().get(index).cloned() else {
            return;
        };
        // Choosing the branch checked out follows HEAD to the next one
        // checked out.
        let current = imp.branches.borrow().current.clone().map(Branch::Local);
        let branch = item.filter(|branch| Some(branch) != current.as_ref());
        imp.branch.replace(branch);
        self.clear_commits();
        self.read_log(0, COMMITS_AT_ONCE);
    }

    /// Shows where the branch stands against the one it tracks and, for
    /// the one checked out, what is not committed yet.
    fn show_status(&self, status: Status) {
        let imp = self.imp();
        match &status.upstream {
            Some(upstream) => {
                imp.upstream_row.set_subtitle(&upstream_text(upstream));
                imp.upstream_row.set_visible(true);
            }
            None => imp.upstream_row.set_visible(false),
        }
        match status.uncommitted {
            Some(uncommitted) => {
                imp.uncommitted_row
                    .set_subtitle(&uncommitted_text(uncommitted));
                imp.uncommitted_row.set_visible(true);
            }
            None => imp.uncommitted_row.set_visible(false),
        }
        imp.status_group
            .set_visible(status.upstream.is_some() || status.uncommitted.is_some());
    }

    /// Adds `commits`, newest first, to the end of the log, grouped by the
    /// local day they were made on. They are at most `limit`, as read.
    fn add_commits(&self, commits: Vec<Commit>, limit: usize) {
        let imp = self.imp();
        let today = Local::now().date_naive();
        for commit in &commits {
            let date = commit.time.with_timezone(&Local).date_naive();
            let group = match &*imp.last_day.borrow() {
                Some((last, group)) if *last == date => group.clone(),
                _ => {
                    let group = self.day_group(date, today);
                    imp.commits_box.append(&group);
                    group
                }
            };
            let row = commit_row(commit);
            let id = commit.id.clone();
            row.connect_activated(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_| view.show_commit(id.clone())
            ));
            group.add(&row);
            imp.last_day.replace(Some((date, group)));
        }
        // A full batch may have more behind it.
        imp.more_commits_button.set_visible(commits.len() == limit);
        let mut shown = imp.commits.borrow_mut();
        shown.extend(commits);
        imp.commits_stack
            .set_visible_child_name(if shown.is_empty() { "empty" } else { "list" });
    }

    /// Reads the commit `id` with the files it changed in the background,
    /// then shows it in a dialog.
    fn show_commit(&self, id: String) {
        let Some(repo) = self.imp().repo.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = view)]
            self,
            async move {
                let read = repo.clone();
                let details = gio::spawn_blocking(move || git_commit(&read, &id))
                    .await
                    .expect("reading a commit does not panic");
                match details {
                    Ok(details) => CommitDialog::new(&details, repo).present(Some(&view)),
                    Err(err) => {
                        show_error(&view, &gettext("Cannot Read Commit"), &err.to_string());
                    }
                }
            }
        ));
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

/// What the Git page shows, read from the repository.
struct Log {
    branches: Branches,
    /// The branch shown, `None` for the one checked out.
    branch: Option<Branch>,
    commits: Vec<Commit>,
    /// Only when reading from the newest commit.
    status: Option<Status>,
}

/// Where the branch shown stands.
struct Status {
    /// Against the branch it tracks, if it is a local one that tracks one.
    upstream: Option<Upstream>,
    /// Only for the branch checked out.
    uncommitted: Option<Uncommitted>,
}

/// Reads at most `limit` commits of `branch` in `repo` after the first
/// `skip`, with the branches and, from the start, where it stands. A
/// branch deleted elsewhere gives way to the one checked out.
fn read_repo(
    repo: &Path,
    branch: Option<Branch>,
    skip: usize,
    limit: usize,
) -> Result<Log, GitLogError> {
    let branches = git_branches(repo)?;
    let branch = branch.filter(|branch| branches.all.contains(branch));
    let commits = git_log(repo, branch.as_ref(), skip, limit)?;
    let status = if skip == 0 {
        let local = match &branch {
            Some(Branch::Local(name)) => Some(name.as_str()),
            Some(Branch::Remote(_)) => None,
            None => branches.current.as_deref(),
        };
        let upstream = match local {
            Some(name) => git_upstream(repo, name)?,
            None => None,
        };
        let uncommitted = match branch {
            Some(_) => None,
            None => Some(git_uncommitted(repo)?),
        };
        Some(Status {
            upstream,
            uncommitted,
        })
    } else {
        None
    };
    Ok(Log {
        branches,
        branch,
        commits,
        status,
    })
}

/// Such as "origin/main · 2 to push · 3 to pull".
fn upstream_text(upstream: &Upstream) -> String {
    let (ahead, behind) = (upstream.ahead, upstream.behind);
    let mut parts = vec![upstream.name.clone()];
    if ahead > 0 {
        // Translators: Commits of a branch not pushed yet, as in "2 to push".
        let text = ngettext("{count} to push", "{count} to push", plural(ahead));
        parts.push(text.replace("{count}", &ahead.to_string()));
    }
    if behind > 0 {
        // Translators: Commits of the remote branch not pulled yet, as in
        // "3 to pull".
        let text = ngettext("{count} to pull", "{count} to pull", plural(behind));
        parts.push(text.replace("{count}", &behind.to_string()));
    }
    if ahead == 0 && behind == 0 {
        parts.push(gettext("up to date"));
    }
    parts.join(" · ")
}

/// Such as "2 changed files · 1 new file".
fn uncommitted_text(uncommitted: Uncommitted) -> String {
    let Uncommitted { changed, new } = uncommitted;
    let mut parts = Vec::new();
    if changed > 0 {
        let text = ngettext(
            "{count} changed file",
            "{count} changed files",
            plural(changed),
        );
        parts.push(text.replace("{count}", &changed.to_string()));
    }
    if new > 0 {
        let text = ngettext("{count} new file", "{count} new files", plural(new));
        parts.push(text.replace("{count}", &new.to_string()));
    }
    if parts.is_empty() {
        return gettext("Nothing to commit");
    }
    parts.join(" · ")
}

/// `count` for choosing a plural form.
fn plural(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// A commit in the log: its summary, then hash, author and time. The log
/// shows it in detail when activated.
fn commit_row(commit: &Commit) -> adw::ActionRow {
    let time = commit.time.with_timezone(&Local).time();
    let row = adw::ActionRow::builder()
        .title(&commit.summary)
        .subtitle(format!(
            "{} · {} · {}",
            commit.short_id(),
            commit.author,
            format_time(time)
        ))
        .use_markup(false)
        .activatable(true)
        .build();
    for tag in &commit.tags {
        row.add_suffix(
            &gtk::Label::builder()
                .label(tag)
                .tooltip_text(tag)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .max_width_chars(16)
                .valign(gtk::Align::Center)
                .css_classes(["commit-tag", "caption-heading"])
                .build(),
        );
    }
    row
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
pub fn note_text(vault: &Vault, note: &NotePath) -> String {
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
pub fn note_preview() -> MarkdownView {
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
pub fn show_preview(preview: &MarkdownView, text: &str) {
    let text = preview_text(without_front_matter(text));
    if !preview.shows(text) {
        preview.set_markdown(text);
    }
}

/// A note in the grid: `preview` as the miniature of a page, above its
/// name, `details` if given, and menu. The grid opens it when activated.
pub fn note_card(
    note: &NotePath,
    preview: &MarkdownView,
    details: Option<&gtk::Label>,
) -> gtk::FlowBoxChild {
    let page = Miniature::new(preview);
    page.add_css_class("note-page");
    // Takes up rounding, so that the names line up.
    page.set_vexpand(true);

    let labels = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    labels.append(
        &gtk::Label::builder()
            .label(note.name())
            .tooltip_text(note.name())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            // Leaves the width of the card to the page.
            .max_width_chars(1)
            .margin_start(3)
            .build(),
    );
    if let Some(details) = details {
        details.set_xalign(0.0);
        details.set_ellipsize(gtk::pango::EllipsizeMode::End);
        details.set_max_width_chars(1);
        details.set_margin_start(3);
        details.add_css_class("caption");
        details.add_css_class("dim-label");
        labels.append(details);
    }
    let footer = gtk::Box::builder().spacing(3).build();
    footer.append(&labels);
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
