//! What the calendar tells about a day, in the cells of the month and again
//! in the tooltips of the month and the week.

use bitlog_core::{Day, ProjectSlug, Vault};
use chrono::TimeDelta;
use gtk::glib;

use crate::colors::{dot_markup, project_hex};
use crate::format::{format_duration, kind_name};

#[derive(Debug)]
pub struct DaySummary {
    pub working_time: TimeDelta,
    /// The projects with time on the day, the most first.
    pub times: Vec<(ProjectSlug, TimeDelta)>,
    /// The location, and the kind of a day off.
    pub facts: Vec<String>,
    pub is_day_off: bool,
}

impl DaySummary {
    pub fn new(vault: &Vault, day: &Day) -> Self {
        let mut times: Vec<_> = day.time_per_project(vault.projects()).into_iter().collect();
        times.retain(|(_, time)| !time.is_zero());
        times.sort_by_key(|(_, time)| std::cmp::Reverse(*time));
        let mut facts = Vec::new();
        if let Some(key) = &day.location {
            facts.push(vault.config().location_name(key).to_owned());
        }
        if !day.is_work() {
            facts.push(kind_name(&day.kind));
        }
        Self {
            working_time: day.working_time(vault.projects()),
            times,
            facts,
            is_day_off: !day.is_work(),
        }
    }

    /// All of it, for a tooltip and for screen readers.
    pub fn details(&self, vault: &Vault) -> Details {
        let mut details = Details::default();
        if !self.working_time.is_zero() {
            let text = format_duration(self.working_time);
            details.push(format!("<b>{}</b>", glib::markup_escape_text(&text)), text);
        }
        for (slug, time) in &self.times {
            let text = format!("{} · {}", vault.project_name(slug), format_duration(*time));
            details.push(dot_markup(project_hex(vault, slug), &text), text);
        }
        if !self.facts.is_empty() {
            let text = self.facts.join(" · ");
            details.push(glib::markup_escape_text(&text).into(), text);
        }
        details
    }
}

/// The details of a day in the calendar, for the tooltip as Pango markup,
/// and for screen readers.
#[derive(Debug, Default)]
pub struct Details {
    pub markup: Vec<String>,
    pub description: Vec<String>,
}

impl Details {
    pub fn push(&mut self, markup: String, text: String) {
        self.markup.push(markup);
        self.description.push(text);
    }

    /// The tooltip, `None` if there is nothing to tell.
    pub fn tooltip(&self) -> Option<String> {
        (!self.markup.is_empty()).then(|| self.markup.join("\n"))
    }
}
