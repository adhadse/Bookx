// Bookx - preferences.rs
// Copyright (C) 2022  Anurag Dhadse <hi@anuragdhadse.com>
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

use crate::settings::{settings_manager, Key};
use crate::ui::BookxWindow;
use crate::BookxApplication;
use adw::PreferencesWindow;
use glib::clone;
use gtk::glib;
use gtk::prelude::*;
use gtk_macros::*;

pub struct BookxPreferencesWindow {
    pub widget: PreferencesWindow,

    builder: gtk::Builder,
}

impl BookxPreferencesWindow {
    pub fn new(window: &gtk::Window) -> Self {
        let builder = gtk::Builder::from_resource("/com/anuragdhadse/Bookx/ui/preferences.ui");
        get_widget!(builder, PreferencesWindow, preferences_window);

        preferences_window.set_transient_for(Some(window));

        let window = Self {
            widget: preferences_window,
            builder,
        };

        window.setup_widgets();
        window.setup_signals();
        window
    }

    pub fn show(&self) {
        self.widget.set_visible(true);
    }

    fn setup_widgets(&self) {
        let bookx_win = BookxWindow::default();
        let bookx_app = BookxApplication::default();

        let manager = adw::StyleManager::default();
        get_widget!(self.builder, gtk::Widget, appearance_group);
        appearance_group.set_visible(!manager.system_supports_color_schemes());

        // TODO: remove it if adding folder action works
        get_widget!(self.builder, gtk::Button, books_dir_btn);
        books_dir_btn.set_label(
            bookx_win
                .get_books_folder()
                .query_info(
                    "standard::display-name",
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    gio::Cancellable::NONE,
                )
                .unwrap()
                .display_name()
                .as_str(),
        );
        books_dir_btn.connect_clicked(clone!(@strong books_dir_btn => move |_| {
            bookx_win.add_books_folder();
            books_dir_btn.set_label(bookx_win.get_books_folder().query_info(
                "standard::display-name",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                gio::Cancellable::NONE,
            ).unwrap().display_name().as_str());
        }));

        // Remove this section to disable removing directories
        get_widget!(self.builder, gtk::Button, remove_books_dir_btn);
        remove_books_dir_btn.connect_clicked(clone!(@strong remove_books_dir_btn => move |_| {
            settings_manager::set_string(Key::BooksDir, String::new());
            bookx_app.refresh_data();
        }));
    }

    fn setup_signals(&self) {
        get_widget!(self.builder, gtk::Switch, dark_mode_button);
        settings_manager::bind_property(Key::DarkMode, &dark_mode_button, "active");
    }
}
