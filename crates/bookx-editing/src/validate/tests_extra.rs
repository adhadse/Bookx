//! Tests of the extended checks (`BKX-` and `ACC-9xx`): one fixture per rule.

use super::extra::*;
use super::*;
use crate::testepub::*;

pub const CMYK_JPEG: &[u8] = include_bytes!("../../testdata/cmyk.jpg");

fn ext(files: &Files) -> Report {
    validate_with(&to_archive(files), &Options::extended())
}

fn acc(files: &Files) -> Report {
    validate_with(
        &to_archive(files),
        &Options {
            extended: true,
            accessibility: true,
        },
    )
}

fn ch2_body(files: &mut Files, body: &str) {
    set(
        files,
        "OEBPS/ch2.xhtml",
        xhtml3(
            "<title>Two</title><link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>",
            &format!(
                "<section epub:type=\"chapter\" id=\"sec\"><h1>Two</h1>{body}<a href=\"ch1.xhtml\">back</a></section>"
            ),
        ),
    );
}

fn restricted_font() -> Vec<u8> {
    let mut f = crate::testfont::tiny_ttf(0);
    let n = u16::from_be_bytes([f[4], f[5]]) as usize;
    for i in 0..n {
        let rec = 12 + i * 16;
        if &f[rec..rec + 4] == b"OS/2" {
            let off =
                u32::from_be_bytes([f[rec + 8], f[rec + 9], f[rec + 10], f[rec + 11]]) as usize;
            f[off + 8] = 0;
            f[off + 9] = 2;
        }
    }
    f
}

pub(crate) struct XFault {
    pub id: &'static str,
    pub also: &'static [&'static str],
    pub version: u8,
    pub a11y: bool,
    pub apply: fn(&mut Files),
}

const fn x(id: &'static str, also: &'static [&'static str], apply: fn(&mut Files)) -> XFault {
    XFault {
        id,
        also,
        version: 3,
        a11y: false,
        apply,
    }
}

pub(crate) fn xfaults() -> Vec<XFault> {
    vec![
        x("BKX-001", &["BKX-024"], |f| {
            add_item(f, "e", "empty.txt", "text/plain", "", Some(b""))
        }),
        x("BKX-002", &[], |f| {
            let mut v = get_string(f, "OEBPS/ch2.xhtml").into_bytes();
            let at = v.windows(4).position(|w| w == b"More").unwrap();
            v[at] = 0xE9;
            set(f, "OEBPS/ch2.xhtml", v);
        }),
        x("BKX-003", &[], |f| {
            rep(
                f,
                "OEBPS/ch2.xhtml",
                "encoding=\"UTF-8\"",
                "encoding=\"ISO-8859-1\"",
            )
        }),
        x("BKX-004", &[], |f| ch2_body(f, "<p>a&nbsp;b</p>")),
        x("BKX-005", &["RSC-016"], |f| ch2_body(f, "<p>AT&T</p>")),
        x("BKX-006", &[], |f| {
            rep(
                f,
                "OEBPS/ch2.xhtml",
                "<!DOCTYPE html>",
                "<!DOCTYPE html [<!ENTITY pub \"Publisher\">]>",
            );
            rep(f, "OEBPS/ch2.xhtml", "More", "More &pub;");
        }),
        x("BKX-007", &[], |f| {
            ch2_body(f, &format!("<p>{}</p>", "word ".repeat(60_000)))
        }),
        x("BKX-008", &[], |f| ch2_body(f, "<p id=\"1x\">a</p>")),
        x("BKX-009", &[], |f| {
            rep(f, "OEBPS/ch2.xhtml", "<body>", "<body>Loose words")
        }),
        x("BKX-010", &[], |f| {
            ch2_body(f, "<p style=\"color red\">a</p>")
        }),
        x("BKX-011", &[], |f| {
            ch2_body(f, "<p style=\"colour: red\">a</p>")
        }),
        x("BKX-012", &[], |f| {
            set(f, "OEBPS/fonts/f.ttf", b"not a font".to_vec())
        }),
        x("BKX-013", &[], |f| {
            set(f, "OEBPS/fonts/f.ttf", restricted_font())
        }),
        x("BKX-014", &[], |f| {
            rep(f, "OEBPS/style.css", "\"Tiny Test\"", "Other")
        }),
        x("BKX-015", &[], |f| {
            let p = png();
            set(f, "OEBPS/images/cover.png", p[..p.len() - 20].to_vec())
        }),
        x("BKX-016", &[], |f| {
            add_item(f, "c", "images/c.jpg", "image/jpeg", "", Some(CMYK_JPEG));
            ch2_body(f, "<img src=\"images/c.jpg\" alt=\"c\"/>");
        }),
        x("BKX-017", &["RSC-007"], |f| {
            ch2_body(f, "<p><a href=\"images/\">x</a></p>")
        }),
        x("BKX-018", &[], |f| {
            ch2_body(f, "<p><a href=\"http://\">x</a></p>")
        }),
        x("BKX-019", &["RSC-007"], |f| {
            ch2_body(f, "<p><a href=\"ch1 two.xhtml\">x</a></p>")
        }),
        x("BKX-020", &[], |f| {
            ch2_body(f, "<p><a href=\"file:///home/me/a.html\">x</a></p>")
        }),
        x("BKX-021", &[], |f| ch2_body(f, "<p><a href=\"\">x</a></p>")),
        x("BKX-022", &["RSC-007"], |f| {
            ch2_body(f, "<p><a href=\"CH1.xhtml\">x</a></p>")
        }),
        x("BKX-023", &[], |f| {
            ch2_body(f, "<p><a href=\"images/cover.png\">x</a></p>")
        }),
        x("BKX-024", &[], |f| {
            add_item(f, "t", "extra.txt", "text/plain", "", Some(b"x"))
        }),
        x("BKX-025", &[], |f| {
            let doc = xhtml3("<title>Three</title>", "<p>Three</p>");
            add_item(
                f,
                "ch3",
                "ch3.xhtml",
                "application/xhtml+xml",
                "",
                Some(doc.as_bytes()),
            );
        }),
        x("BKX-026", &[], |f| {
            set(f, "META-INF/calibre_bookmarks.txt", "x")
        }),
        XFault {
            id: "BKX-028",
            also: &[],
            version: 2,
            a11y: false,
            apply: |f| {
                rep(
                    f,
                    OPF,
                    "</metadata>",
                    "<meta name=\"cover\" content=\"cover\"/></metadata>",
                )
            },
        },
        XFault {
            id: "ACC-901",
            also: &["ACC-902", "ACC-903", "ACC-904"],
            version: 3,
            a11y: true,
            apply: |f| ch2_body(f, "<h3>skipped</h3>"),
        },
        XFault {
            id: "ACC-905",
            also: &["ACC-902", "ACC-903", "ACC-904"],
            version: 3,
            a11y: true,
            apply: |f| {
                ch2_body(
                    f,
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><rect width=\"5\" height=\"5\"/></svg>",
                );
                rep(
                    f,
                    OPF,
                    "<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\"/>",
                    "<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\" properties=\"svg\"/>",
                );
            },
        },
    ]
}

fn clean(v: u8) -> Files {
    if v == 2 { clean_epub2() } else { clean_epub3() }
}

#[test]
fn clean_books_stay_clean_with_the_extended_checks() {
    for v in [2, 3] {
        let r = ext(&clean(v));
        assert!(r.messages.is_empty(), "{}", render_text(&r));
    }
}

#[test]
fn every_extended_rule_has_a_fixture_or_a_dedicated_test() {
    let covered: HashSet<&str> = xfaults().iter().map(|f| f.id).collect();
    let elsewhere = ["BKX-027", "ACC-902", "ACC-903", "ACC-904"];
    for r in EXTRA_RULES {
        assert!(
            covered.contains(r.id) || elsewhere.contains(&r.id),
            "{} has no fixture",
            r.id
        );
        assert!(!r.title.is_empty());
        assert!(rule(r.id).is_some());
    }
}

#[test]
fn each_extended_fault_reports_its_rule() {
    for fault in xfaults() {
        let mut files = clean(fault.version);
        (fault.apply)(&mut files);
        let r = if fault.a11y { acc(&files) } else { ext(&files) };
        let mut want = vec![fault.id];
        want.extend(fault.also);
        want.sort();
        assert_eq!(r.ids(), want, "{}\n{}", fault.id, render_text(&r));
        let m = r.of(fault.id)[0];
        assert_eq!(m.severity, rule(fault.id).unwrap().severity);
        assert!(!m.text.is_empty());
        // the classic validator alone does not know the rule
        let classic = validate(&to_archive(&files));
        assert!(!classic.has(fault.id), "{}", fault.id);
    }
}

#[test]
fn extended_findings_carry_a_place_where_one_exists() {
    let mut f = clean_epub3();
    ch2_body(&mut f, "<p>one</p>\n<p id=\"9a\">two</p>");
    let r = ext(&f);
    let m = r.of("BKX-008")[0];
    assert_eq!(m.file, "OEBPS/ch2.xhtml");
    assert!(m.line.unwrap() >= 6, "{m:?}");

    let mut f = clean_epub3();
    ch2_body(&mut f, "<p>a</p>\n<p>b&nbsp;c</p>");
    let m = ext(&f).of("BKX-004")[0].clone();
    assert!(m.line.unwrap() >= 6 && m.col.is_some());
}

#[test]
fn a_package_without_sections_is_an_error() {
    let mut f = clean_epub3();
    rep(
        &mut f,
        OPF,
        "<spine>\n<itemref idref=\"ch1\"/>\n<itemref idref=\"ch2\"/>\n</spine>",
        "",
    );
    let r = ext(&f);
    assert!(r.has("BKX-027"), "{}", render_text(&r));
    assert!(r.of("BKX-027")[0].text.contains("<spine>"));
    assert_eq!(r.of("BKX-027")[0].severity, Severity::Error);
}

#[test]
fn accessibility_checks_run_only_when_asked() {
    let f = clean_epub3();
    let r = acc(&f);
    assert_eq!(
        r.ids(),
        vec!["ACC-902", "ACC-903", "ACC-904"],
        "{}",
        render_text(&r)
    );
    assert!(r.passed);
    assert!(
        r.of("ACC-902")
            .iter()
            .any(|m| m.text.contains("doc-chapter"))
    );
    // with the metadata present they go away
    let mut g = clean_epub3();
    rep(
        &mut g,
        OPF,
        "</metadata>",
        "<meta property=\"schema:accessMode\">textual</meta><meta property=\"schema:accessibilitySummary\">Plain text.</meta></metadata>",
    );
    rep(
        &mut g,
        "OEBPS/ch1.xhtml",
        "epub:type=\"chapter\"",
        "epub:type=\"chapter\" role=\"doc-chapter\"",
    );
    rep(
        &mut g,
        "OEBPS/ch2.xhtml",
        "epub:type=\"chapter\"",
        "epub:type=\"chapter\" role=\"doc-chapter\"",
    );
    rep(
        &mut g,
        "OEBPS/nav.xhtml",
        "epub:type=\"toc\"",
        "epub:type=\"toc\" role=\"doc-toc\"",
    );
    let r = acc(&g);
    assert!(r.messages.is_empty(), "{}", render_text(&r));
    assert!(ext(&clean_epub3()).messages.is_empty());
}

// ---- helpers --------------------------------------------------------------------------------------------

#[test]
fn entities_are_classified() {
    let hits = scan_entities(
        "<p>&amp; &lt; &nbsp; &eacute; &foo; AT&T &#169; &#xA9; &#x; &#12 & x</p><!-- &bar; --><script>a && b</script>",
    );
    let got: Vec<(&str, &str)> = hits.iter().map(|h| (h.id, h.name.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("BKX-004", "nbsp"),
            ("BKX-004", "eacute"),
            ("BKX-005", "foo"),
            ("BKX-005", "&T"),
            ("BKX-005", "&#x"),
            ("BKX-005", "&#12"),
            ("BKX-005", "&"),
        ]
    );
    let private = scan_entities("<!DOCTYPE html [<!ENTITY pub \"P\">]><p>&pub;</p>");
    assert_eq!(private.len(), 1);
    assert_eq!(private[0].id, "BKX-006");
}

#[test]
fn encodings_are_read_from_the_declaration() {
    assert_eq!(
        declared_encoding(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a/>").as_deref(),
        Some("iso-8859-1")
    );
    assert_eq!(
        declared_encoding(b"<html><head><meta charset=\"windows-1252\"></head>").as_deref(),
        Some("windows-1252")
    );
    assert_eq!(declared_encoding(b"<a/>"), None);
    assert!(is_utf8_label("utf-8") && !is_utf8_label("latin1"));
}

#[test]
fn jpeg_components_tell_cmyk_from_rgb() {
    assert_eq!(jpeg_components(CMYK_JPEG), Some(4));
    let mut rgb = Vec::new();
    image::RgbImage::new(4, 4)
        .write_to(
            &mut std::io::Cursor::new(&mut rgb),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    assert_eq!(jpeg_components(&rgb), Some(3));
    assert_eq!(jpeg_components(b"nope"), None);
}

#[test]
fn link_characters() {
    assert_eq!(link_bad_char("a/b.xhtml"), None);
    assert_eq!(link_bad_char("a%20b.xhtml"), None);
    assert_eq!(link_bad_char("a b.xhtml"), Some(' '));
    assert_eq!(link_bad_char("a\\b.xhtml"), Some('\\'));
    assert_eq!(link_bad_char("C:/a.xhtml"), Some(':'));
    assert_eq!(link_bad_char("a%2.xhtml"), Some('%'));
}

#[test]
fn css_declarations_are_judged() {
    use super::css::declaration_problem as p;
    assert!(p("color: red").is_none());
    assert!(p("-webkit-hyphens: auto").is_none());
    assert!(p("--x: 1").is_none());
    assert!(p("  ").is_none());
    assert_eq!(p("color red").unwrap().0, "BKX-010");
    assert_eq!(p("color:").unwrap().0, "BKX-010");
    assert_eq!(p("*zoom: 1").unwrap().0, "BKX-010");
    assert_eq!(p("background: url(x.png").unwrap().0, "BKX-010");
    assert_eq!(p("colour: red").unwrap().0, "BKX-011");
}

#[test]
fn style_sheets_and_style_elements_are_checked_for_declarations() {
    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/style.css",
        "body { margin: 0;",
        "body { margin 0; colour: red;",
    );
    let r = ext(&f);
    assert_eq!(r.ids(), vec!["BKX-010", "BKX-011"], "{}", render_text(&r));
    let m = r.of("BKX-011")[0];
    assert_eq!((m.file.as_str(), m.line), ("OEBPS/style.css", Some(3)));

    let mut f = clean_epub3();
    rep(
        &mut f,
        "OEBPS/ch2.xhtml",
        "<title>Two</title>",
        "<title>Two</title><style>p { colour: red }</style>",
    );
    let r = ext(&f);
    assert_eq!(r.ids(), vec!["BKX-011"], "{}", render_text(&r));
    assert_eq!(r.of("BKX-011")[0].file, "OEBPS/ch2.xhtml");
    // descriptors of @font-face and nested rules are fine
    let mut f = clean_epub3();
    set(
        &mut f,
        "OEBPS/style.css",
        format!(
            "{STYLE}@media print {{ p {{ margin: 0 }} }}\n@font-face {{ font-family: X; font-display: swap; unicode-range: U+0-7F; src: url(fonts/f.ttf); }}\n"
        ),
    );
    let r = ext(&f);
    assert!(
        !r.has("BKX-010") && !r.has("BKX-011"),
        "{}",
        render_text(&r)
    );
}

#[test]
fn unreferenced_checks_skip_what_is_used() {
    // the font is used by the style sheet, the cover by the page, nav and ncx need no reference
    for v in [2, 3] {
        let r = ext(&clean(v));
        assert!(
            !r.has("BKX-024") && !r.has("BKX-025"),
            "{}",
            render_text(&r)
        );
    }
    let mut f = clean_epub3();
    rep(&mut f, "OEBPS/style.css", "src: url(fonts/f.ttf);", "");
    let r = ext(&f);
    assert!(r.has("BKX-024"), "{}", render_text(&r));
    assert_eq!(r.of("BKX-024")[0].file, "OEBPS/fonts/f.ttf");
}

#[test]
fn named_entities_are_only_a_warning_where_the_document_type_defines_them() {
    let mut f = clean_epub2();
    rep(&mut f, "OEBPS/ch2.xhtml", "More", "More&nbsp;");
    let r = ext(&f);
    assert_eq!(r.ids(), vec!["BKX-004"], "{}", render_text(&r));
    assert_eq!(r.of("BKX-004")[0].severity, Severity::Warning);
    assert!(r.passed);
    let mut g = clean_epub3();
    rep(&mut g, "OEBPS/ch2.xhtml", "More", "More&nbsp;");
    let r = ext(&g);
    assert_eq!(r.of("BKX-004")[0].severity, Severity::Error);
    assert!(!r.passed);
}

#[test]
fn a_directory_link_is_a_folder_link_not_a_case_mismatch() {
    let mut f = clean_epub3();
    ch2_body(&mut f, "<p><a href=\"fonts\">x</a></p>");
    let r = ext(&f);
    assert!(r.has("BKX-017"), "{}", render_text(&r));
    assert!(!r.has("BKX-022"));
}

/// Where epubveri's verdict differs from Bookx's for a book with one extended fault (documented in
/// docs/validator-notes.md). Bookx is stricter in both: a malformed named entity is only a warning for epubveri.
const DIFFERENT_FROM_EPUBVERI: &[&str] = &[];

fn epubveri_binary() -> Option<std::path::PathBuf> {
    use std::process::{Command, Stdio};
    if std::env::var("BOOKX_ORACLE").as_deref() == Ok("0") {
        return None;
    }
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("BOOKX_EPUBVERI") {
        candidates.push(std::path::PathBuf::from(p));
    }
    candidates.push("epubveri".into());
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(std::path::Path::new(&home).join(".cache/epubveri/bin/epubveri"));
    }
    candidates.into_iter().find(|c| {
        Command::new(c)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// The extended rules never change a verdict, so Bookx and epubveri must still agree on books with one
/// extended fault (skipped without the epubveri binary).
#[test]
fn the_extended_rules_do_not_contradict_epubveri() {
    let Some(bin) = epubveri_binary() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("bookx-xoracle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut bad = Vec::new();
    for fault in xfaults() {
        if DIFFERENT_FROM_EPUBVERI.contains(&fault.id) {
            continue;
        }
        let mut files = clean(fault.version);
        (fault.apply)(&mut files);
        let path = dir.join(format!("{}.epub", fault.id));
        std::fs::write(&path, to_bytes(&files)).unwrap();
        let out = std::process::Command::new(&bin)
            .arg("-i")
            .arg(&path)
            .output()
            .expect("running epubveri");
        let theirs_failed = !out.status.success();
        let ours_failed = !acc(&files).passed;
        if theirs_failed != ours_failed {
            bad.push(format!(
                "{}: epubveri {}, bookx {}: {}",
                fault.id,
                if theirs_failed { "fails" } else { "passes" },
                if ours_failed { "fails" } else { "passes" },
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
