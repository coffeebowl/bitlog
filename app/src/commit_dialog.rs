use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use knotbook_core::{ChangedFile, CommitDetails, FileChange, git_file_diff};
use sourceview5::prelude::*;

use crate::format::{format_full_date, format_time};

/// The files the dialog lists at most, so that huge commits open quickly.
const FILES_SHOWN: usize = 200;

/// The lines of a file's changes shown at most, so that huge ones such as
/// generated files open quickly.
const DIFF_LINES: usize = 1000;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/knotbook/Knotbook/commit_dialog.ui")]
    pub struct CommitDialog {
        /// The repository of the commit.
        pub repo: RefCell<PathBuf>,
        /// The full hash of the commit shown.
        pub id: RefCell<String>,
        /// The local day it was made on.
        pub date: Cell<NaiveDate>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub summary_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub body_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub id_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub copy_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub author_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub date_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub committer_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub parents_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub tags_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub day_row: TemplateChild<adw::ButtonRow>,
        #[template_child]
        pub files_group: TemplateChild<adw::PreferencesGroup>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CommitDialog {
        const NAME: &'static str = "KnotbookCommitDialog";
        type Type = super::CommitDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CommitDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            self.copy_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.copy_id()
            ));
            self.day_row.connect_activated(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.show_day()
            ));
        }
    }

    impl WidgetImpl for CommitDialog {}
    impl AdwDialogImpl for CommitDialog {}
}

glib::wrapper! {
    /// A commit with its whole message and the files it changed.
    pub struct CommitDialog(ObjectSubclass<imp::CommitDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CommitDialog {
    /// Shows `details` of a commit of the repository in the folder `repo`.
    pub fn new(details: &CommitDetails, repo: PathBuf) -> Self {
        let dialog: Self = glib::Object::new();
        dialog.imp().repo.replace(repo);
        dialog.show(details);
        dialog
    }

    fn show(&self, details: &CommitDetails) {
        let imp = self.imp();
        let commit = &details.commit;
        imp.id.replace(commit.id.clone());
        imp.date.set(commit.time.with_timezone(&Local).date_naive());

        // The summary is the first paragraph, the rest tells more.
        imp.summary_label.set_label(&commit.summary);
        let body = details
            .message
            .split_once("\n\n")
            .map_or("", |(_, body)| body.trim());
        imp.body_label.set_label(&reflow(body));
        imp.body_label.set_visible(!body.is_empty());

        imp.id_row.set_subtitle(&commit.id);
        imp.author_row
            .set_subtitle(&if details.author_email.is_empty() {
                commit.author.clone()
            } else {
                format!("{} <{}>", commit.author, details.author_email)
            });
        imp.date_row.set_subtitle(&format_when(commit.time));
        match &details.committer {
            Some((name, time)) => {
                imp.committer_row
                    .set_subtitle(&format!("{name} · {}", format_when(*time)));
            }
            None => imp.committer_row.set_visible(false),
        }
        let parents = &details.parents;
        imp.parents_row.set_visible(!parents.is_empty());
        imp.parents_row.set_title(&ngettext(
            "Parent",
            "Parents",
            u32::try_from(parents.len()).unwrap_or(u32::MAX),
        ));
        let short: Vec<_> = parents.iter().map(|id| &id[..7]).collect();
        imp.parents_row.set_subtitle(&short.join(", "));
        let tags = &commit.tags;
        imp.tags_row.set_visible(!tags.is_empty());
        imp.tags_row
            .set_title(&ngettext("Tag", "Tags", plural(tags.len())));
        imp.tags_row.set_subtitle(&tags.join(", "));

        self.show_files(&details.files);
    }

    /// Lists `files` with the lines added and removed, and their sum above.
    fn show_files(&self, files: &[ChangedFile]) {
        let group = &self.imp().files_group;
        let count = u32::try_from(files.len()).unwrap_or(u32::MAX);
        let (added, removed) = files
            .iter()
            .filter_map(|file| file.lines)
            .fold((0, 0), |(a, r), (added, removed)| (a + added, r + removed));
        // Translators: How many files a commit changed, as in "3 files
        // changed".
        let mut description = ngettext("{count} file changed", "{count} files changed", count)
            .replace("{count}", &count.to_string());
        let lines = format_lines(added, removed);
        if !lines.is_empty() {
            description = format!("{description} · {lines}");
        }
        group.set_description(Some(&description));

        for file in files.iter().take(FILES_SHOWN) {
            group.add(&self.file_row(file));
        }
        if files.len() > FILES_SHOWN {
            let more = files.len() - FILES_SHOWN;
            // Translators: The files of a commit left out of its list.
            let title = ngettext("{count} more file", "{count} more files", more as u32)
                .replace("{count}", &more.to_string());
            group.add(
                &adw::ActionRow::builder()
                    .title(title)
                    .css_classes(["dim-label"])
                    .build(),
            );
        }
    }

    /// A changed file: its path, how it changed unless it was only
    /// modified, and the lines added and removed. One with lines changed
    /// expands to show them.
    fn file_row(&self, file: &ChangedFile) -> gtk::Widget {
        let old = file.old_path.as_deref().unwrap_or_default();
        let change = match file.change {
            FileChange::Added => gettext("Added"),
            FileChange::Deleted => gettext("Deleted"),
            FileChange::Modified => String::new(),
            // Translators: A file renamed by a commit, as in "Renamed from
            // src/old.rs".
            FileChange::Renamed => gettext("Renamed from {path}").replace("{path}", old),
            // Translators: A file copied by a commit, as in "Copied from
            // src/old.rs".
            FileChange::Copied => gettext("Copied from {path}").replace("{path}", old),
            FileChange::TypeChanged => gettext("Type changed"),
        };
        let lines = match file.lines {
            Some((added, removed)) => format_lines(added, removed),
            None => gettext("Binary"),
        };
        let lines = (!lines.is_empty()).then(|| {
            gtk::Label::builder()
                .label(lines)
                .css_classes(["dim-label", "numeric", "caption"])
                .build()
        });
        if !file
            .lines
            .is_some_and(|(added, removed)| added + removed > 0)
        {
            let row = adw::ActionRow::builder()
                .title(&file.path)
                .subtitle(change)
                .use_markup(false)
                .build();
            if let Some(lines) = &lines {
                row.add_suffix(lines);
            }
            return row.upcast();
        }
        let row = adw::ExpanderRow::builder()
            .title(&file.path)
            .subtitle(change)
            .use_markup(false)
            .build();
        if let Some(lines) = &lines {
            row.add_suffix(lines);
        }
        // Read when first expanded, so that the dialog opens quickly.
        let read = Cell::new(false);
        let path = file.path.clone();
        row.connect_expanded_notify(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |row| {
                if row.is_expanded() && !read.replace(true) {
                    dialog.show_diff(row, path.clone());
                }
            }
        ));
        row.upcast()
    }

    /// Reads the changes of the file `path` in the background, then shows
    /// them in `row`.
    fn show_diff(&self, row: &adw::ExpanderRow, path: String) {
        let imp = self.imp();
        let repo = imp.repo.borrow().clone();
        let id = imp.id.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak]
            row,
            async move {
                let diff = gio::spawn_blocking(move || git_file_diff(&repo, &id, &path))
                    .await
                    .expect("reading a diff does not panic");
                let child = match diff {
                    Ok(diff) => diff_view(diff.as_deref().unwrap_or_default()),
                    Err(err) => gtk::Label::builder()
                        .label(err.to_string())
                        .wrap(true)
                        .xalign(0.0)
                        .margin_top(12)
                        .margin_bottom(12)
                        .margin_start(12)
                        .margin_end(12)
                        .css_classes(["error"])
                        .build()
                        .upcast(),
                };
                row.add_row(&child);
            }
        ));
    }

    fn copy_id(&self) {
        let imp = self.imp();
        self.clipboard().set_text(&imp.id.borrow());
        imp.toast_overlay
            .add_toast(adw::Toast::new(&gettext("Copied to clipboard")));
    }

    /// Shows the day the commit was made on, in place of the dialog.
    fn show_day(&self) {
        let target = self.imp().date.get().to_string().to_variant();
        let _ = WidgetExt::activate_action(self, "win.show-day", Some(&target));
        self.close();
    }
}

/// The hunks of `diff` in colours, at most `DIFF_LINES` lines of them.
fn diff_view(diff: &str) -> gtk::Widget {
    let total = diff.lines().count();
    let shown: String = diff
        .split_inclusive('\n')
        .take(DIFF_LINES)
        .collect::<String>();
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_language(
        sourceview5::LanguageManager::default()
            .language("diff")
            .as_ref(),
    );
    buffer.set_highlight_matching_brackets(false);
    buffer.set_text(shown.trim_end_matches('\n'));
    // Follows light and dark style like the rest of the app.
    adw::StyleManager::default()
        .bind_property("dark", &buffer, "style-scheme")
        .transform_to(|_, dark: bool| {
            let name = if dark { "Adwaita-dark" } else { "Adwaita" };
            sourceview5::StyleSchemeManager::default().scheme(name)
        })
        .sync_create()
        .build();
    let view = sourceview5::View::builder()
        .buffer(&buffer)
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(12)
        .right_margin(12)
        .build();
    // Long lines scroll sideways; the page scrolls down.
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&view)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .build();
    if total <= DIFF_LINES {
        return scrolled.upcast();
    }
    let more = total - DIFF_LINES;
    let text = ngettext("{count} more line", "{count} more lines", plural(more))
        .replace("{count}", &more.to_string());
    let label = gtk::Label::builder()
        .label(text)
        .margin_top(6)
        .margin_bottom(6)
        .css_classes(["dim-label"])
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&scrolled);
    content.append(&label);
    content.upcast()
}

/// `count` for choosing a plural form.
fn plural(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The lines added and removed, as in "+12 −3", leaving out none.
fn format_lines(added: usize, removed: usize) -> String {
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("+{added}"));
    }
    if removed > 0 {
        parts.push(format!("−{removed}"));
    }
    parts.join(" ")
}

/// `text` with the lines of each paragraph joined, as Git messages are
/// wrapped by hand. Lines that start a list item or are indented stay on
/// their own.
fn reflow(text: &str) -> String {
    let mut reflowed = String::new();
    for line in text.lines() {
        let own_line = line.is_empty()
            || line.starts_with([' ', '\t', '-', '*', '+'])
            || line.split_once(". ").is_some_and(|(number, _)| {
                !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
            });
        let joins = !own_line && !reflowed.is_empty() && !reflowed.ends_with('\n');
        if joins {
            reflowed.push(' ');
        } else if !reflowed.is_empty() {
            reflowed.push('\n');
        }
        reflowed.push_str(line);
    }
    reflowed
}

/// `time` in local time, as in "September 21, 2026 · 09:00".
fn format_when(time: DateTime<FixedOffset>) -> String {
    let local = time.with_timezone(&Local);
    format!(
        "{} · {}",
        format_full_date(local.date_naive()),
        format_time(local.time())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_are_joined_but_not_lists() {
        let text = "The notes are the readme now. The logo is a\nplaceholder.\n\nIt also:\n- adds new.rs\n- renames\n  the notes\n1. first\n2. second";
        assert_eq!(
            reflow(text),
            "The notes are the readme now. The logo is a placeholder.\n\nIt also:\n- adds new.rs\n- renames\n  the notes\n1. first\n2. second"
        );
    }
}
