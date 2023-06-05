// Bookx - app.rs
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
use gtk::prelude::*;
use relm4::prelude::*;
use relm4::{
    actions::{ActionGroupName, RelmAction, RelmActionGroup},
    gtk, main_application, Component, ComponentController, ComponentParts, ComponentSender,
    Controller, SimpleComponent,
};

use gtk::prelude::{ApplicationExt, ApplicationWindowExt, GtkWindowExt, SettingsExt, WidgetExt};
use gtk::{gio, glib};

use crate::components::{AboutDialog, BookxMainContainer, BookxPreferences};
use crate::config::{APP_ID, PROFILE};

pub(super) struct App {
    bookx_preferences: Controller<BookxPreferences>,
    about_dialog: Controller<AboutDialog>,
    bookx_main_container: Controller<BookxMainContainer>,
}

#[derive(Debug)]
pub(super) enum Event {
    OpenPreferences,
    Quit,
}

relm4::new_action_group!(pub(super) WindowActionGroup, "win");
relm4::new_stateless_action!(PreferencesAction, WindowActionGroup, "preferences");
relm4::new_stateless_action!(pub(super) ShortcutsAction, WindowActionGroup, "show-help-overlay");
relm4::new_stateless_action!(AboutAction, WindowActionGroup, "about");

#[relm4::component(pub)]
impl SimpleComponent for App {
    /// The type of data with which this component will be initialized.
    type Init = ();
    /// The type of the messages that this component can receive.
    type Input = Event;
    /// The type of the messages that this component can send.
    type Output = ();
    /// A data structure that contains the widgets that you will need to update.
    type Widgets = AppWidgets;

    menu! {
        primary_menu: {
            section! {
                "_Preferences" => PreferencesAction,
                "_Keyboard" => ShortcutsAction,
                "_About Bookx" => AboutAction,
            }
        }
    }

    view! {
        main_window = gtk::ApplicationWindow::new(&main_application()) {
            connect_close_request[sender] => move |_| {
                sender.input(Event::Quit);
                gtk::Inhibit(true)
            },

            #[wrap(Some)]
            set_help_overlay: shortcuts = &gtk::Builder::from_resource(
                    "/com/adhadse/Bookx/gtk/help-overlay.ui"
                )
                .object::<gtk::ShortcutsWindow>("help_overlay")
                .unwrap() -> gtk::ShortcutsWindow {
                    set_transient_for: Some(&main_window),
                    set_application: Some(&main_application()),
            },

            add_css_class?: if PROFILE == "Devel" {
                    Some("devel")
                } else {
                    None
                },

            #[wrap(Some)]
            set_titlebar = &gtk::HeaderBar {
                pack_end = &gtk::MenuButton {
                    set_icon_name: "open-menu-symbolic",
                    set_menu_model: Some(&primary_menu),
                }
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_margin_all: 5,
                set_spacing: 5,

                append: model.bookx_main_container.widget()
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: &Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let about_dialog = AboutDialog::builder()
            .transient_for(root)
            .launch(())
            .detach();
        let bookx_preferences = BookxPreferences::builder()
            .transient_for(root)
            .launch(())
            .detach();
        let bookx_main_container = BookxMainContainer::builder().launch(()).detach();
        let model = Self {
            bookx_preferences,
            about_dialog,
            bookx_main_container,
        };

        let mut actions = RelmActionGroup::<WindowActionGroup>::new();
        let widgets = view_output!();

        let shortcuts_action = {
            let shortcuts = widgets.shortcuts.clone();
            RelmAction::<ShortcutsAction>::new_stateless(move |_| {
                shortcuts.present();
            })
        };

        let about_action = {
            let sender = model.about_dialog.sender().clone();
            RelmAction::<AboutAction>::new_stateless(move |_| {
                sender.send(()).unwrap();
            })
        };

        let preferences_action = {
            let sender = sender.input_sender().clone();
            RelmAction::<PreferencesAction>::new_stateless(move |_| {
                sender.send(Event::OpenPreferences).unwrap();
            })
        };

        actions.add_action(shortcuts_action);
        actions.add_action(about_action);
        actions.add_action(preferences_action);

        widgets
            .main_window
            .insert_action_group(WindowActionGroup::NAME, Some(&actions.into_action_group()));

        widgets.load_window_size();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            Event::Quit => main_application().quit(),
            Event::OpenPreferences => self.bookx_preferences.widget().present(),
        }
    }

    fn shutdown(&mut self, widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        widgets.save_window_size().unwrap();
    }
}

impl AppWidgets {
    fn save_window_size(&self) -> Result<(), glib::BoolError> {
        let settings = gio::Settings::new(APP_ID);
        let (width, height) = self.main_window.default_size();

        settings.set_int("window-width", width)?;
        settings.set_int("window-height", height)?;

        settings.set_boolean("is-maximized", self.main_window.is_maximized())?;

        Ok(())
    }

    fn load_window_size(&self) {
        let settings = gio::Settings::new(APP_ID);

        let width = settings.int("window-width");
        let height = settings.int("window-height");
        let is_maximized = settings.boolean("is-maximized");

        self.main_window.set_default_size(width, height);

        if is_maximized {
            self.main_window.maximize();
        }
    }
}
