mod alert;
mod asset_preview;
mod calendar_view;
mod cards;
mod colors;
mod commit_dialog;
mod config;
mod conflict_dialog;
mod day_view;
mod format;
mod git_page;
mod heatmap;
mod markdown_help_dialog;
mod markdown_view;
mod miniature;
mod note_dialogs;
mod note_view;
mod notes_page;
mod preferences_dialog;
mod project_dialog;
mod project_picker;
mod project_view;
mod projects_page;
mod reports_page;
mod search_dialog;
mod search_index;
mod share_bar;
mod standup_dialog;
mod style_switcher;
mod sync_conflict_dialog;
mod task_list_view;
mod tasks_page;
mod timeline;
mod week_chart;
mod widgets;
mod window;

use adw::prelude::*;
use gettextrs::{bind_textdomain_codeset, bindtextdomain, gettext, textdomain};
use gtk::{gio, glib};

use crate::window::Window;

fn main() -> glib::ExitCode {
    bindtextdomain(config::gettext_package(), config::localedir())
        .expect("the text domain can be bound");
    bind_textdomain_codeset(config::gettext_package(), "UTF-8")
        .expect("the text domain codeset can be set");
    textdomain(config::gettext_package()).expect("the text domain can be selected");

    let resources =
        gio::Resource::load(config::resources_file()).expect("the resources are installed");
    gio::resources_register(&resources);

    let app = adw::Application::builder()
        .application_id(config::app_id())
        .build();
    app.connect_startup(|app| {
        sourceview5::init();
        style_switcher::follow_settings(app, &gio::Settings::new(config::app_id()));
    });
    app.connect_activate(|app| Window::new(app).present());

    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| {
            // Closing the windows lets them save what is being typed; the
            // app ends with the last one.
            for window in app.windows() {
                window.close();
            }
        })
        .build();
    let about = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| show_about(app))
        .build();
    app.add_action_entries([quit, about]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
    app.set_accels_for_action("win.open-vault", &["<Control>o"]);
    // The same keys as in GNOME Calendar.
    app.set_accels_for_action("win.previous", &["<Alt>Left"]);
    app.set_accels_for_action("win.next", &["<Alt>Right"]);
    app.set_accels_for_action("win.today", &["<Control>t"]);
    app.set_accels_for_action("win.new-block", &["<Control>n"]);
    app.set_accels_for_action("win.search", &["<Control>k"]);
    app.set_accels_for_action("win.preferences", &["<Control>comma"]);

    app.run()
}

fn show_about(app: &adw::Application) {
    let developer = "coffeebowl";
    let dialog = adw::AboutDialog::builder()
        .application_name("BitLog")
        .application_icon(config::app_id())
        .comments(gettext("Keep a daily log of your work"))
        .version(config::version())
        .developer_name(developer)
        .developers([developer])
        .copyright(format!("© 2026 {developer}"))
        .license_type(gtk::License::Gpl30)
        // Translators: Replace "translator-credits" with your name, one per line.
        .translator_credits(gettext("translator-credits"))
        .build();
    dialog.present(app.active_window().as_ref());
}
