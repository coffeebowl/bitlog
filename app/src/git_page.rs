//! The Git page of a project: the log of its repository on this device,
//! with branches and where the branch stands.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{
    Branch, Branches, Commit, GitLogError, RepoWatcher, Uncommitted, Upstream, git_branches,
    git_commit, git_log, git_uncommitted, git_upstream, watch_repo,
};
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};

use crate::alert::show_error;
use crate::commit_dialog::CommitDialog;
use crate::format::{format_duration, format_relative_day, format_time, plural};
use crate::widgets::Choices;

/// Commits the log shows at first and adds with "Load More".
const COMMITS_AT_ONCE: usize = 50;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/git_page.ui")]
    pub struct GitPage {
        /// The repository shown, if any.
        pub repo: RefCell<Option<PathBuf>>,
        /// Watches the repository, so that the log shows new commits.
        pub repo_watcher: RefCell<Option<RepoWatcher>>,
        /// The branch the log shows, `None` for the one checked out.
        pub branch: RefCell<Option<Branch>>,
        /// The branches of the repository as last read.
        pub branches: RefCell<Branches>,
        /// The branches in the dropdown, `None` for a detached HEAD.
        pub branch_choices: Choices<Option<Branch>>,
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
    impl ObjectSubclass for GitPage {
        const NAME: &'static str = "BitLogGitPage";
        type Type = super::GitPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for GitPage {
        fn constructed(&self) {
            self.parent_constructed();
            self.more_commits_button.connect_clicked(glib::clone!(
                #[weak(rename_to = page)]
                self.obj(),
                move |_| page.show_more_commits()
            ));
            self.branch_dropdown.connect_selected_notify(glib::clone!(
                #[weak(rename_to = page)]
                self.obj(),
                move |_| page.choose_branch()
            ));
            // The working tree is not watched, so it is read again when
            // the user may have changed it: when coming back to the window,
            // and when coming to the page (see `GitPage::refresh`).
            self.obj().connect_realize(|page| {
                let Some(window) = page.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let handler = window.connect_is_active_notify(glib::clone!(
                    #[weak]
                    page,
                    move |window| {
                        if window.is_active() && page.is_mapped() {
                            page.refresh();
                        }
                    }
                ));
                page.imp().activation.replace(Some((window, handler)));
            });
            self.obj().connect_unrealize(|page| {
                if let Some((window, handler)) = page.imp().activation.take() {
                    window.disconnect(handler);
                }
            });
        }
    }
    impl WidgetImpl for GitPage {}
    impl BinImpl for GitPage {}
}

glib::wrapper! {
    /// The log of a project's repository, with branches and where the
    /// branch stands.
    pub struct GitPage(ObjectSubclass<imp::GitPage>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl GitPage {
    /// Shows the log of `repo`, with the days in `activity`, the time
    /// spent on the project on each day, marked. Another repository starts
    /// at the branch checked out.
    pub fn show(&self, repo: Option<PathBuf>, activity: BTreeMap<NaiveDate, TimeDelta>) {
        let imp = self.imp();
        if *imp.repo.borrow() != repo {
            imp.branch.replace(None);
            self.watch_repo(repo.as_deref());
            imp.repo.replace(repo);
        }
        imp.activity.replace(activity);
        self.clear_commits();
        self.show_more_commits();
    }

    /// Reads the repository again, as when the page is shown.
    pub fn refresh(&self) {
        self.reload_commits();
    }
}

impl GitPage {
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
        let page: glib::SendWeakRef<Self> = self.downgrade().into();
        let watcher = watch_repo(repo, move || {
            let page = page.clone();
            glib::MainContext::default().invoke(move || {
                if let Some(page) = page.upgrade() {
                    page.reload_commits();
                }
            });
        });
        match watcher {
            Ok(watcher) => {
                imp.repo_watcher.replace(Some(watcher));
            }
            Err(err) => glib::g_warning!("bitlog", "{err}"),
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
        let read = imp.log_reads.get() + 1;
        imp.log_reads.set(read);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let log = gio::spawn_blocking(move || read_repo(&repo, branch, skip, limit))
                    .await
                    .expect("reading the log does not panic");
                let imp = page.imp();
                if imp.log_reads.get() != read {
                    return;
                }
                match log {
                    Ok(log) => {
                        imp.branch.replace(log.branch);
                        page.fill_branches(log.branches);
                        if let Some(status) = log.status {
                            page.show_status(status);
                        }
                        // Unchanged, as after staging, it is left as it is.
                        if skip == 0 && *imp.commits.borrow() != log.commits {
                            page.clear_commits();
                        }
                        if skip > 0 || imp.commits.borrow().is_empty() {
                            page.add_commits(log.commits, limit);
                        }
                    }
                    Err(err) => {
                        page.clear_commits();
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
        imp.branch_choices
            .fill(&*imp.branch_dropdown, items, &shown, |item| match item {
                Some(branch) => branch.name().to_owned(),
                None => gettext("Detached HEAD"),
            });
        imp.branch_dropdown.set_visible(true);
        imp.branches.replace(branches);
    }

    /// Shows the log of the branch chosen in the dropdown.
    fn choose_branch(&self) {
        let imp = self.imp();
        let Some(item) = imp.branch_choices.chosen(&*imp.branch_dropdown) else {
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
                #[weak(rename_to = page)]
                self,
                move |_| page.show_commit(id.clone())
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
            #[weak(rename_to = page)]
            self,
            async move {
                let read = repo.clone();
                let details = gio::spawn_blocking(move || git_commit(&read, &id))
                    .await
                    .expect("reading a commit does not panic");
                match details {
                    Ok(details) => CommitDialog::new(&details, repo).present(Some(&page)),
                    Err(err) => {
                        show_error(&page, &gettext("Cannot Read Commit"), &err.to_string());
                    }
                }
            }
        ));
    }

    /// The group of the commits made on `date`. It tells the time spent
    /// on the project that day, if any.
    fn day_group(&self, date: NaiveDate, today: NaiveDate) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(format_relative_day(date, today))
            .build();
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
