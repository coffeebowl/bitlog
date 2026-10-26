use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use bitlog_core::{Asset, AssetPath, ProjectSlug, Vault};
use gtk::{gdk, glib};

use crate::widgets::cards::{asset_card, folder_card};
use crate::widgets::param;

mod actions;

/// The characters of the folder shown its name keeps at least when the way
/// to it is shortened.
const CURRENT_CHARS: usize = 6;

/// The folders shown between "Files" and the folder shown at most, so that
/// the way fits on a phone; those above them are left out.
const FOLDERS_ON_THE_WAY: usize = 2;

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/bitlog/BitLog/assets_view.ui")]
    pub struct AssetsView {
        pub vault: RefCell<Option<Rc<Vault>>>,
        /// The project whose assets are shown.
        pub slug: RefCell<Option<ProjectSlug>>,
        /// The folder shown, `None` for the top of the assets.
        pub folder: RefCell<Option<AssetPath>>,
        /// The folders in the grid, in its order, before the files.
        pub folders: RefCell<Vec<AssetPath>>,
        /// The files in the grid, in its order, with their cards.
        pub files: RefCell<Vec<(Asset, gtk::FlowBoxChild)>>,
        #[template_child]
        pub files_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub back_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub crumbs: TemplateChild<gtk::Box>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub grid: TemplateChild<gtk::FlowBox>,
        #[template_child]
        pub error_row: TemplateChild<adw::ActionRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AssetsView {
        const NAME: &'static str = "BitLogAssetsView";
        type Type = super::AssetsView;
        type ParentType = adw::PreferencesGroup;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.install_action("assets.up", None, |view, _, _| view.show_parent());
            klass.install_action_async("assets.add", None, |view, _, _| async move {
                view.choose_assets().await;
            });
            klass.install_action_async("assets.new-folder", None, |view, _, _| async move {
                view.new_folder().await;
            });
            klass.install_action_async("assets.open-folder", None, |view, _, _| async move {
                view.open_folder().await;
            });
            klass.install_action_async(
                "assets.open",
                Some(glib::VariantTy::STRING),
                |view, _, asset| async move {
                    view.open_asset(param(asset.as_ref(), "assets")).await;
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

    impl ObjectImpl for AssetsView {
        fn constructed(&self) {
            self.parent_constructed();
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
            self.stack.add_controller(drop);
            self.files_button.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_| view.show_folder(None)
            ));
            self.grid.connect_child_activated(glib::clone!(
                #[weak(rename_to = view)]
                self.obj(),
                move |_, card| {
                    let index = usize::try_from(card.index()).expect("cards are in the grid");
                    view.open_card(index);
                }
            ));
            // Anywhere on the page, as in the file manager.
            let shortcuts = gtk::ShortcutController::new();
            shortcuts.set_scope(gtk::ShortcutScope::Managed);
            shortcuts.add_shortcut(gtk::Shortcut::new(
                Some(gtk::KeyvalTrigger::new(
                    gdk::Key::Up,
                    gdk::ModifierType::ALT_MASK,
                )),
                Some(gtk::NamedAction::new("assets.up")),
            ));
            self.obj().add_controller(shortcuts);
        }
    }
    impl WidgetImpl for AssetsView {}
    impl PreferencesGroupImpl for AssetsView {}
}

glib::wrapper! {
    /// The files of a project, in folders: those of one folder in a grid,
    /// folders first, with a way up to the folder above.
    pub struct AssetsView(ObjectSubclass<imp::AssetsView>)
        @extends adw::PreferencesGroup, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AssetsView {
    /// Shows the assets of the project `slug` in `vault`, in the folder
    /// shown last if it is the same project.
    pub fn show(&self, vault: &Rc<Vault>, slug: &ProjectSlug) {
        let imp = self.imp();
        if imp.slug.borrow().as_ref() != Some(slug) {
            imp.folder.take();
        }
        imp.slug.replace(Some(slug.clone()));
        imp.vault.replace(Some(vault.clone()));
        self.reload();
    }

    /// Shows the folder `folder`, or the top of the assets.
    pub fn show_folder(&self, folder: Option<AssetPath>) {
        self.imp().folder.replace(folder);
        self.reload();
    }

    fn show_parent(&self) {
        self.show_folder(self.folder().and_then(|folder| folder.parent()));
    }

    /// Shows the folder as it is now, or the nearest one above it that is
    /// left. The cards of the files that did not change stay, so that their
    /// previews are not read again.
    pub fn reload(&self) {
        let imp = self.imp();
        let (Some(slug), Some(vault)) = (imp.slug.borrow().clone(), imp.vault.borrow().clone())
        else {
            return;
        };
        let mut folder = imp.folder.take();
        while let Some(gone) = folder
            .as_ref()
            .filter(|folder| !vault.asset_path(folder).is_dir())
        {
            folder = gone.parent();
        }
        imp.folder.replace(folder.clone());
        self.action_set_enabled("assets.up", folder.is_some());

        // The window would keep the focus on a removed card or button, see
        // `TaskListView::show`.
        let focused = imp.grid.focus_child().is_some()
            || imp.crumbs.focus_child().is_some()
            || imp.files_button.has_focus();
        if focused && let Some(root) = self.root() {
            root.set_focus(None::<&gtk::Widget>);
        }
        self.show_crumbs(folder.as_ref());
        imp.grid.remove_all();
        let mut cards: HashMap<AssetPath, (Asset, gtk::FlowBoxChild)> = imp
            .files
            .take()
            .into_iter()
            .map(|(asset, card)| (asset.path.clone(), (asset, card)))
            .collect();
        match vault.asset_folder(&slug, folder.as_ref()) {
            Ok(found) => {
                for folder in &found.folders {
                    imp.grid.append(&folder_card(folder));
                }
                let files: Vec<_> = found
                    .files
                    .into_iter()
                    .map(|asset| {
                        let card = match cards.remove(&asset.path) {
                            Some((known, card)) if known == asset => card,
                            _ => asset_card(&vault, &asset),
                        };
                        imp.grid.append(&card);
                        (asset, card)
                    })
                    .collect();
                let empty = found.folders.is_empty() && files.is_empty();
                imp.stack
                    .set_visible_child_name(if empty { "empty" } else { "list" });
                imp.folders.replace(found.folders);
                imp.files.replace(files);
            }
            Err(err) => {
                imp.error_row.set_subtitle(&err.to_string());
                imp.stack.set_visible_child_name("error");
            }
        }
        if focused && let Some(card) = imp.grid.child_at_index(0) {
            card.grab_focus();
        }
    }

    fn slug(&self) -> Option<ProjectSlug> {
        self.imp().slug.borrow().clone()
    }

    fn vault(&self) -> Rc<Vault> {
        self.imp()
            .vault
            .borrow()
            .clone()
            .expect("assets are only shown with a vault")
    }

    /// The folder shown, `None` for the top of the assets.
    fn folder(&self) -> Option<AssetPath> {
        self.imp().folder.borrow().clone()
    }

    /// Shows the way to `folder` after the title, the folders above it
    /// buttons that go there, or nothing at the top.
    fn show_crumbs(&self, folder: Option<&AssetPath>) {
        let imp = self.imp();
        let crumbs = &imp.crumbs;
        while let Some(crumb) = crumbs.first_child() {
            crumbs.remove(&crumb);
        }
        imp.back_button.set_visible(folder.is_some());
        imp.files_button.set_can_target(folder.is_some());
        imp.files_button.set_focusable(folder.is_some());
        let Some(folder) = folder else {
            return;
        };
        let mut above = Vec::new();
        let mut parent = folder.parent();
        while let Some(folder) = parent {
            parent = folder.parent();
            above.insert(0, folder);
        }
        if above.len() > FOLDERS_ON_THE_WAY {
            let last_left_out = above.len() - FOLDERS_ON_THE_WAY;
            let ellipsis = crumb_label("…");
            ellipsis.set_tooltip_text(Some(above[last_left_out].path()));
            above.drain(..=last_left_out);
            crumbs.append(&crumb_separator());
            ellipsis.set_margin_start(10);
            ellipsis.set_margin_end(10);
            ellipsis.add_css_class("dim-label");
            crumbs.append(&ellipsis);
        }
        for target in above {
            crumbs.append(&crumb_separator());
            let button = gtk::Button::builder()
                .child(&crumb_label(target.name()))
                .css_classes(["flat", "crumb", "dim-label"])
                .build();
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = view)]
                self,
                move |_| view.show_folder_later(Some(target.clone()))
            ));
            crumbs.append(&button);
        }
        crumbs.append(&crumb_separator());
        let current = crumb_label(folder.name());
        // Lines up with the names on buttons.
        current.set_margin_start(10);
        // Shortened last, as the file manager does.
        let width = folder.name().chars().count().min(CURRENT_CHARS);
        current.set_width_chars(i32::try_from(width).expect("at most CURRENT_CHARS"));
        crumbs.append(&current);
    }

    /// Shows `folder` once GTK is done with the card or button that asked
    /// for it, which showing it removes.
    fn show_folder_later(&self, folder: Option<AssetPath>) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move || view.show_folder(folder)
        ));
    }

    /// Opens the folder or file of the card at `index` in the grid.
    fn open_card(&self, index: usize) {
        let imp = self.imp();
        let folder = imp.folders.borrow().get(index).cloned();
        if folder.is_some() {
            self.show_folder_later(folder);
            return;
        }
        let file = index - imp.folders.borrow().len();
        let target = imp.files.borrow()[file].0.path.to_string().to_variant();
        let _ = WidgetExt::activate_action(self, "assets.open", Some(&target));
    }
}

/// The name of a folder on the way to the one shown, shortened if the way
/// is too long.
fn crumb_label(name: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(name)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build()
}

fn crumb_separator() -> gtk::Image {
    gtk::Image::builder()
        .icon_name("go-next-symbolic")
        .css_classes(["dim-label"])
        .build()
}
