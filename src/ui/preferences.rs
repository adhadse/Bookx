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
use adw::PreferencesWindow;
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
        get_widget!(builder, PreferencesWindow, settings_window);

        settings_window.set_transient_for(Some(window));

        let window = Self {
            widget: settings_window,
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
        let manager = adw::StyleManager::default();
        get_widget!(self.builder, gtk::Widget, appearance_group);
        appearance_group.set_visible(!manager.system_supports_color_schemes());

        get_widget!(self.builder, gtk::Widget, books_dir_btn);
        books_dir_btn.connect_clicked(glib::clone!(
            @strong books_dir_btn, @weak self as preferences => move |_, response| {

            }
        ))
    }

    fn setup_signals(&self) {
        get_widget!(self.builder, gtk::Switch, dark_mode_button);
        settings_manager::bind_property(Key::DarkMode, &dark_mode_button, "active");
    }
}
