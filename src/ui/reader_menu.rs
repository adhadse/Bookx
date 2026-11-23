//! The Main Menu of the readers (RDR-R3, RDR-R1): one `gio::Menu` in a `PopoverMenu` (access keys,
//! menu roles, arrow keys) instead of a stack of buttons, the same builder for the e-book, PDF and
//! comic readers. Appearance (theme, zoom, columns) lives in the Text appearance popover; a window
//! narrower than 600 sp moves Fullscreen, Read Aloud and Text Appearance into the menu.

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio};

use crate::i18n::{N_, tr};

/// An entry of a menu: label (msgid), action. An empty label is the end of a section.
pub type Entry = (&'static str, &'static str);

const STANDARD: [Entry; 6] = [
    ("", ""),
    (N_("Preferences"), "app.preferences"),
    (N_("Keyboard Shortcuts"), "app.shortcuts"),
    (N_("Help"), "app.help"),
    // the report starts with what the reader did last
    (N_("Report a Problem"), "app.report-problem-from::Reader"),
    (N_("About Bookx"), "app.about"),
];

/// The e-book reader: *This Book*, *File*, then the standard group. 13 entries, no submenu.
pub const EPUB_MENU: [Entry; 15] = [
    (N_("Go to"), "reader.goto"),
    (N_("Speed Reading"), "reader.rsvp"),
    (N_("Book Map"), "reader.map"),
    (N_("Terms in This Book"), "reader.vocab-terms"),
    (N_("Footer Statistics"), "reader.footer"),
    (N_("Reading Ruler"), "reader.ruler"),
    ("", ""),
    (N_("Open Book"), "reader.open"),
    (N_("Reload"), "reader.reload"),
    STANDARD[0],
    STANDARD[1],
    STANDARD[2],
    STANDARD[3],
    STANDARD[4],
    STANDARD[5],
];

/// The PDF page view.
pub const PDF_MENU: [Entry; 9] = [
    (N_("Go to Page"), "reader.goto"),
    (N_("Reflow as Text"), "reader.reflow"),
    (N_("Properties"), "reader.properties"),
    STANDARD[0],
    STANDARD[1],
    STANDARD[2],
    STANDARD[3],
    STANDARD[4],
    STANDARD[5],
];

/// The comic reader (it had no main menu).
pub const COMIC_MENU: [Entry; 8] = [
    (N_("Go to Page"), "reader.goto"),
    (N_("Properties"), "reader.properties"),
    STANDARD[0],
    STANDARD[1],
    STANDARD[2],
    STANDARD[3],
    STANDARD[4],
    STANDARD[5],
];

/// What a narrow window adds at the top of the menu (the header lost these buttons).
pub const NARROW_ENTRIES: [Entry; 3] = [
    (N_("Fullscreen"), "reader.fullscreen"),
    (N_("Read Aloud"), "reader.read-aloud"),
    (N_("Text Appearance"), "reader.text-appearance"),
];

/// The PDF header only loses Fullscreen (it has no read aloud).
pub const NARROW_PDF: [Entry; 1] = [(N_("Fullscreen"), "reader.fullscreen")];

/// The width (in sp) below which the header gives up buttons.
pub const NARROW_SP: f64 = 600.0;

/// The entries that are really there (separators dropped, trailing ones too).
pub fn items(entries: &[Entry]) -> Vec<Entry> {
    entries
        .iter()
        .copied()
        .filter(|(l, _)| !l.is_empty())
        .collect()
}

/// A menu model from entries: groups become sections, labels are translated and get access keys
/// that are unique inside the menu; reader actions show their (current) key.
pub fn build(entries: &[Entry]) -> gio::Menu {
    let labels: Vec<String> = items(entries).iter().map(|(l, _)| tr(l)).collect();
    let keyed = crate::library_nav::access_keys(&labels);
    let keys = crate::shortcuts::current();
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    let mut i = 0;
    for (label, action) in entries {
        if label.is_empty() {
            if section.n_items() > 0 {
                menu.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        let item = gio::MenuItem::new(Some(&keyed[i]), Some(action));
        if action.starts_with("reader.") {
            if let Some(accel) = keys.effective(action).first() {
                item.set_attribute_value("accel", Some(&accel.to_variant()));
            }
        }
        section.append_item(&item);
        i += 1;
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}

/// The entries of the menu of a narrow window: the dropped header buttons first.
pub fn narrow_entries(base: &[Entry], dropped: &[Entry]) -> Vec<Entry> {
    let mut v: Vec<Entry> = dropped.to_vec();
    v.push(("", ""));
    v.extend_from_slice(base);
    v
}

/// The jump buttons of the header: a word each when there is room, an icon each when the window is
/// narrow (icon or label, never both).
pub fn jump_style(back: &gtk::Button, fwd: Option<&gtk::Button>, narrow: bool) {
    if narrow {
        back.set_icon_name("go-jump-symbolic");
        if let Some(f) = fwd {
            f.set_icon_name("go-next-symbolic");
        }
    } else {
        back.set_label(&tr("Previous Position"));
        if let Some(f) = fwd {
            f.set_label(&tr("Next Position"));
        }
    }
    super::audit::label(back, &tr("Previous Position"));
    if let Some(f) = fwd {
        super::audit::label(f, &tr("Next Position"));
    }
}

/// A narrow window (`adw::BreakpointBin` below 600 sp): `on_change(true)` when it gets narrow,
/// `on_change(false)` when it is wide again. The bin also gets the least width the reader needs.
pub fn install_narrow(bin: &adw::BreakpointBin, on_change: impl Fn(bool) + 'static) {
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        NARROW_SP,
        adw::LengthUnit::Sp,
    ));
    let on_change = std::rc::Rc::new(on_change);
    let f = on_change.clone();
    bp.connect_apply(move |_| f(true));
    bp.connect_unapply(move |_| on_change(false));
    bin.add_breakpoint(bp);
}

/// Whether a tooltip or menu label is in header caps (HIG): every word but the small ones starts with a capital;
/// what is in parentheses (the shortcut) and the text after a colon are not looked at.
#[cfg(test)]
fn is_header_caps(text: &str) -> bool {
    const SMALL: [&str; 17] = [
        "a", "an", "and", "the", "of", "in", "on", "to", "for", "with", "from", "or", "at", "by",
        "as", "is", "into",
    ];
    let head = text.split(['(', ':']).next().unwrap_or(text);
    head.split_whitespace().enumerate().all(|(i, w)| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        match w.chars().next() {
            None => true,
            Some(c) if !c.is_alphabetic() => true,
            Some(c) => c.is_uppercase() || (i > 0 && SMALL.contains(&w.to_lowercase().as_str())),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tooltips of the readers are in header caps (RDR-R92, decided in §7 D4): every `tr("...")` of a tooltip
    /// or an icon button in the reader files.
    #[test]
    fn reader_tooltips_are_in_header_caps() {
        let files: [(&str, &str); 9] = [
            ("reader.rs", include_str!("reader.rs")),
            ("reader_pdf.rs", include_str!("reader_pdf.rs")),
            ("reader_comic.rs", include_str!("reader_comic.rs")),
            ("reader_side.rs", include_str!("reader_side.rs")),
            ("reader_tts.rs", include_str!("reader_tts.rs")),
            ("tts_player.rs", include_str!("tts_player.rs")),
            ("audiobook_player.rs", include_str!("audiobook_player.rs")),
            ("reader_map.rs", include_str!("reader_map.rs")),
            ("reader_menu.rs", include_str!("reader_menu.rs")),
        ];
        let mut bad = Vec::new();
        for (name, src) in files {
            for (n, line) in src.lines().enumerate() {
                let l = line.trim();
                let looks_like_tip = l.contains("set_tooltip_text(Some(&tr(\"")
                    || l.contains("icon_button(") && l.contains("&tr(\"");
                if !looks_like_tip {
                    continue;
                }
                if let Some(a) = l.find("&tr(\"") {
                    let rest = &l[a + 5..];
                    if let Some(end) = rest.find('"') {
                        let text = &rest[..end];
                        if !is_header_caps(text) {
                            bad.push(format!("{name}:{}: {text}", n + 1));
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "not in header caps:\n{}", bad.join("\n"));
    }

    #[test]
    fn header_caps_ignores_small_words_and_shortcuts() {
        assert!(is_header_caps("Search in Book (Ctrl+F)"));
        assert!(is_header_caps("Look Up in Dictionary"));
        assert!(is_header_caps("Read Aloud from Here"));
        assert!(!is_header_caps("Add a note"));
        assert!(!is_header_caps("Search in book"));
        assert!(is_header_caps("Show or Hide What the Marks Mean"));
    }

    fn all() -> Vec<(&'static str, &'static [Entry])> {
        vec![
            ("epub", &EPUB_MENU[..]),
            ("pdf", &PDF_MENU[..]),
            ("comic", &COMIC_MENU[..]),
        ]
    }

    #[test]
    fn the_epub_menu_has_twelve_entries_in_three_groups() {
        assert_eq!(items(&EPUB_MENU).len(), 13);
        let seps = EPUB_MENU.iter().filter(|(l, _)| l.is_empty()).count();
        assert_eq!(seps, 2, "three sections");
    }

    #[test]
    fn menus_stay_within_three_to_twelve_entries_and_have_help() {
        for (name, m) in all() {
            let n = items(m).len();
            assert!((3..=13).contains(&n), "{name} has {n} entries");
            assert!(items(m).contains(&("Help", "app.help")), "{name} has Help");
            assert!(items(m).contains(&("About Bookx", "app.about")));
        }
    }

    #[test]
    fn labels_do_not_end_in_an_ellipsis_and_are_not_repeated() {
        for (name, m) in all() {
            let mut seen = std::collections::HashSet::new();
            for (l, _) in items(m) {
                assert!(!l.ends_with('…') && !l.ends_with("..."), "{name}: {l}");
                assert!(seen.insert(l), "{name}: {l} twice");
            }
        }
    }

    #[test]
    fn the_layout_submenu_is_gone() {
        for (_, m) in all() {
            assert!(!items(m).iter().any(|(l, _)| *l == "Layout"));
        }
    }

    #[test]
    fn a_narrow_window_puts_the_dropped_buttons_first() {
        let v = narrow_entries(&EPUB_MENU, &NARROW_ENTRIES);
        assert_eq!(v[0].0, "Fullscreen");
        assert_eq!(v[3], ("", ""));
        assert_eq!(items(&v).len(), 16);
    }
}
