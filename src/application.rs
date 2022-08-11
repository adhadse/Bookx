use crate::deps::*;
use log::{debug, info};

use glib::clone;
use gtk::subclass::application::GtkApplicationImpl;
use gtk_macros::*;

use adw::prelude::*;
use adw::subclass::prelude::*;

use crate::config;
use crate::ui::BookxWindow;

mod imp {
    use super::*;
    //use glib::WeakRef;
    use once_cell::sync::OnceCell;

    // The basic struct that holds our
    // state and widgets
    #[derive(Debug, Default)]
    pub struct BookxApplication {
        pub window: OnceCell<glib::WeakRef<BookxWindow>>,
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
        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            // Force dark theme
            obj.style_manager()
                .set_color_scheme(adw::ColorScheme::PreferDark);

            // Set up the actions
            obj.setup_actions();
        }
    }

    // Overrides GApplication vfuncs
    impl ApplicationImpl for BookxApplication {
        fn activate(&self, app: &Self::Type) {
            debug!("GtkApplication<BookxApplication>::activate");
            self.parent_activate(app);

            if let Some(window) = self.window.get() {
                let window = window.upgrade().unwrap();
                window.present();
                return;
            }

            let window = BookxWindow::new(app);
            self.window
                .set(window.downgrade())
                .expect("Window already set.");

            app.main_window().present();
        }

        fn startup(&self, app: &Self::Type) {
            debug!("GtkApplication<BookxApplication>::startup");
            self.parent_startup(app);

            // Set icons for shell
            // adw::Window::set_default_icon_name(config::APP_ID);
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
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionMap, gio::ActionGroup;
}

// This is where the member functions of BookxApplication go.
impl BookxApplication {
    pub fn new() -> Self {
        glib::Object::new(&[
            ("application-id", &Some(config::APP_ID)),
            ("flags", &gio::ApplicationFlags::empty()),
            ("resource-base-path", &Some("/com/anuragdhadse/bookx/")),
        ])
        .expect("Application initialization failed...")
    }

    fn main_window(&self) -> BookxWindow {
        self.imp().window.get().unwrap().upgrade().unwrap()
    }

    fn setup_actions(&self) {
        // action! is a macro from gtk_macros
        // that creates a GSimpleAction with a callback.
        // clone! is a macro from glib-rs that allows
        // you to easily handle references in callbacks
        // without refcycles or leaks.
        //
        // When you don't want the callback to keep the
        // Object alive, pass as @weak. Otherwise, pass
        // as @strong. Most of the time you will want
        // to use @weak.

        action!(
            self,
            "about",
            clone!(@weak self as app => move |_, _| {
                app.show_about();
            })
        );

        // Sets up keyboard shortcuts
        self.set_accels_for_action("app.help", &["F1"]);
        self.set_accels_for_action("app.quit", &["<Primary>Q"]);
        self.set_accels_for_action("app.new-window", &["<Primary>N"]);
        self.set_accels_for_action("win.open", &["<Primary>O"]);
        // self.set_accels_for_action("win.print", &["<Primary>P"]);
        self.set_accels_for_action("win.copy", &["<Primary>C"]);
        self.set_accels_for_action("win.show-help-overlay", &["<Primary>question"]);
        self.set_accels_for_action("win.toggle-fullscreen", &["F11"]);
        self.set_accels_for_action("window.close", &["<Primary>W"]);
    }

    fn show_about(&self) {
        // Uncomment and delete the similar code when libadwaita 0.2 comes out of alpha release
        // let about = adw::AboutWindow::builder()
        //     .application_icon("Bookx")
        //     .application_icon(config::APP_ID)
        //     .license_type(gtk::License::Apache20)
        //     .website("https://bookx.anuragdhadse.com/")
        //     .issue_url("https://github.com/adhadse/Bookx/issues/")
        //     .version(config::VERSION)
        //     .translator_credits(String::from("translator-credits"))
        //     .copyright("© 2022 Anurag Dhadse")
        //     .developers(vec![
        //         String::from("Anurag Dhadse")
        //     ])
        //     .designers(vec![
        //         String::from("Anurag Dhadse")
        //     ])
        //     .build();

        // if let Some(window) = self.active_window() {
        //     about.set_transient_for(Some(&window));
        // }

        let about = gtk::AboutDialog::builder()
            .authors(vec![String::from("Anurag Dhadse")])
            .artists(vec![String::from("Anurag Dhadse")])
            .copyright("© 2022 Anurag Dhadse")
            .license_type(gtk::License::Apache20)
            .program_name("Bookx")
            .logo_icon_name(config::APP_ID)
            .version(config::VERSION)
            .build();

        if let Some(window) = self.active_window() {
            about.set_modal(true);
            about.set_transient_for(Some(&window));
        }

        about.show();
    }

    pub fn run(&self) {
        info!("Bookx ({})", config::APP_ID);
        info!("Version: {} ({})", config::VERSION, config::PROFILE);
        info!("Datadir: {}", config::DATADIR);

        ApplicationExtManual::run(self);
    }
}
