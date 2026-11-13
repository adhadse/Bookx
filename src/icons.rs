//! Bundled icons. They are embedded in the binary and unpacked into the user's cache
//! so they can be found through the regular GTK icon theme lookup.

use relm4::gtk::{self, gdk};

const ICONS: &[(&str, &str)] = &[
    // Funnel for the Filter buttons: drawn for Bookx (CC0 1.0), see NOTICE.md.
    (
        "actions/funnel-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/funnel-symbolic.svg"),
    ),
    (
        "actions/tag-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/tag-symbolic.svg"),
    ),
    (
        "actions/library-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/library-symbolic.svg"),
    ),
    (
        "actions/publisher-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/publisher-symbolic.svg"),
    ),
    // Wikipedia "W": Simple Icons (CC0 1.0), see the header of the SVG for attribution and
    // trademark notice. Wikipedia is a trademark of the Wikimedia Foundation.
    (
        "actions/wikipedia-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/wikipedia-symbolic.svg"),
    ),
    (
        "actions/translate-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/translate-symbolic.svg"),
    ),
    (
        "actions/book-open-variant-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/actions/book-open-variant-symbolic.svg"),
    ),
    (
        "apps/com.adhadse.Bookx.svg",
        include_str!("../data/icons/hicolor/scalable/apps/com.adhadse.Bookx.svg"),
    ),
    (
        "apps/com.adhadse.Bookx-symbolic.svg",
        include_str!("../data/icons/hicolor/scalable/apps/com.adhadse.Bookx-symbolic.svg"),
    ),
];

pub fn install() {
    let Some(dirs) = directories::ProjectDirs::from("com", "adhadse", "Bookx") else {
        return;
    };
    let base = dirs.cache_dir().join("icons");
    for (name, svg) in ICONS {
        let path = base.join("hicolor/scalable").join(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::read_to_string(&path)
            .map(|c| c != *svg)
            .unwrap_or(true)
        {
            let _ = std::fs::write(&path, svg);
        }
    }
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(base);
    }
}
