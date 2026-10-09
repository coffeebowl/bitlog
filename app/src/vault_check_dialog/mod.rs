//! The app's `bitlog doctor`: the problems in the vault, with a way to each
//! of them and fixes where the core has them. Both should find and fix the
//! same; a new problem or fix belongs in both.

mod problems;

use std::cell::{Ref, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{ReadError, Vault};
use bitlog_index::Found;
use gettextrs::gettext;
use glib::subclass::Signal;
use gtk::{gio, glib};

use self::problems::{Entry, Fix, Kind, Place, Report};
use crate::alert::show_error;
use crate::launch;
use crate::window::show_action;

/// An action name with its target.
type RowAction = (&'static str, glib::Variant);

/// The page of one kind of problems, while it is shown.
#[derive(Debug)]
pub struct KindPage {
    kind: Kind,
    preferences: adw::PreferencesPage,
    group: adw::PreferencesGroup,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/vault_check_dialog.ui")]
    pub struct VaultCheckDialog {
        #[template_child]
        pub navigation: TemplateChild<adw::NavigationView>,
        #[template_child]
        pub check_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub sync_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub severe_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub other_group: TemplateChild<adw::PreferencesGroup>,
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// What the last check found.
        pub report: RefCell<Option<Report>>,
        /// The rows of the overview, one for each kind, with their group.
        pub rows: RefCell<Vec<(adw::PreferencesGroup, adw::ActionRow)>>,
        pub kind_page: RefCell<Option<KindPage>>,
        /// The place chosen and the window to show it in once the dialog is
        /// closed.
        pub chosen: RefCell<Option<(gtk::Root, RowAction)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VaultCheckDialog {
        const NAME: &'static str = "BitLogVaultCheckDialog";
        type Type = super::VaultCheckDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for VaultCheckDialog {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            // Emitted after fixing problems in day files.
            SIGNALS.get_or_init(|| vec![Signal::builder("days-changed").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let dialog = self.obj();
            self.check_button.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| dialog.check()
            ));
            self.navigation.connect_popped(glib::clone!(
                #[weak]
                dialog,
                move |_, _| {
                    dialog.imp().kind_page.take();
                }
            ));
            dialog.connect_closed(|dialog| {
                if let Some((window, (action, target))) = dialog.imp().chosen.take() {
                    WidgetExt::activate_action(&window, action, Some(&target))
                        .expect("the rows run actions of the window");
                }
            });
        }
    }

    impl WidgetImpl for VaultCheckDialog {}
    impl AdwDialogImpl for VaultCheckDialog {}
}

glib::wrapper! {
    /// Lists the problems in the vault, as `bitlog doctor` does.
    pub struct VaultCheckDialog(ObjectSubclass<imp::VaultCheckDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// How many severe problems `vault` has, as the dialog marks them.
pub async fn count_severe(vault: &Vault) -> Result<usize, ReadError> {
    let vault = vault.clone();
    let report = gio::spawn_blocking(move || problems::check(&vault))
        .await
        .expect("checking the vault does not panic")?;
    Ok(report.severe())
}

impl VaultCheckDialog {
    /// A dialog that checks `vault` right away.
    pub fn new(vault: Rc<Vault>) -> Self {
        let dialog: Self = glib::Object::new();
        dialog.imp().vault.replace(Some(vault));
        dialog.check();
        dialog
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("the dialog is set up with a vault")
    }

    pub fn connect_days_changed(&self, callback: impl Fn() + 'static) {
        self.connect_closure(
            "days-changed",
            false,
            glib::closure_local!(move |_: &Self| callback()),
        );
    }

    /// Checks the vault in the background and shows what it finds.
    fn check(&self) {
        let imp = self.imp();
        imp.check_button.set_sensitive(false);
        // What is shown stays while checking again, so that lists keep
        // their place.
        if imp.report.borrow().is_none() {
            imp.stack.set_visible_child_name("checking");
        }
        imp.navigation.set_sensitive(false);
        let vault = Vault::clone(&self.vault());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            async move {
                let report = gio::spawn_blocking(move || problems::check(&vault))
                    .await
                    .expect("checking the vault does not panic");
                dialog.show(report);
            }
        ));
    }

    fn show(&self, report: Result<Report, ReadError>) {
        let imp = self.imp();
        imp.check_button.set_sensitive(true);
        imp.navigation.set_sensitive(true);
        let report = match report {
            Ok(report) => report,
            Err(err) => {
                imp.report.take();
                imp.navigation.pop_to_tag("overview");
                imp.error_page.set_description(Some(&err.to_string()));
                imp.stack.set_visible_child_name("error");
                return;
            }
        };
        self.show_overview(&report);
        imp.stack.set_visible_child_name("overview");
        imp.report.replace(Some(report));
        let shown = imp.kind_page.borrow().as_ref().map(|page| page.kind);
        if let Some(kind) = shown {
            self.update_kind_page(kind);
        }
    }

    /// Lists all kinds of problems, each with how many were found.
    fn show_overview(&self, report: &Report) {
        let imp = self.imp();
        for (group, row) in imp.rows.take() {
            group.remove(&row);
        }
        let rows = report
            .groups
            .iter()
            .map(|(kind, entries)| {
                let unchecked = *kind == Kind::UnusedImage && report.unused_images_unchecked;
                let row = self.overview_row(*kind, entries.len(), unchecked);
                // Each group is reported its own way, as its description says.
                let group = match kind {
                    Kind::Conflict => imp.sync_group.get(),
                    _ if kind.is_severe() => imp.severe_group.get(),
                    _ => imp.other_group.get(),
                };
                group.add(&row);
                (group, row)
            })
            .collect();
        imp.rows.replace(rows);
    }

    /// The row of `kind` in the overview: a check mark if nothing was
    /// found, else how many problems, leading to them.
    fn overview_row(&self, kind: Kind, count: usize, unchecked: bool) -> adw::ActionRow {
        let row = adw::ActionRow::builder().title(kind.title()).build();
        if unchecked {
            row.set_subtitle(&gettext("Only checked when all texts can be read"));
            row.add_suffix(
                &gtk::Label::builder()
                    .label(gettext("Not checked"))
                    .css_classes(["dim-label"])
                    .build(),
            );
        } else if count == 0 {
            row.add_suffix(
                &gtk::Image::builder()
                    .icon_name("object-select-symbolic")
                    .tooltip_text(gettext("No problems found"))
                    .css_classes(["success"])
                    .build(),
            );
        } else {
            row.add_suffix(
                &gtk::Label::builder()
                    .label(count.to_string())
                    .css_classes(["numeric"])
                    .build(),
            );
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.set_activatable(true);
            row.connect_activated(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |_| dialog.show_kind(kind)
            ));
        }
        row
    }

    /// The problems of `kind` the last check found.
    fn entries(&self, kind: Kind) -> Ref<'_, [Entry]> {
        Ref::map(self.imp().report.borrow(), |report| {
            let report = report.as_ref().expect("problems are shown after a check");
            report
                .groups
                .iter()
                .find(|(found, _)| *found == kind)
                .map_or(&[][..], |(_, entries)| entries)
        })
    }

    /// Opens a page with the problems of `kind`.
    fn show_kind(&self, kind: Kind) {
        let preferences = adw::PreferencesPage::new();
        let group = self.kind_group(kind, &self.entries(kind));
        preferences.add(&group);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&preferences));
        let page = adw::NavigationPage::builder()
            .title(kind.title())
            .child(&view)
            .build();
        let imp = self.imp();
        imp.navigation.push(&page);
        imp.kind_page.replace(Some(KindPage {
            kind,
            preferences,
            group,
        }));
    }

    /// Shows the problems of `kind` found now on its page, or goes back to
    /// the overview if none are left.
    fn update_kind_page(&self, kind: Kind) {
        let imp = self.imp();
        if self.entries(kind).is_empty() {
            imp.navigation.pop_to_tag("overview");
            return;
        }
        let group = self.kind_group(kind, &self.entries(kind));
        let mut kind_page = imp.kind_page.borrow_mut();
        let page = kind_page.as_mut().expect("the page of the kind is shown");
        let scrolled = page
            .preferences
            .first_child()
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("a preferences page scrolls");
        let position = scrolled.vadjustment().value();
        page.preferences.remove(&page.group);
        page.preferences.add(&group);
        page.group = group;
        scrolled.vadjustment().set_value(position);
    }

    fn kind_group(&self, kind: Kind, entries: &[Entry]) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .description(kind.description())
            .build();
        // Like `bitlog doctor --fix`, only all at once: the fixes of the
        // core work on whole days.
        let mut fixes: Vec<Fix> = entries.iter().filter_map(|entry| entry.fix).collect();
        fixes.dedup();
        if !fixes.is_empty() {
            group.set_header_suffix(Some(&self.fix_all_button(kind, entries.len(), fixes)));
        }
        for entry in entries {
            group.add(&self.row(entry));
        }
        group
    }

    fn fix_all_button(&self, kind: Kind, count: usize, fixes: Vec<Fix>) -> gtk::Button {
        let button = gtk::Button::builder()
            .label(gettext("_Fix All"))
            .use_underline(true)
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| {
                let fixes = fixes.clone();
                glib::spawn_future_local(async move {
                    if dialog.confirm_fix_all(kind, count).await {
                        dialog.fix(&fixes);
                    }
                });
            }
        ));
        button
    }

    async fn confirm_fix_all(&self, kind: Kind, count: usize) -> bool {
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Fix All?"))
            .body(kind.fix_all_question(count))
            .close_response("cancel")
            .default_response("fix")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("fix", &gettext("_Fix All")),
        ]);
        dialog.set_response_appearance("fix", adw::ResponseAppearance::Suggested);
        dialog.choose_future(Some(self)).await == "fix"
    }

    /// A row that shows where the problem is.
    fn row(&self, entry: &Entry) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&entry.title)
            .subtitle(&entry.subtitle)
            .activatable(true)
            .build();
        let (icon, tooltip) = match entry.place {
            Place::File(_) => ("folder-open-symbolic", gettext("Show in Folder")),
            _ => ("go-next-symbolic", gettext("Show")),
        };
        row.add_suffix(
            &gtk::Image::builder()
                .icon_name(icon)
                .tooltip_text(tooltip)
                .build(),
        );
        let place = entry.place.clone();
        row.connect_activated(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.show_place(&place)
        ));
        row
    }

    /// Shows `place` in the window, closing the dialog, or a file in its
    /// folder.
    fn show_place(&self, place: &Place) {
        let action = match place {
            Place::Day(date) => show_action(&Found::DayNote(*date)),
            Place::Block(date, id) => show_action(&Found::Block {
                date: *date,
                id: id.clone(),
            }),
            Place::Note(note) => show_action(&Found::Note(note.clone())),
            Place::Conflict(copy) => ("win.resolve-conflict", copy.to_string_lossy().to_variant()),
            Place::File(path) => {
                let path = path.clone();
                glib::spawn_future_local(glib::clone!(
                    #[weak(rename_to = dialog)]
                    self,
                    async move { launch::show_in_folder(&dialog, &path).await }
                ));
                return;
            }
        };
        let window = self.root().expect("the dialog is shown in a window");
        self.imp().chosen.replace(Some((window, action)));
        self.close();
    }

    /// Applies `fixes` and checks again.
    fn fix(&self, fixes: &[Fix]) {
        let vault = self.vault();
        for fix in fixes {
            if let Err(err) = fix.apply(&vault) {
                show_error(self, &gettext("Cannot Fix Problem"), &err.to_string());
                break;
            }
        }
        // Own writes are not watched.
        self.emit_by_name::<()>("days-changed", &[]);
        self.check();
    }
}
