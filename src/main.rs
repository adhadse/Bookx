mod application;
#[rustfmt::skip]
mod config;
mod settings;
mod ui;

use application::BookxApplication;
use gettextrs::*;

mod deps {
    pub use gtk::{gdk, gdk_pixbuf, gio, glib, graphene};
}
use gtk::{gio, glib};

fn main() {
    // Initialize logger
    pretty_env_logger::init();

    // Prepare i18n
    setlocale(LocaleCategory::LcAll, "");
    bindtextdomain(config::PKGNAME, config::LOCALEDIR).expect("Unable to bind the text domain");
    textdomain(config::PKGNAME).expect("Unable to switch to the text domain");

    // Load app resources
    let path = &format!(
        "{}/{}/{}.gresource",
        config::DATADIR,
        config::PKGNAME,
        config::APP_ID
    );
    let res = gio::Resource::load(path).expect("Could not load resources");
    gio::resources_register(&res);

    glib::set_application_name(config::NAME);

    let app = BookxApplication::new();
    let _ret = app.run();
}
