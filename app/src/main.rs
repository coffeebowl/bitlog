mod config;
mod window;

use adw::prelude::*;
use gettextrs::{bind_textdomain_codeset, bindtextdomain, textdomain};
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
    app.add_action_entries([quit]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);

    app.run()
}
