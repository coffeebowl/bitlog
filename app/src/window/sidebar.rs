//! The sidebar: the pages, then the active projects.

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{ProjectStatus, Vault};
use gettextrs::gettext;
use gtk::{gdk, glib, graphene};

use super::Window;
use crate::colors;
use crate::drawing;

/// The pages in the order of the sidebar, each with its title, its icon and
/// the action that shows it. The search offers them as commands.
pub fn pages() -> [(String, &'static str, &'static str); 6] {
    [
        (gettext("Today"), "weather-clear-symbolic", "win.show-today"),
        (
            gettext("Calendar"),
            "x-office-calendar-symbolic",
            "win.show-calendar",
        ),
        (
            gettext("Tasks"),
            "checkbox-checked-symbolic",
            "win.show-tasks",
        ),
        (gettext("Reports"), "view-grid-symbolic", "win.show-reports"),
        (
            gettext("Notes"),
            "text-x-generic-symbolic",
            "win.show-notes",
        ),
        (gettext("Projects"), "folder-symbolic", "win.show-projects"),
    ]
}

impl Window {
    /// Lists the pages in the sidebar and shows what is activated there.
    pub(super) fn add_sidebar_pages(&self) {
        let imp = self.imp();
        for (title, icon, _) in pages() {
            let item = adw::SidebarItem::builder()
                .title(title)
                .icon_name(icon)
                .build();
            imp.pages_section.append(item);
        }
        imp.sidebar.connect_activated(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, index| window.sidebar_activated(index as usize)
        ));
    }

    /// Lists the active projects of `vault` in the sidebar in their order,
    /// apart from breaks. The others are on the project page.
    pub(super) fn show_sidebar_projects(&self, vault: &Vault) {
        let imp = self.imp();
        imp.projects_section.remove_all();
        let projects = vault
            .projects()
            .iter()
            .filter(|project| project.status == ProjectStatus::Active && !project.is_break())
            .map(|project| {
                let item = adw::SidebarItem::builder()
                    .title(project.name.as_str())
                    .tooltip(glib::markup_escape_text(&project.name))
                    .icon_paintable(&dot_paintable(&project.color))
                    .build();
                imp.projects_section.append(item);
                project.slug.clone()
            })
            .collect();
        imp.sidebar_projects.replace(projects);
        self.select_project_item();
    }

    /// Shows the page or the project of the item `index` of the sidebar,
    /// which lists the pages first.
    fn sidebar_activated(&self, index: usize) {
        let pages = pages();
        if let Some((_, _, action)) = pages.get(index) {
            WidgetExt::activate_action(self, action, None)
                .expect("the window has the actions of its pages");
        } else {
            let slug = self.imp().sidebar_projects.borrow()[index - pages.len()].clone();
            self.show_project(&slug);
        }
    }

    /// Selects the item of the page that `action` shows in the sidebar.
    pub(super) fn select_page(&self, action: &str) {
        let index = pages().iter().position(|page| page.2 == action);
        let index = index.expect("the sidebar lists the pages shown");
        self.imp().sidebar.set_selected(index as u32);
    }

    /// Selects the item of the project shown in the sidebar, or that of the
    /// project page if the project has none, while the project page is
    /// shown.
    pub(super) fn select_project_item(&self) {
        let imp = self.imp();
        if !self.shows(&imp.projects_page) {
            return;
        }
        let shown = imp.projects_page.shown_project();
        let projects = imp.sidebar_projects.borrow();
        let position = projects
            .iter()
            .position(|slug| Some(slug) == shown.as_ref());
        match position {
            Some(position) => imp.sidebar.set_selected((pages().len() + position) as u32),
            None => self.select_page("win.show-projects"),
        }
    }
}

/// A dot in `color` as the icon of a project in the sidebar, as big as
/// the dot of [`colors::color_dot`] is in text.
fn dot_paintable(color: &str) -> gdk::Paintable {
    let snapshot = gtk::Snapshot::new();
    drawing::append_dot(&snapshot, 8.0, 8.0, 4.5, &colors::parse(color));
    snapshot
        .to_paintable(Some(&graphene::Size::new(16.0, 16.0)))
        .expect("a dot was drawn")
}
