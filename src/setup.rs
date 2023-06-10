use relm4::gtk;

use gettextrs::{gettext, LocaleCategory};
use gtk::{gio, glib};

use crate::config::{APP_ID, LOCALEDIR, PKGNAME, RESOURCES_FILE};

pub fn setup() {
    // Initialize GTK
    gtk::init().unwrap();

    setup_gettext();

    glib::set_application_name(&gettext("Bookx"));

    let res = gio::Resource::load(RESOURCES_FILE).expect("Could not load gresource file");
    gio::resources_register(&res);

    setup_css(&res);

    gtk::Window::set_default_icon_name(APP_ID);
}

fn setup_gettext() {
    // Prepare i18n
    gettextrs::setlocale(LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain(PKGNAME, LOCALEDIR).expect("Unable to bind the text domain");
    gettextrs::textdomain(PKGNAME).expect("Unable to switch to the text domain");
}

fn setup_css(res: &gio::Resource) {
    let data = res
        .lookup_data(
            "/com/adhadse/Bookx/style.css",
            gio::ResourceLookupFlags::NONE,
        )
        .unwrap();
    relm4::set_global_css(&glib::GString::from_utf8_checked(data.to_vec()).unwrap());
}
