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
    actions::{RelmAction, RelmActionGroup},
    adw, gtk, main_application, Component, ComponentController, ComponentParts, ComponentSender,
    Controller,
};

use gtk::prelude::{ApplicationExt, ApplicationWindowExt, GtkWindowExt, SettingsExt, WidgetExt};
use gtk::{gio, glib};

use crate::components::{
    AboutDialog, BookxMainContainer, BookxMainContainerMessage, BookxPreferences,
};
use crate::config::{APP_ID, PROFILE};

pub(super) struct App {
    bookx_preferences: Controller<BookxPreferences>,
    about_dialog: Controller<AboutDialog>,
    bookx_main_container: Controller<BookxMainContainer>,
    app_mode: AppMode,
}

#[derive(Debug)]
pub enum AppMode {
    Library,
    Reader,
}

#[derive(Debug)]
pub(super) enum Event {
    OpenPreferences,
    Quit,
    SetMode(AppMode),
}

relm4::new_action_group!(pub(super) WindowActionGroup, "win");
relm4::new_stateless_action!(PreferencesAction, WindowActionGroup, "preferences");
relm4::new_stateless_action!(pub(super) ShortcutsAction, WindowActionGroup, "show-help-overlay");
relm4::new_stateless_action!(AboutAction, WindowActionGroup, "about");

#[relm4::component(pub)]
impl Component for App {
    type CommandOutput = ();
    /// The type of data with which this component will be initialized.
    type Init = AppMode;
    /// The type of the messages that this component can receive.
    type Input = Event;
    /// The type of the messages that this component can send.
    type Output = Event;
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
        main_window = adw::ApplicationWindow::new(&main_application()) {
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

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,

                adw::HeaderBar {
                    #[name="navigation_back_button"]
                    pack_start = &gtk::Revealer {
                        set_transition_type: gtk::RevealerTransitionType::Crossfade,
                        set_reveal_child: false,
                        gtk::Button {
                            set_icon_name: "go-previous-symbolic",
                            set_visible: true,
                            connect_clicked[sender] => move |_| {
                                sender.input(Event::SetMode(AppMode::Library))
                            }
                        },
                    },
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
            },
        }
    }

    fn init(
        app_mode: Self::Init,
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
        let bookx_main_container = BookxMainContainer::builder().launch(()).forward(
            sender.input_sender(),
            |msg| match msg {
                BookxMainContainerMessage::OpenBookxReader => Event::SetMode(AppMode::Reader),
                BookxMainContainerMessage::CloseBookxReader => Event::SetMode(AppMode::Library),
            },
        );
        let model = Self {
            bookx_preferences,
            about_dialog,
            bookx_main_container,
            app_mode,
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

        actions.register_for_widget(&widgets.main_window);

        widgets.load_window_size();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            Event::Quit => main_application().quit(),
            Event::OpenPreferences => self.bookx_preferences.widget().present(),
            Event::SetMode(AppMode::Reader) => {
                widgets.load_bookx_reader_window_size();
                widgets.navigation_back_button.set_reveal_child(true);
                self.app_mode = AppMode::Reader;
            }
            Event::SetMode(AppMode::Library) => {
                widgets.save_bookx_reader_window_size().unwrap();
                widgets.load_window_size();
                widgets.navigation_back_button.set_reveal_child(false);
                self.app_mode = AppMode::Library;
                // let the main container know user want to move back to library
                self.bookx_main_container
                    .sender()
                    .send(BookxMainContainerMessage::CloseBookxReader)
                    .unwrap();
            }
        }
    }

    fn shutdown(&mut self, widgets: &mut Self::Widgets, output: relm4::Sender<Self::Output>) {
        match self.app_mode {
            AppMode::Library => {
                widgets.save_window_size().unwrap();
            }
            AppMode::Reader => {
                widgets.save_bookx_reader_window_size().unwrap();
            }
        }
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
        self.main_window.unmaximize();
        let settings = gio::Settings::new(APP_ID);

        let width = settings.int("window-width");
        let height = settings.int("window-height");
        let is_maximized = settings.boolean("is-maximized");

        self.main_window.set_default_size(width, height);

        if is_maximized {
            self.main_window.maximize();
        }
    }

    fn save_bookx_reader_window_size(&self) -> Result<(), glib::BoolError> {
        let settings = gio::Settings::new(APP_ID);
        let (width, height) = self.main_window.default_size();

        settings.set_int("bookx-reader-window-width", width)?;
        settings.set_int("bookx-reader-window-height", height)?;

        settings.set_boolean("is-bookx-reader-maximized", self.main_window.is_maximized())?;

        Ok(())
    }

    fn load_bookx_reader_window_size(&self) {
        tracing::info!("calling bookx reader resizing");
        let settings = gio::Settings::new(APP_ID);

        let bookx_reader_width = settings.int("bookx-reader-window-width");
        let bookx_reader_height = settings.int("bookx-reader-window-height");
        let is_maximized = settings.boolean("is-bookx-reader-maximized");

        self.main_window
            .set_default_size(bookx_reader_width, bookx_reader_height);

        if is_maximized {
            self.main_window.maximize();
        }
    }
}
