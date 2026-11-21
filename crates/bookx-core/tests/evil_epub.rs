//! Hostile EPUB internals: zip structure, archive budgets, OPF and chapter markup. Parsing and
//! layout input must give an error or a (possibly empty) document in bounded time, with the
//! default 2 MiB thread stack. Fuzz twin: `fuzz/fuzz_targets/epub.rs`.

use std::io::{Cursor, Write};
use std::time::{Duration, Instant};

use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::html::parse_html;
use bookx_core::opf::Package;

fn quick<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    assert!(
        t.elapsed() < Duration::from_secs(20),
        "{label}: {:?}",
        t.elapsed()
    );
    r
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (n, d) in files {
        w.start_file(*n, o).unwrap();
        w.write_all(d).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn nested(open: &str, close: &str, n: usize, inner: &str) -> String {
    format!("{}{inner}{}", open.repeat(n), close.repeat(n))
}

#[test]
fn deeply_nested_chapter_markup_does_not_overflow_the_stack() {
    let shapes: Vec<(&str, String)> = vec![
        ("div", nested("<div>", "</div>", 100_000, "text")),
        ("span", nested("<span>", "</span>", 100_000, "text")),
        ("bold-italic", nested("<b><i>", "</i></b>", 50_000, "text")),
        ("ul", nested("<ul><li>", "</li></ul>", 50_000, "text")),
        (
            "blockquote",
            nested("<blockquote>", "</blockquote>", 100_000, "text"),
        ),
        (
            "table",
            nested("<table><tr><td>", "</td></tr></table>", 20_000, "text"),
        ),
        ("a", nested("<a href='#x'>", "</a>", 50_000, "text")),
        ("unclosed", "<div><p><span><b>".repeat(50_000)),
        ("ruby", nested("<ruby>", "</ruby>", 30_000, "text")),
        (
            "sections",
            nested("<section><h1>", "</h1></section>", 30_000, "text"),
        ),
    ];
    for (label, body) in shapes {
        let html = format!("<html><body>{body}</body></html>");
        let doc = quick(label, || parse_html(&html, "OEBPS/c.xhtml"));
        let _ = doc;
    }
}

#[test]
fn flooded_markup_is_bounded() {
    for (label, html) in [
        (
            "attributes",
            format!("<p {}>x</p>", "a='1' ".repeat(400_000)),
        ),
        (
            "classes",
            format!("<p class='{}'>x</p>", "c ".repeat(200_000)),
        ),
        (
            "style",
            format!("<p style='{}'>x</p>", "color:red;".repeat(80_000)),
        ),
        (
            "entities",
            "&amp;&lt;&#x10ffff;&#0;&#xD800;&bogus;".repeat(80_000),
        ),
        ("comments", "<!--".repeat(80_000)),
        ("many-paragraphs", "<p>x</p>".repeat(100_000)),
        ("many-anchors", "<a id='a'></a>".repeat(80_000)),
        ("huge-img-list", "<img src='a.png'/>".repeat(100_000)),
        (
            "script-style",
            format!(
                "<script>{}</script><style>{}</style>",
                "x".repeat(1 << 22),
                "y".repeat(1 << 22)
            ),
        ),
    ] {
        let _ = quick(label, || parse_html(&html, "c.xhtml"));
    }
}

#[test]
fn hostile_zip_layouts_are_errors_or_harmless_archives() {
    let many = (0..2000)
        .map(|i| (format!("f{i}"), vec![b'x']))
        .collect::<Vec<_>>();
    let refs: Vec<(&str, &[u8])> = many
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    for (label, bytes) in [
        ("empty", vec![]),
        ("zeros", vec![0; 1 << 20]),
        ("pk", b"PK\x03\x04".to_vec()),
        ("many", zip_of(&refs)),
        (
            "traversal",
            zip_of(&[("../../x", b"a"), ("/abs", b"b"), ("a/../../c", b"c")]),
        ),
        ("case-dups", zip_of(&[("A.xhtml", b"1"), ("a.xhtml", b"2")])),
        ("nul-name", zip_of(&[("a\0b", b"1")])),
        ("long-name", zip_of(&[(&"n".repeat(60_000), b"1")])),
    ] {
        if let Ok(arc) = quick(label, || Archive::from_bytes(bytes)) {
            // whatever names it holds, saving it again must not panic or write outside memory
            let _ = quick(label, || arc.to_bytes());
            let _ = quick(label, || OpenBook::from_archive(arc));
        }
    }
}

#[test]
fn hostile_opf_documents_are_errors_or_packages() {
    let ns = r#"xmlns="http://www.idpf.org/2007/opf""#;
    let items = |n: usize| {
        (0..n)
            .map(|i| {
                format!(r#"<item id="i{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>"#)
            })
            .collect::<String>()
    };
    let refs = |n: usize| {
        (0..n)
            .map(|i| format!(r#"<itemref idref="i{i}"/>"#))
            .collect::<String>()
    };
    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("not-xml", "\0\0\0".into()),
        (
            "no-manifest",
            format!(r#"<package {ns}><metadata/></package>"#),
        ),
        (
            "many-items",
            format!(
                r#"<package {ns}><manifest>{}</manifest><spine>{}</spine></package>"#,
                items(100_000),
                refs(100_000)
            ),
        ),
        (
            "dup-ids",
            format!(
                r#"<package {ns}><manifest>{}</manifest><spine>{}</spine></package>"#,
                items(1).repeat(50_000),
                refs(1).repeat(50_000)
            ),
        ),
        (
            "idref-loops",
            format!(
                r#"<package {ns}><manifest><item id="a" href="../../etc/passwd" media-type="x"/><item id="b" href="/abs" media-type="x"/><item id="c" href="http://169.254.169.254/" media-type="x"/></manifest><spine><itemref idref="a"/><itemref idref="a"/><itemref idref="zz"/></spine></package>"#
            ),
        ),
        (
            "huge-metadata",
            format!(
                r#"<package {ns}><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">{}</metadata></package>"#,
                "<dc:creator>x</dc:creator>".repeat(200_000)
            ),
        ),
        ("deep", nested("<package>", "</package>", 100_000, "")),
        (
            "attr-flood",
            format!("<package {ns} {}/>", "a='1' ".repeat(300_000)),
        ),
    ];
    for (label, text) in cases {
        let _ = quick(label, || Package::parse_text("OEBPS/content.opf", &text));
    }
}
