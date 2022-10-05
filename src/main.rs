// Bookx - main.rs
// Copyright (C) 2022  Anurag Dhadse <hello@adhadse.com>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

mod application;
#[rustfmt::skip]
mod config;
mod library;
mod settings;
mod ui;

use application::BookxApplication;
use gettextrs;

mod deps {
    pub use gtk::{gdk, gdk_pixbuf, gio, glib, graphene};
}
use gtk::{gio, glib};

fn main() {
    // Initialize logger
    pretty_env_logger::init();

    // Prepare i18n
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain(config::PKGNAME, config::LOCALEDIR)
        .expect("Unable to bind the text domain");
    gettextrs::textdomain(config::PKGNAME).expect("Unable to switch to the text domain");

    // Load app resources
    let path = &format!(
        "{}/{}/{}.gresource",
        config::DATADIR,
        config::PKGNAME,
        config::APP_ID
    );
    let res = gio::Resource::load(path).expect("Could not load resources");
    gio::resources_register(&res);

    BookxApplication::run();
}
