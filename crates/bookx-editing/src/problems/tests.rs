use super::*;
use crate::testepub::*;
use crate::validate::tests_extra::xfaults;
use bookx_core::book::OpenBook;

fn book(files: &Files) -> OpenBook {
    OpenBook::from_archive(to_archive(files)).expect("book")
}

/// The fixtures name no author, which is a note of its own.
fn minus_author(mut v: Vec<Problem>) -> Vec<Problem> {
    v.retain(|p| p.id != "CHK-002");
    v
}

fn opts() -> CheckOptions {
    CheckOptions::default()
}

fn ids(b: &OpenBook, o: &CheckOptions) -> Vec<String> {
    let mut v: Vec<String> = collect(&b.archive, o).into_iter().map(|p| p.id).collect();
    v.sort();
    v.dedup();
    v
}

fn p(id: &str, sev: Severity, file: &str, text: &str) -> Problem {
    Problem {
        id: id.into(),
        severity: sev,
        file: file.into(),
        line: Some(1),
        col: None,
        text: text.into(),
    }
}

#[test]
fn clean_books_have_no_problems() {
    for f in [clean_epub3(), clean_epub2()] {
        let b = book(&f);
        assert!(minus_author(collect(&b.archive, &opts())).is_empty());
        let all = collect(&b.archive, &opts());
        assert_eq!(all.len(), 1);
        assert_eq!(
            (all[0].id.as_str(), all[0].severity),
            ("CHK-002", Severity::Info)
        );
    }
}

#[test]
fn quick_checks_are_merged_without_duplicates() {
    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "href=\"ch1.xhtml\"",
        "href=\"gone.xhtml\"",
    );
    let b = book(&f);
    let all = collect(&b.archive, &opts());
    let broken: Vec<_> = all.iter().filter(|p| p.id == "RSC-007").collect();
    assert_eq!(broken.len(), 1, "{all:?}");
    // a book without an author gets the editor-only note
    let mut g = clean_epub3();
    rep(
        &mut g,
        OPF,
        "<dc:title>",
        "<dc:creator>x</dc:creator><dc:title>",
    );
    let b = book(&g);
    assert!(!ids(&b, &opts()).contains(&"CHK-002".to_string()));
    let mut h = clean_epub3();
    rep(
        &mut h,
        OPF,
        "<dc:identifier id=\"uid\">",
        "<dc:identifier id=\"other\">",
    );
    let b = book(&h);
    let got = collect(&b.archive, &opts());
    assert_eq!(
        got.iter().filter(|p| p.id == "OPF-030").count(),
        1,
        "{got:?}"
    );
}

#[test]
fn problems_are_sorted_errors_first() {
    let mut f = clean_epub3();
    set(&mut f, "OEBPS/extra.txt", "x");
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "href=\"ch1.xhtml\"",
        "href=\"gone.xhtml\"",
    );
    let b = book(&f);
    let all = collect(&b.archive, &opts());
    assert!(all.len() >= 2);
    assert_eq!(all[0].severity, Severity::Error);
    let sevs: Vec<_> = all.iter().map(|p| p.severity).collect();
    let mut sorted = sevs.clone();
    sorted.sort();
    assert_eq!(sevs, sorted);
}

#[test]
fn usage_findings_appear_only_with_the_accessibility_option() {
    let b = book(&clean_epub3());
    assert_eq!(ids(&b, &opts()), vec!["CHK-002"]);
    let on = ids(
        &b,
        &CheckOptions {
            accessibility: true,
        },
    );
    assert!(on.contains(&"ACC-903".to_string()), "{on:?}");
}

#[test]
fn ignored_kinds_are_hidden_except_those_that_cannot_be_skipped() {
    let list = vec![
        p("OPF-003", Severity::Warning, "a", "x"),
        p("OPF-003", Severity::Warning, "b", "y"),
        p("RSC-016", Severity::Error, "c", "z"),
        p("HTM-907", Severity::Error, "d", "w"),
    ];
    let ignored: BTreeSet<String> = ["OPF-003", "RSC-016"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (kept, hidden) = without_ignored(list, &ignored);
    assert_eq!(hidden, 2);
    assert_eq!(
        kept.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["RSC-016", "HTM-907"]
    );
    assert!(!can_ignore("RSC-016") && can_ignore("OPF-003"));
    assert!(title("OPF-003").contains("manifest"));
    assert_eq!(title("ZZZ-1"), "ZZZ-1");
}

#[test]
fn grouping_counts_and_steps() {
    let list = vec![
        p("A", Severity::Error, "b.xhtml", "1"),
        p("B", Severity::Warning, "a.xhtml", "2"),
        p("C", Severity::Error, "b.xhtml", "3"),
        p("D", Severity::Info, "c.xhtml", "4"),
    ];
    let g = group_by_file(&list);
    assert_eq!(g[0], ("b.xhtml".to_string(), vec![0, 2]));
    assert_eq!(g[1].0, "a.xhtml");
    assert_eq!(g[2].0, "c.xhtml");
    assert_eq!(counts(&list), (2, 1, 1));
    assert_eq!(step(0, None, true), None);
    assert_eq!(step(3, None, true), Some(0));
    assert_eq!(step(3, None, false), Some(2));
    assert_eq!(step(3, Some(2), true), Some(0));
    assert_eq!(step(3, Some(0), false), Some(2));
    assert_eq!(step(3, Some(1), true), Some(2));
}

#[test]
fn the_selection_survives_a_re_run() {
    let old = p("HTM-907", Severity::Error, "x.xhtml", "img one");
    let mut new = vec![
        p("OPF-003", Severity::Warning, "x.xhtml", "other"),
        p("HTM-907", Severity::Error, "x.xhtml", "img one"),
    ];
    new[1].line = Some(40);
    assert_eq!(reselect(&old, &new), Some(1));
    new[1].text = "img two".into();
    assert_eq!(reselect(&old, &new), Some(1), "same kind in the same file");
    new[1].file = "y.xhtml".into();
    assert_eq!(reselect(&old, &new), None);
    assert_eq!(stale_keys(&new, "x.xhtml").len(), 1);
}

#[test]
fn fix_buttons_match_the_help_texts() {
    for id in help::ids() {
        let h = help(id).unwrap();
        assert_eq!(
            h.how.contains("Use Fix"),
            fix_label(id).is_some(),
            "{id}: help says {:?}, fix_label is {:?}",
            h.how,
            fix_label(id)
        );
    }
}

fn fixed(id: &str, files: &Files, a11y: bool) -> (OpenBook, Vec<Problem>, FixOutcome) {
    let o = CheckOptions {
        accessibility: a11y,
    };
    let mut b = book(files);
    let before = collect(&b.archive, &o);
    assert!(
        before.iter().any(|p| p.id == id),
        "{id} not reported: {before:?}"
    );
    let out = apply_fix(&mut b, id, &Scope::Kind, &o).unwrap();
    assert!(out.changed, "{id}: nothing changed");
    let after = collect(&b.archive, &o);
    (b, after, out)
}

#[test]
fn every_offered_fix_removes_its_problem_without_adding_errors() {
    let mut tried = BTreeSet::new();
    for fault in faults() {
        // the RSC-007 fixture links to a file that exists nowhere, which no fix can repair (the
        // case-mismatch fixture of BKX-022 covers the repair)
        if fix_label(fault.id).is_none() || fault.id == "RSC-007" {
            continue;
        }
        let mut files = if fault.version == 2 {
            clean_epub2()
        } else {
            clean_epub3()
        };
        (fault.apply)(&mut files);
        let Ok(b0) = OpenBook::from_archive(to_archive(&files)) else {
            continue;
        };
        let errors_before = collect(&b0.archive, &opts())
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count();
        let (_, after, _) = fixed(fault.id, &files, false);
        assert!(
            !after.iter().any(|p| p.id == fault.id),
            "{} still reported: {after:?}",
            fault.id
        );
        let errors_after = after
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count();
        assert!(errors_after <= errors_before, "{}: {after:?}", fault.id);
        tried.insert(fault.id);
    }
    for fault in xfaults() {
        if fix_label(fault.id).is_none() {
            continue;
        }
        let mut files = if fault.version == 2 {
            clean_epub2()
        } else {
            clean_epub3()
        };
        (fault.apply)(&mut files);
        let (_, after, _) = fixed(fault.id, &files, fault.a11y);
        assert!(
            !after.iter().any(|p| p.id == fault.id),
            "{} still reported: {after:?}",
            fault.id
        );
        tried.insert(fault.id);
    }
    eprintln!("fixes tried: {tried:?}");
}

fn text(b: &OpenBook, path: &str) -> String {
    b.archive.get_string(path).unwrap()
}

fn one(id: &str, files: &Files, scope: Scope) -> OpenBook {
    let mut b = book(files);
    apply_fix(&mut b, id, &scope, &opts()).unwrap();
    b
}

#[test]
fn entities_are_fixed_the_way_the_help_says() {
    let mut f = clean_epub3();
    rep(&mut f, "OEBPS/ch2.xhtml", "More", "a&nbsp;b &eacute;");
    let b = one("BKX-004", &f, Scope::Kind);
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("a&#160;b &#233;"));

    let mut f = clean_epub3();
    rep(&mut f, "OEBPS/ch2.xhtml", "More", "AT&T &foo; &");
    let b = one("BKX-005", &f, Scope::Kind);
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("AT&amp;T &amp;foo; &amp;"));
    assert!(!collect(&b.archive, &opts())
        .iter()
        .any(|p| p.severity == Severity::Error));

    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "<!DOCTYPE html>",
        "<!DOCTYPE html [\n<!ENTITY pub \"The Publisher\">\n]>",
    );
    rep(&mut f, "OEBPS/ch2.xhtml", "More", "By &pub;");
    let b = one("BKX-006", &f, Scope::Kind);
    let t = text(&b, "OEBPS/ch2.xhtml");
    assert!(
        t.contains("By The Publisher") && !t.contains("<!ENTITY"),
        "{t}"
    );
}

#[test]
fn encodings_are_converted() {
    let mut f = clean_epub3();
    let mut bytes = text_of(&f, "OEBPS/ch2.xhtml").into_bytes();
    let at = bytes.windows(4).position(|w| w == b"More").unwrap();
    bytes[at] = 0xE9; // é in Latin-1
    set(&mut f, "OEBPS/ch2.xhtml", bytes);
    let b = one("BKX-002", &f, Scope::Kind);
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("éore"));

    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "encoding=\"UTF-8\"",
        "encoding=\"ISO-8859-1\"",
    );
    let mut bytes = text_of(&f, "OEBPS/ch2.xhtml").into_bytes();
    let at = bytes.windows(4).position(|w| w == b"More").unwrap();
    bytes[at] = 0xE9;
    set(&mut f, "OEBPS/ch2.xhtml", bytes);
    let b = one("BKX-003", &f, Scope::Kind);
    let t = text(&b, "OEBPS/ch2.xhtml");
    assert!(
        t.contains("encoding=\"UTF-8\"") && t.contains("éore"),
        "{t}"
    );
    assert!(!ids(&b, &opts()).contains(&"BKX-003".to_string()));
}

fn text_of(f: &Files, name: &str) -> String {
    get_string(f, name)
}

#[test]
fn ids_are_renamed_with_their_links() {
    let mut f = clean_epub3();
    rep(&mut f, "OEBPS/ch2.xhtml", "id=\"sec\"", "id=\"2 sec\"");
    rep(
        &mut f,
        "OEBPS/ch1.xhtml",
        "ch2.xhtml#sec",
        "ch2.xhtml#2%20sec",
    );
    rep(
        &mut f,
        "OEBPS/nav.xhtml",
        "ch2.xhtml\">Two",
        "ch2.xhtml#2%20sec\">Two",
    );
    let b = one("BKX-008", &f, Scope::Kind);
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("id=\"id2_sec\""));
    assert!(text(&b, "OEBPS/ch1.xhtml").contains("ch2.xhtml#id2_sec"));
    assert!(text(&b, "OEBPS/nav.xhtml").contains("ch2.xhtml#id2_sec"));
    let left: Vec<_> = collect(&b.archive, &opts())
        .into_iter()
        .filter(|p| p.id != "CHK-002")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn bare_text_is_wrapped_with_its_inline_neighbours() {
    let out = fix::wrap_bare_text(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body>\n  Loose <b>bold</b> words\n<p>kept</p>\n  tail\n</body></html>",
    )
    .unwrap();
    assert!(out.contains("<p>Loose <b>bold</b> words</p>"), "{out}");
    assert!(
        out.contains("<p>kept</p>") && out.contains("<p>tail</p>"),
        "{out}"
    );
    assert!(fix::wrap_bare_text(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>x</p></body></html>"
    )
    .is_none());
    assert!(fix::wrap_bare_text("<html><body>broken").is_none());
}

#[test]
fn images_links_covers_and_the_reading_order() {
    let mut f = clean_epub3();
    add_item(
        &mut f,
        "c",
        "images/c.jpg",
        "image/jpeg",
        "",
        Some(crate::validate::tests_extra::CMYK_JPEG),
    );
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "</section>",
        "<img src=\"images/c.jpg\" alt=\"c\"/></section>",
    );
    let b = one("BKX-016", &f, Scope::Kind);
    assert_eq!(
        crate::validate::jpeg_components(b.archive.get("OEBPS/images/c.jpg").unwrap()),
        Some(3)
    );

    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "href=\"ch1.xhtml\"",
        "href=\"ch 1.xhtml\"",
    );
    let b = one("BKX-019", &f, Scope::Kind);
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("href=\"ch%201.xhtml\""));

    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "href=\"ch1.xhtml\"",
        "href=\"CH1.xhtml#top\"",
    );
    let b = one("BKX-022", &f, Scope::Kind);
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("href=\"ch1.xhtml#top\""));

    let mut f = clean_epub3();
    let doc = xhtml3("<title>Three</title>", "<p>Three</p>");
    add_item(
        &mut f,
        "ch3",
        "ch3.xhtml",
        "application/xhtml+xml",
        "",
        Some(doc.as_bytes()),
    );
    let b = one("BKX-025", &f, Scope::Kind);
    assert!(b.spine_paths().contains(&"OEBPS/ch3.xhtml".to_string()));

    let mut f = clean_epub2();
    rep(
        &mut f,
        OPF,
        "</metadata>",
        "<meta name=\"cover\" content=\"cover\"/></metadata>",
    );
    let b = one("BKX-028", &f, Scope::Kind);
    assert_eq!(text(&b, OPF).matches("name=\"cover\"").count(), 1);
}

#[test]
fn accessibility_fixes_add_roles_and_metadata() {
    let o = CheckOptions {
        accessibility: true,
    };
    let mut b = book(&clean_epub3());
    for id in ["ACC-902", "ACC-903", "ACC-904"] {
        if collect(&b.archive, &o).iter().any(|p| p.id == id) {
            apply_fix(&mut b, id, &Scope::Kind, &o).unwrap();
        }
    }
    let left = minus_author(collect(&b.archive, &o));
    assert!(left.is_empty(), "{left:?}");
    assert!(text(&b, "OEBPS/ch1.xhtml").contains("role=\"doc-chapter\""));
}

#[test]
fn a_fix_for_one_file_leaves_the_others_alone() {
    let mut f = clean_epub3();
    rep(&mut f, "OEBPS/ch1.xhtml", "Text", "a&nbsp;b");
    rep(&mut f, "OEBPS/ch2.xhtml", "More", "c&nbsp;d");
    let b = one("BKX-004", &f, Scope::File("OEBPS/ch1.xhtml".into()));
    assert!(text(&b, "OEBPS/ch1.xhtml").contains("a&#160;b"));
    assert!(text(&b, "OEBPS/ch2.xhtml").contains("c&nbsp;d"));
    let mut b2 = book(&f);
    let out = apply_fix(
        &mut b2,
        "BKX-004",
        &Scope::File("OEBPS/nav.xhtml".into()),
        &opts(),
    )
    .unwrap();
    assert!(!out.changed);
    assert!(
        !apply_fix(&mut b2, "ZZZ-1", &Scope::Kind, &opts())
            .unwrap()
            .changed
    );
}

#[test]
fn unsafe_file_names_are_made_safe() {
    assert_eq!(fix::safe_name("my file?.txt"), "my_file_.txt");
    assert_eq!(fix::safe_name("trail."), "trail");
    assert_eq!(fix::valid_id("1a b"), "id1a_b");
    assert_eq!(fix::valid_id("ok-1"), "ok-1");
}
