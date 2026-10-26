//! Moving files and folders by dragging them onto a folder, and adding
//! those dragged here from other apps.

use adw::prelude::*;
use bitlog_core::AssetPath;
use gtk::{gdk, gio, glib, graphene};

use super::AssetsView;

/// How wide a card dragged is shown, small enough to leave its way in sight.
const DRAG_WIDTH: f32 = 64.0;

impl AssetsView {
    /// Lets files be dropped onto `widget`, to go into the folder `into`
    /// gives when they are, `None` for the top: moved if they are assets of
    /// the project, copied otherwise.
    pub(super) fn accept_drops(
        &self,
        widget: &impl IsA<gtk::Widget>,
        into: impl Fn(&Self) -> Option<AssetPath> + 'static,
    ) {
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_drop(glib::clone!(
            #[weak(rename_to = view)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(files) = value.get::<gdk::FileList>() else {
                    return false;
                };
                let files = files.files();
                let folder = into(&view);
                glib::spawn_future_local(async move { view.add_assets(&files, folder).await });
                true
            }
        ));
        widget.add_controller(drop);
    }

    /// Lets `card` be dragged as the file or folder `asset`, onto a folder
    /// here or into another app.
    pub(super) fn make_draggable(&self, card: &gtk::FlowBoxChild, asset: &AssetPath) {
        let file = gio::File::for_path(self.vault().asset_path(asset));
        let source = gtk::DragSource::new();
        // Another app would take a file moved there out of the vault. Here,
        // where it comes from decides whether it is moved.
        source.set_actions(gdk::DragAction::COPY);
        source.connect_prepare(move |source, _, _| {
            // A finger scrolls the grid instead.
            let touch = source
                .current_event_device()
                .is_some_and(|device| device.source() == gdk::InputSource::Touchscreen);
            if touch {
                return None;
            }
            let files = gdk::FileList::from_array(std::slice::from_ref(&file));
            Some(gdk::ContentProvider::for_value(&files.to_value()))
        });
        source.connect_drag_begin(glib::clone!(
            #[weak]
            card,
            move |source, _| {
                // Below and right of the pointer, so that what it points at
                // stays in sight.
                source.set_icon(small_picture(&card).as_ref(), 0, 0);
                card.add_css_class("dragging");
            }
        ));
        source.connect_drag_end(glib::clone!(
            #[weak]
            card,
            move |_, _, _| card.remove_css_class("dragging")
        ));
        card.add_controller(source);
    }
}

/// `card` as it looks now, before it is dimmed, `DRAG_WIDTH` wide.
fn small_picture(card: &gtk::FlowBoxChild) -> Option<gdk::Paintable> {
    let height = DRAG_WIDTH * card.height() as f32 / card.width() as f32;
    let size = graphene::Size::new(DRAG_WIDTH, height);
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(card))
        .current_image()
        .snapshot(&snapshot, f64::from(size.width()), f64::from(size.height()));
    snapshot.to_paintable(Some(&size))
}
