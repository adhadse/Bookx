//! Performance budgets (see `AGENTS.md`): reports, compare and spell check on a 5 MB, 100-chapter book each
//! stay under 500 ms in release builds. Debug builds get `DEBUG_FACTOR` times more room and
//! `BOOKX_PERF_SCALE` scales every budget.

mod common;

use std::time::{Duration, Instant};

use bookx_core::book::OpenBook;
use bookx_editing::compare::{archive_unified_diff, compare_books, CompareOptions};
use bookx_editing::reports::{build_report, ReportOptions};
use bookx_editing::spell::{check_book_spelling, SpellChecker};

const DEBUG_FACTOR: f64 = 30.0;

fn budget(release_ms: u64) -> Duration {
    let scale: f64 = std::env::var("BOOKX_PERF_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let f = if cfg!(debug_assertions) {
        DEBUG_FACTOR
    } else {
        1.0
    };
    Duration::from_micros((release_ms as f64 * 1000.0 * f * scale) as u64)
}

fn check(what: &str, took: Duration, max: Duration) {
    assert!(
        took <= max,
        "{what}: took {took:?}, budget {max:?} (BOOKX_PERF_SCALE adjusts budgets)"
    );
}

const WORDS: &[&str] = &[
    "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "and", "chapter", "of", "text",
    "words", "both", "hello", "world", "a", "tale", "night", "morning", "river", "stone", "garden",
    "window", "silence", "thunder",
];

/// About 5 MB of markup in 100 chapters.
fn big_book() -> OpenBook {
    let mut seed = 12345u64;
    let mut next = move |n: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % n
    };
    let bodies: Vec<String> = (0..100)
        .map(|c| {
            let mut s = format!("<h1 id=\"t\">Chapter {c}</h1>\n");
            while s.len() < 50_000 {
                let words: Vec<&str> = (0..40).map(|_| WORDS[next(WORDS.len())]).collect();
                s.push_str(&format!(
                    "<p class=\"{}\">{} <a href=\"chapter-{}.xhtml#t\">link</a>.</p>\n",
                    ["a", "b", "c"][next(3)],
                    words.join(" "),
                    1 + next(100)
                ));
            }
            s
        })
        .collect();
    let mut b = common::book_from_bodies(&bodies);
    b.archive.set_string(
        "OEBPS/style.css",
        ".a { color: red }\n.b { margin: 0 }\n.c { text-indent: 1em }\n.unused { x: y }\n",
    );
    b.archive.set("OEBPS/images/p.png", common::png(200, 200));
    b
}

fn size(b: &OpenBook) -> usize {
    b.archive.names().iter().map(|n| b.archive.size_of(n)).sum()
}

#[test]
fn reports_on_a_5mb_book() {
    let book = big_book();
    assert!(size(&book) > 5_000_000, "fixture is {} bytes", size(&book));
    let t = Instant::now();
    let r = build_report(&book, &ReportOptions::default());
    check("build_report", t.elapsed(), budget(500));
    assert!(r.words.overall.total > 500_000);
    assert_eq!(r.css.unused_rules, 1);
}

#[test]
fn compare_on_a_5mb_book() {
    let a = big_book();
    let mut b = common::reopen(&a);
    for c in [3, 40, 77] {
        let path = format!("OEBPS/chapter-{c}.xhtml");
        let t = b
            .archive
            .get_string(&path)
            .unwrap()
            .replacen("quick", "slow", 20);
        b.archive.set_string(&path, &t);
    }
    let t = Instant::now();
    let d = compare_books(&a, &b, CompareOptions::default());
    let unified = archive_unified_diff(&a.archive, &b.archive, CompareOptions::default());
    check("compare", t.elapsed(), budget(500));
    assert_eq!(d.changed.len(), 3);
    assert!(unified.contains("-<p"));
}

#[test]
fn spell_check_on_a_5mb_book() {
    let book = big_book();
    let mut checker = SpellChecker::new();
    let dic = format!("{}\n{}\n", WORDS.len() + 2, WORDS.join("\n"));
    checker.add_dictionary("en", "SET UTF-8\n", &dic).unwrap();
    let t = Instant::now();
    let r = check_book_spelling(&book, &checker);
    check("spell check", t.elapsed(), budget(500));
    assert!(r.words_checked > 500_000);
    assert!(r
        .misspellings
        .iter()
        .all(|m| m.file.contains("nav") || m.file.contains("chapter")));
}

/// The editor's live pass runs these on every pause in typing: the change bar and the tag pair at
/// the caret, on a chapter at the editor's size limit (1.5 MB).
#[test]
fn live_editor_pass_on_a_large_chapter() {
    let book = big_book();
    let mut old = String::new();
    while old.len() < 1_500_000 {
        old.push_str(&book.archive.get_string("OEBPS/chapter-1.xhtml").unwrap());
    }
    let new = old.replacen("quick", "slow", 40);
    let t = Instant::now();
    let changes = bookx_editing::compare::changed_lines(&old, &new);
    check("changed_lines", t.elapsed(), budget(250));
    assert!(!changes.is_empty() && changes.len() <= 40);
    let at = new.len() / 2;
    let t = Instant::now();
    let _ = bookx_editing::tags::highlight_pair(&new, at);
    check("highlight_pair", t.elapsed(), budget(250));
}

#[test]
fn validate_a_5mb_book() {
    let book = big_book();
    assert!(size(&book) > 5_000_000, "fixture is {} bytes", size(&book));
    let t = Instant::now();
    let r = bookx_editing::validate::validate(&book.archive);
    check("validate", t.elapsed(), budget(500));
    // the generated chapters have links to existing chapters and fragments: no errors, only the fixture's quirks
    assert_eq!(
        r.summary.errors,
        0,
        "{}",
        bookx_editing::validate::render_text(&r)
    );
    // the same through the bytes entry point, which also scans the zip
    let bytes = book.archive.to_bytes().unwrap();
    let t = Instant::now();
    let r = bookx_editing::validate::validate_bytes(&bytes);
    check("validate_bytes", t.elapsed(), budget(500));
    assert_eq!(r.summary.errors, 0);
}

/// The Reports window builds a tab when it is first shown: every tab on a 5 MB book.
#[test]
fn report_tables_on_a_5mb_book() {
    use bookx_editing::reports_tables::{build_table, TableKind};
    let book = big_book();
    for k in TableKind::ALL {
        let t = Instant::now();
        let table = build_table(&book, k);
        check(k.title(), t.elapsed(), budget(1200));
        assert!(!table.rows.is_empty(), "{} is empty", k.title());
    }
}

/// The spelling panel scans the whole book on a worker thread whenever it is refreshed.
#[test]
fn spelling_panel_scan_on_a_5mb_book() {
    use bookx_editing::spell_session::{collect_sources, scan_sources};
    let book = big_book();
    let mut checker = SpellChecker::new();
    let dic = format!("{}\n{}\n", WORDS.len() + 2, WORDS.join("\n"));
    checker.add_dictionary("en", "SET UTF-8\n", &dic).unwrap();
    let t = Instant::now();
    let (lang, files) = collect_sources(&book);
    let w = scan_sources(&lang, &files, &checker, &Default::default());
    check("spelling scan", t.elapsed(), budget(1000));
    assert!(w.words_checked > 500_000);
}

/// Searching the special character dialog by name walks every named character.
#[test]
fn special_character_search() {
    let t = Instant::now();
    let a = bookx_editing::charinfo::search("letter", 300);
    let b = bookx_editing::charinfo::search("zzzzqq", 300);
    check("character search", t.elapsed(), budget(300));
    assert_eq!(a.len(), 300);
    assert!(b.is_empty());
}

/// The editor's preview sync maps every block of a 1.5 MB chapter to its source after each reload, and the
/// compare view lays out a side-by-side diff of it; both stay interactive.
#[test]
fn preview_map_and_side_by_side_on_a_large_chapter() {
    let book = big_book();
    let ch = book.archive.get_string("OEBPS/chapter-1.xhtml").unwrap();
    let inner = &ch[ch.find("<body>").unwrap() + 6..ch.rfind("</body>").unwrap()];
    let mut src = format!("{}<body>", &ch[..ch.find("<body>").unwrap()]);
    while src.len() < 1_500_000 {
        src.push_str(inner);
    }
    src.push_str("</body></html>");
    let doc = bookx_core::html::parse_html(&src, "OEBPS/chapter-1.xhtml");
    let t = Instant::now();
    let map = bookx_editing::preview_map::map_blocks(&src, &doc);
    check("map_blocks", t.elapsed(), budget(250));
    assert!(map.blocks.len() > 1000);
    let mapped = map.blocks.iter().filter(|b| b.is_some()).count();
    assert!(
        mapped * 10 > map.blocks.len() * 9,
        "{mapped} of {}",
        map.blocks.len()
    );

    let new = src.replacen("quick", "slow", 40);
    let t = Instant::now();
    let d = bookx_editing::compare::side_by_side(&src, &new, 3);
    check("side_by_side", t.elapsed(), budget(400));
    assert!(d.changed_rows > 0 && d.changed_rows <= 40);

    let t = Instant::now();
    let l = bookx_editing::live_css::inspect(
        &book.archive,
        "OEBPS/chapter-1.xhtml",
        &src,
        src.len() / 2,
    );
    check("live_css::inspect", t.elapsed(), budget(500));
    assert!(l.is_ok());
}
