//! End-to-end: a hand-written broken EPUB goes through fixes, upgrade, accessibility, arrangement, with
//! checkpoints, compare and reports along the way. Each stage is validated from the zip bytes alone.

mod common;

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::check::{check_book, Severity};
use bookx_editing::accessibility::{make_accessible, AccessibilityOptions};
use bookx_editing::checkpoints::Checkpoints;
use bookx_editing::compare::{archive_unified_diff, compare_archives, CompareOptions};
use bookx_editing::fixes::{fix_archive, FixKind, FixOptions};
use bookx_editing::inline_toc::{insert_inline_toc, InlineTocOptions};
use bookx_editing::links::{arrange_files, ArrangeOptions};
use bookx_editing::markup::normalized_text;
use bookx_editing::reports::{build_report, Report, ReportOptions};
use bookx_editing::semantics::{read_semantics, set_semantic, Semantic};
use bookx_editing::upgrade::{upgrade_to_epub3, UpgradeOptions};

fn errors(a: &Archive) -> Vec<String> {
    check_book(a)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{}: {}", i.file, i.message))
        .collect()
}

#[test]
fn messy_epub_through_the_whole_toolchain() {
    let original = Archive::from_bytes(common::messy_epub()).unwrap();
    let before_errors = errors(&original);
    assert!(
        before_errors.len() >= 2,
        "the fixture must be broken: {before_errors:?}"
    );

    // ---- fixes -------------------------------------------------------------------------------------------
    let mut cps = Checkpoints::new(10);
    let c0 = cps.create("as imported", &original);
    let (mut book, report) = fix_archive(original.clone(), &FixOptions::default()).unwrap();
    let kinds: Vec<FixKind> = report.fixes.iter().map(|f| f.kind).collect();
    for k in [
        FixKind::Mimetype,
        FixKind::ManifestRemoved,
        FixKind::DuplicateManifestId,
        FixKind::ManifestAdded,
        FixKind::Metadata,
        FixKind::WellFormedness,
        FixKind::DuplicateElementId,
        FixKind::MissingAlt,
    ] {
        assert!(kinds.contains(&k), "{k:?} missing in {:#?}", report.fixes);
    }
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(
        report.remaining_errors.is_empty(),
        "{:?}",
        report.remaining_errors
    );
    let files = common::validate_epub(&book.archive.to_bytes().unwrap());
    assert!(!files.contains_key("OEBPS/ghost.xhtml"));
    let c1 = cps.create("after fixes", &book.archive);

    // text of the repaired chapter is intact
    let c1_text = normalized_text(&book.archive.get_string("OEBPS/c1.html").unwrap());
    assert!(c1_text.contains("Tom & Jerry"), "{c1_text}");
    assert!(c1_text.contains("again."));
    assert!(c1_text.contains("A second paragraph — with an entity."));
    // fixing again is a no-op
    let (_, again) = fix_archive(book.archive.clone(), &FixOptions::default()).unwrap();
    assert!(again.is_empty(), "{:?}", again.fixes);

    // ---- compare what the fixes did ------------------------------------------------------------------------
    let diff = compare_archives(&original, &book.archive, CompareOptions::default());
    assert!(diff.changed.iter().any(|c| c.path == "OEBPS/c1.html"));
    assert!(diff.changed.iter().any(|c| c.path == "OEBPS/content.opf"));
    assert!(
        archive_unified_diff(&original, &book.archive, CompareOptions::default())
            .contains("+++ b/OEBPS/c1.html")
    );
    let live = cps
        .diff_against(c0, &book.archive, CompareOptions::default())
        .unwrap();
    assert_eq!(live, diff);

    // ---- upgrade -------------------------------------------------------------------------------------------
    let up = upgrade_to_epub3(
        &mut book,
        &UpgradeOptions {
            modified: Some("2026-11-12T00:00:00Z".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!up.is_empty());
    assert!(book.package.is_epub3());
    let files = common::validate_epub(&book.archive.to_bytes().unwrap());
    let opf = String::from_utf8(files["OEBPS/content.opf"].clone()).unwrap();
    assert!(
        opf.contains("property=\"role\""),
        "opf:role converted to a refinement"
    );
    assert!(
        opf.contains("urn:uuid:0f0e0d0c"),
        "opf:scheme folded into the identifier"
    );
    assert!(!opf.contains("<guide"));
    assert_eq!(
        read_semantics(&book)
            .iter()
            .filter(|s| s.semantic == Semantic::Bodymatter)
            .count(),
        1
    );
    let c2 = cps.create("epub3", &book.archive);

    // ---- inline toc, semantics, accessibility ---------------------------------------------------------------
    let toc_path = insert_inline_toc(&mut book, &InlineTocOptions::default()).unwrap();
    set_semantic(&mut book, Semantic::Toc, &toc_path, "Contents").unwrap();
    let acc = make_accessible(
        &mut book,
        &AccessibilityOptions {
            mark_missing_alt_decorative: true,
            summary: Some("Navigable.".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(acc
        .changes
        .iter()
        .any(|c| c.contains("accessibility metadata")));
    let files = common::validate_epub(&book.archive.to_bytes().unwrap());
    let opf = String::from_utf8(files["OEBPS/content.opf"].clone()).unwrap();
    assert!(opf.contains("schema:accessibilitySummary"));
    let toc_page = String::from_utf8(files[&toc_path].clone()).unwrap();
    assert!(toc_page.contains("First") && toc_page.contains("Second"));
    cps.create("accessible", &book.archive);

    // ---- arrange ----------------------------------------------------------------------------------------------
    let texts_before = common::spine_text(&book);
    let toc_before: Vec<String> = book.toc().into_iter().map(|t| t.title).collect();
    let moved = arrange_files(&mut book, &ArrangeOptions::default()).unwrap();
    assert!(!moved.moved.is_empty());
    let files = common::validate_epub(&book.archive.to_bytes().unwrap());
    assert!(files.keys().any(|k| k.starts_with("OEBPS/Text/")));
    assert!(files.keys().any(|k| k.starts_with("OEBPS/Images/")));
    assert_eq!(common::spine_text(&book), texts_before);
    assert_eq!(
        book.toc().into_iter().map(|t| t.title).collect::<Vec<_>>(),
        toc_before
    );
    assert!(
        errors(&book.archive).is_empty(),
        "{:?}",
        errors(&book.archive)
    );

    // ---- reports --------------------------------------------------------------------------------------------------
    let rep = build_report(&book, &ReportOptions::default());
    assert!(rep.files.by_type["text"].count >= 4);
    assert!(rep.words.overall.total > 20);
    assert!(rep.links.broken.is_empty(), "{:?}", rep.links.broken);
    assert!(rep.css.unused_rules >= 1, "the .unused rule");
    assert_eq!(rep.images.images.len(), 2);
    assert_eq!(Report::from_json(&rep.to_json()).unwrap(), rep);

    // ---- checkpoints: persistence and restore ------------------------------------------------------------------
    let zip = cps.to_zip().unwrap();
    let back = Checkpoints::from_zip(&zip).unwrap();
    assert_eq!(back.list().len(), 4);
    let restored = back.restore(c1).unwrap();
    for n in restored.names() {
        assert!(restored.get(n).is_some());
    }
    assert!(errors(&restored).is_empty());
    let restored_c0 = back.restore(c0).unwrap();
    assert_eq!(
        restored_c0.to_bytes().unwrap(),
        original.to_bytes().unwrap()
    );
    assert!(back.shares_file(c1, c2, "OEBPS/style.css"));
    let reopened = OpenBook::from_archive(restored).unwrap();
    assert_eq!(reopened.spine_paths().len(), 2);
}
