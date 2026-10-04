//! One project: its time, notes, assets, blocks and, with a repository on
//! this device, its Git log.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Asset, AssetPath, NotePath, Period, Project, ProjectSlug, Vault};
use bitlog_index::{Found, ProjectBlock};
use chrono::{Local, NaiveDate, TimeDelta};
use gettextrs::{gettext, ngettext};
use gtk::{gdk, glib};

use crate::cards::{asset_card, note_card, note_preview, note_text, show_preview};
use crate::colors;
use crate::format::{
    format_duration, format_full_date, format_range, format_share, format_weekday_date,
    title_markup,
};
use crate::git_page::GitPage;
use crate::heatmap::Heatmap;
use crate::markdown_view::MarkdownView;
use crate::search_index::{ProjectData, SearchIndex};
use crate::widgets::param;
use crate::window::show_action;

mod assets;

/// Blocks the timeline shows at first and adds with "Load More".
const BLOCKS_AT_ONCE: u32 = 10;

/// The days the activity shows, up to today.
const ACTIVITY_DAYS: u64 = 30;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/project_view.ui")]
    pub struct ProjectView {
        /// The project shown.
        pub slug: RefCell<Option<ProjectSlug>>,
        /// The notes in the grid, in its order, with their previews.
        pub notes: RefCell<Vec<(NotePath, MarkdownView)>>,
        /// The assets in the grid, in its order, with their cards.
        pub assets: RefCell<Vec<(Asset, gtk::FlowBoxChild)>>,
        pub vault: RefCell<Option<Rc<Vault>>>,
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
        pub add_assets_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub assets_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub assets_grid: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub assets_error_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub timeline_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub more_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub git_tab: TemplateChild<adw::ViewStackPage>,
        #[template_child]
        pub git_page: TemplateChild<GitPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ProjectView {
        const NAME: &'static str = "BitLogProjectView";
        type Type = super::ProjectView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            GitPage::ensure_type();
            Heatmap::ensure_type();
            klass.bind_template();
            klass.install_action_async("assets.add", None, |view, _, _| async move {
                view.choose_assets().await;
            });
            klass.install_action_async("assets.open-folder", None, |view, _, _| async move {
                view.open_assets_folder().await;
            });
            klass.install_action_async(
                "assets.open",
                Some(glib::VariantTy::STRING),
                |view, _, asset| async move {
                    view.open_asset(param(asset.as_ref(), "assets"), false)
                        .await;
                },
            );
            klass.install_action_async(
                "assets.open-with",
                Some(glib::VariantTy::STRING),
                |view, _, asset| async move {
                    view.open_asset(param(asset.as_ref(), "assets"), true).await;
                },
            );
            klass.install_action_async(
                "assets.show",
                Some(glib::VariantTy::STRING),
                |view, _, asset| async move {
                    view.show_asset_in_folder(param(asset.as_ref(), "assets"))
                        .await;
                },
            );
            klass.install_action_async(
                "assets.rename",
                Some(glib::VariantTy::STRING),
                |view, _, asset| async move {
                    view.rename_asset(param(asset.as_ref(), "assets")).await;
                },
            );
            klass.install_action_async(
                "assets.trash",
                Some(glib::VariantTy::STRING),
                |view, _, asset| async move {
                    view.trash_asset(param(asset.as_ref(), "assets")).await;
                },
            );
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
            // The working tree is not watched, so it is read again when
            // the user comes to the Git page.
            self.view_stack
                .connect_visible_child_name_notify(glib::clone!(
                    #[weak(rename_to = view)]
                    self.obj(),
                    move |stack| {
                        if view.is_mapped() && stack.visible_child_name().as_deref() == Some("git")
                        {
                            view.imp().git_page.refresh();
                        }
                    }
                ));
            let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
            drop.connect_drop(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                #[upgrade_or]
                false,
                move |_, value, _, _| {
                    let Ok(files) = value.get::<gdk::FileList>() else {
                        return false;
                    };
                    let files = files.files();
                    glib::spawn_future_local(async move { view.add_assets(&files).await });
                    true
                }
            ));
            self.assets_stack.add_controller(drop);
            self.assets_grid.connect_child_activated(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_, card| {
                    let index = usize::try_from(card.index()).expect("cards are in the grid");
                    let target = view.imp().assets.borrow()[index]
                        .0
                        .path
                        .to_string()
                        .to_variant();
                    let _ = WidgetExt::activate_action(&view, "assets.open", Some(&target));
                }
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
    /// One project with its notes and assets.
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
    pub fn show(&self, vault: &Rc<Vault>, project: &Project) {
        let imp = self.imp();
        let repo = vault
            .repo_paths()
            .map(|mut repos| repos.remove(&project.slug))
            .unwrap_or_else(|err| {
                glib::g_warning!("bitlog", "{err}");
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
        imp.git_tab.set_visible(repo.is_some());
        imp.switcher.set_visible(repo.is_some());
        imp.slug.replace(Some(project.slug.clone()));
        imp.vault.replace(Some(vault.clone()));
        self.look_up(vault, project, repo);
        self.set_title(&project.name);
        imp.window_title.set_title(&project.name);
        imp.window_title.set_subtitle(&project.category);
        let target = project.slug.to_string().to_variant();
        imp.new_note_button.set_action_target_value(Some(&target));
        imp.edit_button.set_action_target_value(Some(&target));
        imp.empty_new_note_button
            .set_action_target_value(Some(&target));

        self.show_assets();

        imp.notes_grid.remove_all();
        imp.notes.take();
        let notes = match vault.notes(&project.slug) {
            Ok(notes) => notes,
            Err(err) => {
                imp.notes_error_row.set_subtitle(&err.to_string());
                imp.notes_stack.set_visible_child_name("error");
                imp.new_note_button.set_visible(true);
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
        imp.new_note_button.set_visible(!notes.is_empty());
        imp.notes.replace(notes);
    }

    /// Shows the assets of the project as they are now. The cards of the
    /// assets that did not change stay, so that their previews are not read
    /// again.
    pub fn show_assets(&self) {
        let imp = self.imp();
        let (Some(slug), Some(vault)) = (self.slug(), imp.vault.borrow().clone()) else {
            return;
        };
        imp.assets_grid.remove_all();
        let mut cards: HashMap<AssetPath, (Asset, gtk::FlowBoxChild)> = imp
            .assets
            .take()
            .into_iter()
            .map(|(asset, card)| (asset.path.clone(), (asset, card)))
            .collect();
        match vault.assets(&slug) {
            Ok(assets) => {
                let assets: Vec<_> = assets
                    .into_iter()
                    .map(|asset| {
                        let card = match cards.remove(&asset.path) {
                            Some((known, card)) if known == asset => card,
                            _ => asset_card(&vault, &asset),
                        };
                        imp.assets_grid.append(&card);
                        (asset, card)
                    })
                    .collect();
                imp.assets_stack
                    .set_visible_child_name(if assets.is_empty() { "empty" } else { "list" });
                imp.add_assets_button.set_visible(!assets.is_empty());
                imp.assets.replace(assets);
            }
            Err(err) => {
                imp.assets_error_row.set_subtitle(&err.to_string());
                imp.assets_stack.set_visible_child_name("error");
                imp.add_assets_button.set_visible(true);
            }
        }
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
    /// shows them, and the log of `repo` with the days worked on marked.
    fn look_up(&self, vault: &Rc<Vault>, project: &Project, repo: Option<PathBuf>) {
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
                let activity = match data {
                    Ok(data) => {
                        let activity = data.activity.iter().copied().collect();
                        view.show_data(&vault, &project, data, today);
                        activity
                    }
                    Err(err) => {
                        glib::g_warning!("bitlog", "{err}");
                        BTreeMap::new()
                    }
                };
                view.imp().git_page.show(repo, activity);
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
                    format_range(&format_full_date(*first), &format_full_date(*last))
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
        let color = colors::parse(&project.color);
        imp.heatmap.show(
            activity.iter().map(|(date, time)| (*date, *time, color)),
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
        // The page is titled with the project's name.
        let project_name = self.title();
        for block in &blocks {
            imp.timeline_list
                .append(&block_row(block, &project_name, today));
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
                    Err(err) => glib::g_warning!("bitlog", "{err}"),
                }
            }
        ));
    }
}

/// The window action that shows `block`, with its target.
fn block_action(block: &ProjectBlock) -> (&'static str, glib::Variant) {
    show_action(&Found::Block {
        date: block.date,
        id: block.id.clone(),
    })
}

/// A block in the timeline: its title, or else the name of its project,
/// its day and time and how long it took, opening it when activated.
fn block_row(block: &ProjectBlock, project_name: &str, today: NaiveDate) -> adw::ActionRow {
    let minute = |minute: u32| format!("{:02}:{:02}", minute / 60 % 24, minute % 60);
    let row = adw::ActionRow::builder()
        .title(title_markup(&block.title, project_name))
        .subtitle(glib::markup_escape_text(&format!(
            "{} · {}–{}",
            format_weekday_date(block.date, today),
            minute(block.start_minute),
            minute(block.end_minute)
        )))
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
