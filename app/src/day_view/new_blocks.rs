//! Adding blocks: in free time chosen in the timeline, after the last
//! block, until now or for a project; and the commands the search offers
//! for the day.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{ProjectSlug, format_span, minute_of_day, time_at_minute};
use chrono::Local;
use gettextrs::gettext;
use gtk::glib;

use super::{DayView, location_choices};
use crate::dialogs::search_dialog::Command;
use crate::format::{DAY_KINDS, kind_name};
use crate::widgets::project_picker::{choosable_projects, project_popover};

impl DayView {
    /// What the search offers for the day shown: new blocks, each with the
    /// time it would take up, and its kind and location.
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = vec![Command::new(
            gettext("Standup"),
            "system-users-symbolic",
            "day.standup",
        )];
        if self.imp().file.borrow().is_none() {
            return commands;
        }
        if let Some(span) = self.next_slot() {
            // Through the window, which shows its shortcut.
            commands.push(
                Command::new(gettext("New Block"), "list-add-symbolic", "win.new-block")
                    .subtitle(format_span(span)),
            );
        }
        if let Some(span) = self.span_until_now() {
            commands.push(
                Command::new(
                    gettext("Block Until Now"),
                    "list-add-symbolic",
                    "day.block-until-now",
                )
                .subtitle(format_span(span)),
            );
        }
        if let Some(span) = self.project_span() {
            let vault = self.vault();
            for project in choosable_projects(&vault) {
                commands.push(
                    Command::new(
                        // Translators: A command of the search, as in "New Block for Webshop".
                        gettext("New Block for {project}").replace("{project}", &project.name),
                        "list-add-symbolic",
                        "day.new-block-for",
                    )
                    .target(project.slug.to_string())
                    .subtitle(format_span(span))
                    .when_typed(),
                );
            }
        }
        for kind in DAY_KINDS {
            commands.push(
                Command::new(
                    // Translators: A command of the search, as in "Kind: Vacation".
                    gettext("Kind: {kind}").replace("{kind}", &kind_name(kind)),
                    "calendar-symbolic",
                    "day.kind",
                )
                .target(kind)
                .when_typed(),
            );
        }
        for (key, name) in location_choices(&self.vault()) {
            commands.push(
                Command::new(
                    // Translators: A command of the search, as in "Location: Remote".
                    gettext("Location: {location}").replace("{location}", &name),
                    "mark-location-symbolic",
                    "day.location",
                )
                .target(key)
                .when_typed(),
            );
        }
        commands
    }

    /// Adds a block of `project` from `start` to `end`, in minutes of the
    /// day, and shows it in the panel.
    fn add_block(&self, start: u32, end: u32, project: ProjectSlug) {
        let vault = self.vault();
        let mut id = None;
        self.update(|day| {
            id = Some(day.add_block(
                time_at_minute(start),
                time_at_minute(end),
                project,
                "",
                vault.projects(),
            )?);
            Ok(())
        });
        if let Some(id) = id {
            self.show_block_by_id(&id);
        }
    }

    /// Asks for the project of a new block from `start` to `end`, in minutes
    /// of the day.
    pub(super) fn choose_project(&self, start: u32, end: u32) {
        let popover = project_popover(
            &self.vault(),
            glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |project| view.add_block(start, end, project)
            ),
        );
        self.imp().timeline.show_popover(&popover);
    }

    /// Offers one slot for a new block after the last one, or at the start
    /// of the grid.
    pub(super) fn new_block(&self) {
        let anchor = self.last_end();
        self.offer_block(anchor, anchor);
    }

    /// Offers a block from the end of the last one until now.
    pub(super) fn block_until_now(&self) {
        match self.span_until_now() {
            Some((start, end)) => self.offer_block(start, end),
            None => self.error_bell(),
        }
    }

    /// Adds a block of `project`, see `project_span`.
    pub(super) fn new_block_for(&self, project: ProjectSlug) {
        match self.project_span() {
            Some((start, end)) => self.add_block(start, end, project),
            None => self.error_bell(),
        }
    }

    /// Where a block for a project goes: until now if the day shown is
    /// today, else in the slot after the last block.
    fn project_span(&self) -> Option<(u32, u32)> {
        self.span_until_now().or_else(|| self.next_slot())
    }

    /// The slot after the last block, if it is free.
    fn next_slot(&self) -> Option<(u32, u32)> {
        let anchor = self.last_end();
        self.imp().timeline.free_span(anchor, anchor)
    }

    /// Where the last block ends, or the grid starts if there is none.
    fn last_end(&self) -> u32 {
        let file = self.imp().file.borrow();
        let day = &file
            .as_ref()
            .expect("new blocks need a day with a file")
            .day;
        let start = self.vault().config().grid.day_start;
        // A block into the next day leaves no room after it.
        day.blocks
            .iter()
            .map(|block| block.span().1)
            .filter(|&end| end < 24 * 60)
            .max()
            .unwrap_or(minute_of_day(start))
    }

    /// The free time from the end of the last block to the slot boundary
    /// nearest to now, if the day shown is today and that is later.
    fn span_until_now(&self) -> Option<(u32, u32)> {
        let now = Local::now();
        if now.date_naive() != self.date() {
            return None;
        }
        let slot = self.vault().config().grid.slot_minutes;
        let nearest = (minute_of_day(now.time()) + slot / 2) / slot * slot;
        let anchor = self.last_end();
        if nearest <= anchor {
            return None;
        }
        self.imp().timeline.free_span(anchor, nearest)
    }

    /// Marks the free time from `anchor` to `minute` and asks there for the
    /// project of a block in it.
    fn offer_block(&self, anchor: u32, minute: u32) {
        let imp = self.imp();
        let Some((start, end)) = imp.timeline.select_free_span(anchor, minute) else {
            self.error_bell();
            return;
        };
        // Once the grid has its size, scroll to the span and ask there.
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || {
                let imp = view.imp();
                let area = imp
                    .timeline
                    .pending_area()
                    .expect("the span is still marked");
                let point = gtk::graphene::Point::new(area.x(), area.y());
                if let Some(point) = imp.timeline.compute_point(&*imp.day_scroll, &point) {
                    let adjustment = imp.day_scroll.vadjustment();
                    let top = adjustment.value() + f64::from(point.y());
                    adjustment.clamp_page(top, top + f64::from(area.height()));
                }
                view.choose_project(start, end);
            }
        ));
    }
}
