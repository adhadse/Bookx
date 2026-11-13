// Bookx - preferences.rs
// Copyright (C) 2023  Anurag Dhadse <hello@adhadse.com>
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

use gettextrs::gettext;
use relm4::{
    adw::{self, prelude::*},
    gtk, ComponentParts, ComponentSender, SimpleComponent,
};

pub struct BookxPreferences {}

#[relm4::component(pub)]
impl SimpleComponent for BookxPreferences {
    type Init = ();
    type Input = ();
    type Output = ();

    view! {
        #[name = "preferences_window"]
        adw::PreferencesWindow {
            set_default_width: 480,
            set_search_enabled: false,
            set_modal: true,

            add = &adw::PreferencesPage {
                set_title: &gettext("General"),

                add = &adw::PreferencesGroup {
                    set_title: &gettext("Books Location"),
                    adw::ActionRow {
                        set_title: &gettext("Load books from folder"),
                        #[wrap(Some)]
                        set_activatable_widget = &gtk::Button::builder().valign(gtk::Align::Center).build(),
                    }
                }
            }
        }
    }

    fn init(
        _: Self::Init,
        root: &Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Self {};

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }
}
