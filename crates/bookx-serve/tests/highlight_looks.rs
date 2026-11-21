//! Colour and style of highlights as the devices write them: a KOReader sidecar of the real shape (every
//! colour with every drawer, the pre-2024 layout) and the rows of a Kobo database. Nothing here
//! needs a device or the network.

use bookx_core::devstyle;
use bookx_serve::{kobo, reading};

const ALL_LOOKS: &str = include_str!("fixtures/koreader_all_looks.lua");
const LEGACY: &str = include_str!("fixtures/koreader_legacy_highlights.lua");

const COLORS: [&str; 9] = [
    "red", "orange", "yellow", "green", "olive", "cyan", "blue", "purple", "gray",
];
const DRAWERS: [(&str, &str); 4] = [
    ("lighten", "background"),
    ("underscore", "underline"),
    ("strikeout", "strikethrough"),
    ("invert", "invert"),
];

#[test]
fn every_koreader_colour_with_every_drawer_is_read_as_written() {
    let notes = reading::parse_sidecar(ALL_LOOKS).notes;
    assert_eq!(notes.len(), 36);
    let mut i = 0;
    for c in COLORS {
        for (d, style) in DRAWERS {
            let n = &notes[i];
            i += 1;
            assert_eq!(n.color.as_deref(), Some(c), "colour of {}", n.text);
            assert_eq!(n.style.as_deref(), Some(style), "style of {}", n.text);
            assert!(!n.bookmark);
            // and the words go back to KOReader as they came
            assert_eq!(devstyle::to_koreader_color(n.color.as_deref().unwrap()), c);
            assert_eq!(devstyle::to_koreader_drawer(n.style.as_deref().unwrap()), d);
        }
    }
}

/// Root cause R6: the pre-2024 layout keeps `drawer` in `highlight`, not in `bookmarks`, so the
/// style of every old highlight was dropped.
#[test]
fn the_pre_2024_layout_gets_its_style_from_the_highlight_table() {
    let notes = reading::parse_sidecar(LEGACY).notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].text, "Quote about underscores".to_string());
    assert_eq!(notes[0].style.as_deref(), Some("underline"));
    assert_eq!(
        notes[1].style, None,
        "no highlight entry: the file says nothing"
    );
}

#[test]
fn a_colour_or_drawer_from_a_newer_koreader_is_kept_verbatim() {
    let lua = r##"return {
        ["doc_props"] = { ["title"] = "T" },
        ["annotations"] = {
            [1] = { ["color"] = "teal", ["datetime"] = "2027-01-01 10:00:00", ["drawer"] = "wavy",
                    ["page"] = "/a", ["pos0"] = "/a", ["pos1"] = "/b", ["text"] = "A new kind of highlight" },
            [2] = { ["color"] = "#ff8800", ["datetime"] = "2027-01-01 10:01:00", ["drawer"] = "lighten",
                    ["page"] = "/a", ["pos0"] = "/a", ["pos1"] = "/b", ["text"] = "A custom colour" },
        } }"##;
    let notes = reading::parse_sidecar(lua).notes;
    assert_eq!(notes[0].color.as_deref(), Some("teal"));
    assert_eq!(notes[0].style.as_deref(), Some("wavy"));
    assert_eq!(notes[1].color.as_deref(), Some("#ff8800"));
    // and back out again
    assert_eq!(devstyle::to_koreader_color("teal"), "teal");
    assert_eq!(devstyle::to_koreader_drawer("wavy"), "wavy");
}

#[test]
fn kobo_rows_keep_the_four_colours_and_a_newer_number() {
    let dir = std::env::temp_dir().join(format!("bookx-hl-looks-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("KoboReader.sqlite");
    kobo::create_real_schema_database(&db).unwrap();
    let book = "file:///mnt/onboard/Looks.epub";
    kobo::add_test_highlight(&db, book, "Looks", "Ann", "Quote zero", "").unwrap();
    for (n, text) in [
        (1, "Quote one"),
        (2, "Quote two"),
        (3, "Quote three"),
        (9, "Quote nine"),
    ] {
        kobo::add_test_highlight_colored(&db, book, text, "", n, "highlight").unwrap();
    }
    kobo::add_test_highlight_colored(&db, book, "A bookmark", "", 2, "dogear").unwrap();
    let notes = kobo::read_book_notes(&db).unwrap();
    let look = |text: &str| {
        notes
            .iter()
            .find(|b| b.note.text == text)
            .unwrap_or_else(|| panic!("{text}"))
            .note
            .color
            .clone()
    };
    assert_eq!(look("Quote zero").as_deref(), Some("yellow"));
    assert_eq!(look("Quote one").as_deref(), Some("pink"));
    assert_eq!(look("Quote two").as_deref(), Some("blue"));
    assert_eq!(look("Quote three").as_deref(), Some("green"));
    assert_eq!(look("Quote nine").as_deref(), Some("kobo-9"));
    assert!(
        notes.iter().all(|b| b.note.text != "A bookmark"),
        "a dog-ear is no highlight"
    );
    let _ = std::fs::remove_dir_all(dir);
}
