//! The repair of highlights stored with the wrong look, over a seeded library and device files
//! built like the owner's: a library where every highlight is yellow and plain (what every import
//! gave before the device looks were understood), a KOReader folder and a Kobo database that know
//! better.

use std::path::{Path, PathBuf};

use bookx_core::convert::{Chapter, create_epub};
use bookx_core::library::{AddOutcome, Highlight, Library};
use bookx_core::opf::Metadata;
use bookx_serve::look_repair as repair;
use chrono::{TimeZone, Utc};

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("bookx-look-repair-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const TITLE: &str = "Looks of Highlights";

fn library(dir: &Path) -> (Library, u64) {
    let md = Metadata {
        title: TITLE.into(),
        authors: vec!["Ann Author".into()],
        ..Default::default()
    };
    let ch = Chapter {
        title: "One".into(),
        body: "<p>Some text.</p>".into(),
    };
    let p = dir.join("book.epub");
    std::fs::write(&p, create_epub(&md, &[ch], None).to_bytes().unwrap()).unwrap();
    let mut lib = Library::in_memory(&dir.join("lib")).unwrap();
    let id = match lib.add_path(&p, true).unwrap() {
        AddOutcome::Added(id) | AddOutcome::Duplicate(id) => id,
    };
    (lib, id)
}

/// The 36 highlights of the KOReader fixture, all stored the way the old code stored them.
fn seed_wrong(lib: &mut Library, id: u64) {
    let rows = repair::truths_from_sidecar(
        include_str!("fixtures/koreader_all_looks.lua"),
        "Looks.sdr",
        "KOReader sidecar",
    );
    let book = lib.get_mut(id).unwrap();
    for (i, r) in rows.iter().enumerate() {
        book.highlights.push(Highlight {
            id: format!("bx-{i}"),
            spine: 0,
            start: i as i32,
            end: i as i32 + 3,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: r.text.clone(),
            created: Utc.timestamp_opt(r.created.unwrap(), 0).unwrap(),
        });
    }
}

fn device_folder(dir: &Path) -> PathBuf {
    let dev = dir.join("device");
    std::fs::create_dir_all(dev.join("Looks.sdr")).unwrap();
    std::fs::write(
        dev.join("Looks.sdr/metadata.epub.lua"),
        include_str!("fixtures/koreader_all_looks.lua"),
    )
    .unwrap();
    dev
}

fn looks(lib: &Library, id: u64) -> Vec<(String, String)> {
    lib.get(id)
        .unwrap()
        .highlights
        .iter()
        .map(|h| (h.color.clone(), h.style.clone()))
        .collect()
}

#[test]
fn a_library_where_everything_is_yellow_gets_the_looks_of_the_device_once() {
    let d = tmp();
    let (mut lib, id) = library(&d);
    seed_wrong(&mut lib, id);
    assert!(
        looks(&lib, id)
            .iter()
            .all(|l| l == &("yellow".into(), "".into()))
    );
    let truths = repair::truths_from_folder(&device_folder(&d));
    assert_eq!(truths.len(), 36);

    let plan = repair::plan(&lib.books, &Default::default(), &truths);
    // yellow with the plain drawer is what the library already says: 1 of the 36 is right
    assert_eq!(plan.report.right, 1);
    assert_eq!(plan.report.fixed, 35);
    assert_eq!(plan.report.no_highlight, 0, "nothing is created");
    // a dry run changes nothing
    assert!(
        looks(&lib, id)
            .iter()
            .all(|l| l == &("yellow".into(), "".into()))
    );

    assert_eq!(repair::apply(&mut lib, &plan).unwrap(), 35);
    let want: Vec<(&str, &str)> = {
        let colors = [
            "red", "orange", "yellow", "green", "olive", "cyan", "blue", "purple", "gray",
        ];
        let styles = ["background", "underline", "strikethrough", "invert"];
        colors
            .iter()
            .flat_map(|c| styles.iter().map(move |s| (*c, *s)))
            .collect()
    };
    for (got, want) in looks(&lib, id).iter().zip(want) {
        assert_eq!(
            (
                got.0.as_str(),
                if got.1.is_empty() {
                    "background"
                } else {
                    got.1.as_str()
                }
            ),
            want
        );
    }
    assert_eq!(
        lib.get(id).unwrap().highlights.len(),
        36,
        "nothing was created or deleted"
    );

    // idempotent: the second run finds nothing to do
    let again = repair::plan(
        &lib.books,
        &bookx_core::devstyle::DeviceLooks::load(&lib.root),
        &truths,
    );
    assert!(again.fixes.is_empty(), "{:?}", again.report);
    assert_eq!(repair::apply(&mut lib, &again).unwrap(), 0);

    // one undo step brings every look back
    let before = lib.history().len();
    assert!(lib.undo_last().unwrap().is_some());
    assert!(lib.history().len() >= before);
    assert!(
        looks(&lib, id)
            .iter()
            .all(|l| l == &("yellow".into(), "".into()))
    );
    // an undone repair can be run again: nothing was left behind that says "the reader changed it"
    let ledger = bookx_core::devstyle::DeviceLooks::load(&lib.root);
    let again = repair::plan(&lib.books, &ledger, &truths);
    assert_eq!(again.report.fixed, 35);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_look_the_reader_chose_survives_the_repair() {
    let d = tmp();
    let (mut lib, id) = library(&d);
    seed_wrong(&mut lib, id);
    // the reader recoloured the second highlight (red, plain) in Bookx
    lib.get_mut(id).unwrap().highlights[1].color = "blue".into();
    let truths = repair::truths_from_folder(&device_folder(&d));
    let plan = repair::plan(&lib.books, &Default::default(), &truths);
    assert_eq!(plan.report.kept, 1);
    repair::apply(&mut lib, &plan).unwrap();
    assert_eq!(looks(&lib, id)[1].0, "blue");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_one_time_pass_waits_for_a_source_then_never_runs_again() {
    let d = tmp();
    let (mut lib, id) = library(&d);
    seed_wrong(&mut lib, id);
    // no device: nothing to learn from, and the marker is not written
    assert!(repair::one_time(&mut lib, &[]).is_none());
    assert!(!repair::marker(&lib.root).exists());
    // a device is there: corrected once, marker written
    let truths = repair::truths_from_folder(&device_folder(&d));
    let r = repair::one_time(&mut lib, &truths).expect("changed something");
    assert_eq!(r.fixed, 35);
    assert!(repair::marker(&lib.root).exists());
    // a library the marker says was done is left alone, even if highlights were reset
    for h in &mut lib.get_mut(id).unwrap().highlights {
        h.color = "yellow".into();
        h.style = String::new();
    }
    assert!(repair::one_time(&mut lib, &truths).is_none());
    assert!(
        looks(&lib, id)
            .iter()
            .all(|l| l == &("yellow".into(), "".into()))
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_kobo_database_repairs_its_four_colours() {
    let d = tmp();
    let (mut lib, id) = library(&d);
    let db = d.join("KoboReader.sqlite");
    bookx_serve::kobo::create_real_schema_database(&db).unwrap();
    let book = "file:///mnt/onboard/Looks.epub";
    bookx_serve::kobo::add_test_highlight(
        &db,
        book,
        TITLE,
        "Ann Author",
        "Quote zero is here in the text",
        "",
    )
    .unwrap();
    for (n, text) in [
        (1, "Quote one is here in the text"),
        (2, "Quote two is here in the text"),
        (3, "Quote three is here in the text"),
    ] {
        bookx_serve::kobo::add_test_highlight_colored(&db, book, text, "", n, "highlight").unwrap();
    }
    let b = lib.get_mut(id).unwrap();
    for (i, t) in ["zero", "one", "two", "three"].iter().enumerate() {
        b.highlights.push(Highlight {
            id: format!("k{i}"),
            spine: 0,
            start: i as i32,
            end: i as i32 + 3,
            color: "yellow".into(),
            style: String::new(),
            note: String::new(),
            text: format!("Quote {t} is here in the text"),
            created: Utc::now(),
        });
    }
    let truths = repair::truths_from_path(&db);
    assert_eq!(truths.len(), 4);
    let plan = repair::plan(&lib.books, &Default::default(), &truths);
    repair::apply(&mut lib, &plan).unwrap();
    let got: Vec<String> = looks(&lib, id).into_iter().map(|l| l.0).collect();
    assert_eq!(got, ["yellow", "pink", "blue", "green"]);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_device_folder_is_read_without_following_links_or_huge_files() {
    let d = tmp();
    let dev = device_folder(&d);
    // a link out of the folder to a sidecar that must not be read
    let outside = d.join("outside.sdr");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("metadata.epub.lua"),
        include_str!("fixtures/koreader_all_looks.lua"),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, dev.join("Linked.sdr")).unwrap();
    let truths = repair::truths_from_folder(&dev);
    assert_eq!(truths.len(), 36, "the linked folder is not followed");
    // garbage is not a sidecar and not an error
    std::fs::create_dir_all(dev.join("Junk.sdr")).unwrap();
    std::fs::write(
        dev.join("Junk.sdr/metadata.epub.lua"),
        b"\xff\xfe not lua {{{",
    )
    .unwrap();
    assert_eq!(repair::truths_from_folder(&dev).len(), 36);
    let _ = std::fs::remove_dir_all(d);
}
