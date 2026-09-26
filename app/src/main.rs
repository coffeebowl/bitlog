mod config;
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
    app.connect_activate(|app| Window::new(app).present());

    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &adw::Application, _, _| app.quit())
        .build();
    let about = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| show_about(app))
        .build();
    app.add_action_entries([quit, about]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
    app.set_accels_for_action("win.open-vault", &["<Control>o"]);

    app.run()
}

fn show_about(app: &adw::Application) {
    let developer = "coffeebowl";
    let dialog = adw::AboutDialog::builder()
        .application_name("Knotbook")
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
