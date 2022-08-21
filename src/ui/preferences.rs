use adw::PreferencesWindow;
use gtk::prelude::*;

use crate::settings::{settings_manager, Key};

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
        appearance_group.set_visible(!manager.system_supports_color_schemes())
    }

    fn setup_signals(&self) {
        get_widget!(self.builder, gtk::Switch, dark_mode_button);
        settings_manager::bind_property(Key::DarkMode, &dark_mode_button, "active");

        // TODO: add signals for adding folder maybe.
    }
}
