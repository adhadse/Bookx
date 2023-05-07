// Bookx - application.rs
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

use crate::application::Action::Book;
use crate::{
    config,
    models::BookAction,
    settings::{settings_manager, Key},
    views::BookxLibrary,
    widgets::{BookxPreferencesWindow, BookxView, BookxWindow},
};
use adw::{prelude::*, subclass::prelude::*, ToastPriority};
use gtk::{
    glib::{self, clone, ObjectExt, ParamSpec, ParamSpecObject, Receiver, Sender, ToValue},
    subclass::application::GtkApplicationImpl,
};
use gtk_macros::*;
use log::{debug, error, info};
use once_cell::{sync::Lazy, sync::OnceCell};
use std::{cell::RefCell, rc::Rc, str::FromStr};

#[derive(Debug, Clone)]
pub enum Action {
    // BookxApplication.process_action() handles sending actions between
    // different senders and receivers using send! macro
    SettingsKeyChanged(Key),
    Notification(String),
    Book(Box<BookAction>),
}

mod imp {
    use super::*;

    // The basic struct that holds our
    // state and widgets
    pub struct BookxApplication {
        pub sender: Sender<Action>,
        pub receiver: RefCell<Option<Receiver<Action>>>,

        pub window: BookxWindow,
        pub settings: gio::Settings,
    }

    // Sets up the basics for the GObject
    // The `#[glib::object_subclass] macro implements
    // some boilerplate code for the object setup, e.g. get_type()
    #[glib::object_subclass]
    impl ObjectSubclass for BookxApplication {
        const NAME: &'static str = "BookxApplication";
        type Type = super::BookxApplication;
        type ParentType = adw::Application;
    }

    // Overrides GObject vfuncs
    impl ObjectImpl for BookxApplication {
        fn constructed(&self) {
            let obj = self.instance();
            self.parent_constructed();

            // Force dark theme
            obj.style_manager()
                .set_color_scheme(adw::ColorScheme::PreferDark);

            // Setup actions
            obj.setup_actions();
        }
    }

    // Overrides Gio.Application for Bookx
    impl ApplicationImpl for BookxApplication {
        fn activate(&self) {
            let application = self.instance();
            let window = BookxWindow::new(self.sender.clone(), &application);

            debug!("gio::Application -> activate()");
            let app = application
                .downcast_ref::<super::BookxApplication>()
                .unwrap();
            glib::set_application_name(config::NAME);
            gtk::Window::set_default_icon_name(config::APP_ID);

            // If the window already exists,
            // present it instead creating a new one again
            if let Some(weak_window) = self.window.get() {
                weak_window.upgrade().unwrap().present();
                info!("Application window presented");
                return;
            }

            // No window available -> create one
            let window = app.create_window();
            let _ = self.window.set(window.downgrade());
            info!("Created application window.");

            // Setup app level GActions
            app.setup_actions();

            // Setup action channel
            let receiver = self.receiver.borrow_mut().take().unwrap();
            receiver.attach(
                None,
                clone!(@strong app => move |action| app.process_action(action)),
            );

            // Retrieve Books data
            app.refresh_data();

            // Setup settings signal (we get notified when a key gets changed)
            self.settings.connect_changed(
                None,
                clone!(@strong self.sender as sender => move |_, key_str| {
                   let key: Key = Key::from_str(key_str).unwrap();
                   send!(sender, Action::SettingsKeyChanged(key));
                }),
            );

            // TODO: Needs to be called after settings.connect_changed for it to trigger.
            app.update_color_scheme();
        }

        fn startup(&self, app: &Self::Type) {
            debug!("GtkApplication<BookxApplication>::startup");
            self.parent_startup(app);
        }
    }

    // This is empty, but we still need to provide an
    // empty implementation for each type we subclass.
    impl GtkApplicationImpl for BookxApplication {}
    impl AdwApplicationImpl for BookxApplication {}
}

// Creates a wrapper struct that inherits the functions
// from objects listed as @extends or interfaces it @implements.
// This is what allows us to do e.g. application.quit() on
// BookxApplication without casting.
glib::wrapper! {
    pub struct BookxApplication(ObjectSubclass<imp::BookxApplication>)
        @extends gio::Application, gtk::Application, adw::Application;
        //@implements gio::ActionGroup, gio::ActionMap;
}

// This is where the member functions of BookxApplication go.
impl BookxApplication {
    pub fn new() -> Self {
        let (sender, r) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
        let receiver = RefCell::new(Some(r));

        let settings = settings_manager::settings();

        info!("Bookx ({})", config::APP_ID);
        info!("Version: {} ({})", config::VERSION, config::PROFILE);
        info!("Datadir: {}", config::DATADIR);

        let app = glib::Object::builder()
            .property("application-id", config::APP_ID)
            .property("flags", config::APP_ID)
            .property("resource-base-path", config::DATADIR)
            .build();

        app
    }

    fn main_window(&self) -> BookxWindow {
        self.imp().window.get().unwrap().upgrade().unwrap()
    }

    fn create_window(&self) -> BookxWindow {
        let imp = self.imp();
        let window = BookxWindow::new(imp.sender.clone(), self.clone());

        // Set initial view
        window.set_view(BookxView::Library);

        window.present();
        window
    }

    fn setup_actions(&self) {
        // gio::ActionEntryBuilder allows us to build and store an action on an object
        // that implements gio::ActionMap. Here we build the application's actions and
        // add them with add_action_entries().
        let actions = [
            gio::ActionEntryBuilder::new("about")
                .activate(|app: &Self, _, _| app.show_about())
                .build(),
            gio::ActionEntryBuilder::new("help")
                .activate(|app: &Self, _, _| app.show_help())
                .build(),
            gio::ActionEntryBuilder::new("quit")
                .activate(|app: &Self, _, _| app.quit())
                .build,
            gio::ActionEntryBuilder::new("new-window")
                .activate(|app: &Self, _, _| {
                    let win = BookxWindow::new(self.sender, app.clone());
                    win.present();
                })
                .build(),
        ];

        self.add_action_entries(actions).unwrap();

        // Sets up keyboard shortcuts
        self.set_accels_for_action("app.help", &["F1"]);
        self.set_accels_for_action("app.show-preferences", &["<primary>comma"]);
        self.set_accels_for_action("app.quit", &["<primary>q"]);
    }

    fn process_action(&self, action: Action) -> glib::Continue {
        let _imp = self.imp();
        if self.active_window().is_none() {
            return glib::Continue(true);
        }

        let _window = BookxWindow::default();

        match action {
            Action::SettingsKeyChanged(key) => self.apply_settings_changes(key),
            Action::Notification(message) => self
                .main_window()
                .show_notification(message, ToastPriority::Normal),
            Action::Book(book_action) => self.process_book_action(book_action),
        }
        glib::Continue(true)
    }

    // handles action specific to a book
    fn process_book_action(&self, book_action: Box<BookAction>) {
        match *book_action {
            BookAction::ShowBookProperties(book) => {
                self.main_window().show_book_properties(book);
            }
            BookAction::EditMetadata(book) => {
                // TODO: call library function create a new window to edit metadata
            }
            BookAction::DeleteBook(book) => {
                // TODO: call library and delete book file
            }
            BookAction::ExportAnnotations(book) => {
                // TODO: call library
            }
            BookAction::SendToDevice(book) => {
                // TODO: call library
            }
            BookAction::OpenBook(book) => {
                // TODO
            }
        }
    }

    fn apply_settings_changes(&self, key: Key) {
        debug!("Settings key changed: {:?}", &key);
        match key {
            Key::DarkMode => self.update_color_scheme(),
            _ => (),
        }
    }

    fn update_color_scheme(&self) {
        let manager = adw::StyleManager::default();
        if !manager.system_supports_color_schemes() {
            let color_scheme = if settings_manager::boolean(Key::DarkMode) {
                adw::ColorScheme::PreferDark
            } else {
                adw::ColorScheme::PreferLight
            };
            manager.set_color_scheme(color_scheme);
        }
    }

    pub fn refresh_data(&self) {
        let fut = clone!(@weak self as this => async move {
            let imp = this.imp();
            // TODO: who is responsbile?
            imp.library.refresh_data().await;
        });
        spawn!(fut);
    }

    fn show_about(&self) {
        let about = adw::AboutWindow::builder()
            .application_icon("Bookx")
            .application_icon(config::APP_ID)
            .license_type(gtk::License::Gpl30)
            .website("https://bookx.adhadse.com/")
            .issue_url("https://github.com/adhadse/Bookx/issues/")
            .version(config::VERSION)
            .translator_credits(String::from("translator-credits"))
            .copyright("© 2022 Anurag Dhadse")
            .developers(vec![String::from("Anurag Dhadse")])
            .designers(vec![String::from("Anurag Dhadse")])
            .build();

        if let Some(window) = self.active_window() {
            about.set_transient_for(Some(&window));
        }

        about.show();
    }

    pub fn show_help(&self) {
        gtk::show_uri(self.active_window().as_ref(), "help:bookx", 0);
    }
}

impl Default for BookxApplication {
    fn default() -> Self {
        gio::Application::default()
            .expect("Could not get default GApplication")
            .downcast()
            .unwrap()
    }
}
