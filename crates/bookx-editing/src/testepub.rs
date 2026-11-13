//! Hand-written EPUB 2 and EPUB 3 fixtures for the validator tests: a clean book of each version and a table of
//! faults, each of which breaks exactly one validator rule. Used by the unit tests, the integration tests and
//! the optional epubveri oracle test. Everything is generated in code.

use std::io::{Cursor, Write};

use bookx_core::archive::Archive;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// The files of an EPUB in zip order.
pub type Files = Vec<(String, Vec<u8>)>;

pub const OPF: &str = "OEBPS/content.opf";
pub const UID: &str = "urn:uuid:12345678-1234-1234-1234-123456789abc";

pub fn set(files: &mut Files, name: &str, content: impl Into<Vec<u8>>) {
    let content = content.into();
    match files.iter_mut().find(|f| f.0 == name) {
        Some(f) => f.1 = content,
        None => files.push((name.to_string(), content)),
    }
}

pub fn remove(files: &mut Files, name: &str) {
    let before = files.len();
    files.retain(|f| f.0 != name);
    assert!(files.len() < before, "no file {name} to remove");
}

pub fn get_string(files: &Files, name: &str) -> String {
    let f = files
        .iter()
        .find(|f| f.0 == name)
        .unwrap_or_else(|| panic!("no file {name}"));
    String::from_utf8_lossy(&f.1).to_string()
}

/// Replaces `from` by `to` in a file; panics when `from` is not there (so no fixture is silently a no-op).
pub fn rep(files: &mut Files, name: &str, from: &str, to: &str) {
    let s = get_string(files, name);
    assert!(s.contains(from), "{name} does not contain {from:?}");
    set(files, name, s.replacen(from, to, 1));
}

/// Adds a manifest item (before `</manifest>`) and, when `content` is given, the file next to the package.
pub fn add_item(
    files: &mut Files,
    id: &str,
    href: &str,
    media_type: &str,
    extra_attrs: &str,
    content: Option<&[u8]>,
) {
    rep(
        files,
        OPF,
        "</manifest>",
        &format!("<item id=\"{id}\" href=\"{href}\" media-type=\"{media_type}\"{extra_attrs}/></manifest>"),
    );
    if let Some(c) = content {
        let decoded = decode_href(href);
        set(files, &format!("OEBPS/{decoded}"), c.to_vec());
    }
}

fn decode_href(href: &str) -> String {
    bookx_core::util::percent_decode(href)
}

pub fn png() -> Vec<u8> {
    let img = image::RgbImage::from_fn(8, 8, |x, y| {
        image::Rgb([(x * 30) as u8, (y * 30) as u8, 99])
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("png");
    out
}

/// A zip with `mimetype` first and stored, everything else deflated, in the order given.
pub fn to_bytes(files: &Files) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut out);
        for (name, data) in files {
            let method = if name == "mimetype" {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(method),
            )
            .expect("zip entry");
            zip.write_all(data).expect("zip data");
        }
        zip.finish().expect("zip");
    }
    out.into_inner()
}

pub fn to_archive(files: &Files) -> Archive {
    Archive::from_bytes(to_bytes(files)).expect("archive")
}

/// XHTML document for EPUB 3 (HTML5 doctype).
pub fn xhtml3(head: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"en\" xml:lang=\"en\">\n<head>{head}</head>\n<body>{body}</body>\n</html>\n"
    )
}

/// XHTML document for EPUB 2 (XHTML 1.1 doctype).
pub fn xhtml2(head: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\">\n<head>{head}</head>\n<body>{body}</body>\n</html>\n"
    )
}

const CSS_HEAD: &str = "<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>";

pub const STYLE: &str = "@charset \"UTF-8\";\n@font-face { font-family: \"Tiny Test\"; src: url(fonts/f.ttf); }\nbody { margin: 0; background: url(\"images/cover.png\"); }\n";

const CONTAINER: &str = "<?xml version=\"1.0\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>\n";

/// A clean EPUB 3.
pub fn clean_epub3() -> Files {
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\" xml:lang=\"en\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"uid\">{UID}</dc:identifier>\n<dc:title>Fixture</dc:title>\n<dc:language>en</dc:language>\n<meta property=\"dcterms:modified\">2024-01-02T03:04:05Z</meta>\n</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n<item id=\"ch1\" href=\"ch1.xhtml\" media-type=\"application/xhtml+xml\"/>\n<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\"/>\n<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n<item id=\"cover\" href=\"images/cover.png\" media-type=\"image/png\" properties=\"cover-image\"/>\n<item id=\"font\" href=\"fonts/f.ttf\" media-type=\"font/ttf\"/>\n</manifest>\n<spine>\n<itemref idref=\"ch1\"/>\n<itemref idref=\"ch2\"/>\n</spine>\n</package>\n"
    );
    let ch1 = xhtml3(
        &format!("<title>One</title>{CSS_HEAD}"),
        "<section epub:type=\"chapter\" id=\"c1\"><h1 id=\"top\">One</h1><p>Text <a href=\"ch2.xhtml#sec\">next</a> and <a href=\"#top\">top</a>.</p><img src=\"images/cover.png\" alt=\"cover\"/></section>",
    );
    let ch2 = xhtml3(
        &format!("<title>Two</title>{CSS_HEAD}"),
        "<section epub:type=\"chapter\" id=\"sec\"><h1>Two</h1><p>More <a href=\"ch1.xhtml\">back</a>.</p></section>",
    );
    let nav = xhtml3(
        "<title>Contents</title>",
        "<nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1><ol><li><a href=\"ch1.xhtml\">One</a><ol><li><a href=\"ch1.xhtml#top\">Top</a></li></ol></li><li><a href=\"ch2.xhtml\">Two</a></li></ol></nav>",
    );
    vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            CONTAINER.as_bytes().to_vec(),
        ),
        (OPF.into(), opf.into_bytes()),
        ("OEBPS/nav.xhtml".into(), nav.into_bytes()),
        ("OEBPS/ch1.xhtml".into(), ch1.into_bytes()),
        ("OEBPS/ch2.xhtml".into(), ch2.into_bytes()),
        ("OEBPS/style.css".into(), STYLE.as_bytes().to_vec()),
        ("OEBPS/images/cover.png".into(), png()),
        ("OEBPS/fonts/f.ttf".into(), crate::testfont::tiny_ttf(0)),
    ]
}

pub fn ncx(uid: &str, nav_map: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"{uid}\"/></head><docTitle><text>Fixture</text></docTitle>{nav_map}</ncx>\n"
    )
}

pub const NAV_MAP: &str = "<navMap><navPoint id=\"n1\" playOrder=\"1\"><navLabel><text>One</text></navLabel><content src=\"ch1.xhtml\"/><navPoint id=\"n11\" playOrder=\"2\"><navLabel><text>Top</text></navLabel><content src=\"ch1.xhtml#top\"/></navPoint></navPoint><navPoint id=\"n2\" playOrder=\"3\"><navLabel><text>Two</text></navLabel><content src=\"ch2.xhtml\"/></navPoint></navMap>";

/// A clean EPUB 2.
pub fn clean_epub2() -> Files {
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"uid\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n<dc:identifier id=\"uid\">{UID}</dc:identifier>\n<dc:title>Fixture</dc:title>\n<dc:language>en</dc:language>\n<meta name=\"cover\" content=\"cover\"/>\n</metadata>\n<manifest>\n<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n<item id=\"ch1\" href=\"ch1.xhtml\" media-type=\"application/xhtml+xml\"/>\n<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\"/>\n<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n<item id=\"cover\" href=\"images/cover.png\" media-type=\"image/png\"/>\n<item id=\"font\" href=\"fonts/f.ttf\" media-type=\"application/x-font-ttf\"/>\n</manifest>\n<spine toc=\"ncx\">\n<itemref idref=\"ch1\"/>\n<itemref idref=\"ch2\"/>\n</spine>\n<guide><reference type=\"text\" title=\"Start\" href=\"ch1.xhtml\"/></guide>\n</package>\n"
    );
    let ch1 = xhtml2(
        &format!("<title>One</title>{CSS_HEAD}"),
        "<div id=\"c1\"><h1 id=\"top\">One</h1><p>Text <a href=\"ch2.xhtml#sec\">next</a> and <a href=\"#top\">top</a>.</p><p><img src=\"images/cover.png\" alt=\"cover\"/></p></div>",
    );
    let ch2 = xhtml2(
        &format!("<title>Two</title>{CSS_HEAD}"),
        "<div id=\"sec\"><h1>Two</h1><p>More <a href=\"ch1.xhtml\">back</a>.</p></div>",
    );
    vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml".into(),
            CONTAINER.as_bytes().to_vec(),
        ),
        (OPF.into(), opf.into_bytes()),
        ("OEBPS/toc.ncx".into(), ncx(UID, NAV_MAP).into_bytes()),
        ("OEBPS/ch1.xhtml".into(), ch1.into_bytes()),
        ("OEBPS/ch2.xhtml".into(), ch2.into_bytes()),
        ("OEBPS/style.css".into(), STYLE.as_bytes().to_vec()),
        ("OEBPS/images/cover.png".into(), png()),
        ("OEBPS/fonts/f.ttf".into(), crate::testfont::tiny_ttf(0)),
    ]
}

/// One fault: applying `apply` to the clean book of `version` breaks exactly the rule `id` (and also those in
/// `also`, when a second finding is unavoidable).
pub struct Fault {
    pub id: &'static str,
    pub version: u8,
    pub also: &'static [&'static str],
    /// epubveri is expected to report an error for this fault too (used by the oracle test)
    pub oracle_error: bool,
    pub apply: fn(&mut Files),
}

const fn f(id: &'static str, version: u8, oracle_error: bool, apply: fn(&mut Files)) -> Fault {
    Fault {
        id,
        version,
        also: &[],
        oracle_error,
        apply,
    }
}

const OTHER_CSS: &[u8] = b"p { margin: 0; }\n";
const ENCRYPTION: &str = "<?xml version=\"1.0\"?>\n<encryption xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\" xmlns:enc=\"http://www.w3.org/2001/04/xmlenc#\"><enc:EncryptedData><enc:EncryptionMethod Algorithm=\"http://example.com/rot13\"/><enc:CipherData><enc:CipherReference URI=\"OEBPS/ch2.xhtml\"/></enc:CipherData></enc:EncryptedData></encryption>\n";

fn ch2_v3(body: &str) -> String {
    xhtml3(&format!("<title>Two</title>{CSS_HEAD}"), body)
}

/// Faults that can be applied to the archive (so `validate(&Archive)` sees them). Zip-level faults that an
/// `Archive` cannot represent are in [`zip_faults`].
pub fn faults() -> Vec<Fault> {
    vec![
        f("PKG-006", 3, true, |x| remove(x, "mimetype")),
        f("PKG-007", 3, true, |x| {
            set(x, "mimetype", "application/epub+zip\n")
        }),
        f("PKG-009", 3, false, |x| {
            add_item(x, "q", "q%3F.txt", "text/plain", "", Some(b"x"))
        }),
        f("PKG-010", 3, false, |x| {
            add_item(x, "sp", "my%20file.txt", "text/plain", "", Some(b"x"))
        }),
        f("PKG-011", 3, false, |x| {
            add_item(x, "dot", "trail.", "text/plain", "", Some(b"x"))
        }),
        f("PKG-012", 3, false, |x| {
            add_item(x, "uni", "%C3%A9.txt", "text/plain", "", Some(b"x"))
        }),
        f("PKG-024", 3, false, |x| {
            add_item(x, "lo", "a.txt", "text/plain", "", Some(b"x"));
            add_item(x, "up", "A.txt", "text/plain", "", Some(b"y"));
        }),
        f("PKG-028", 3, false, |x| {
            let name = format!("{}.txt", "x".repeat(260));
            add_item(x, "long", &name, "text/plain", "", Some(b"x"));
        }),
        f("RSC-002", 3, true, |x| remove(x, "META-INF/container.xml")),
        f("RSC-003", 3, true, |x| {
            set(x, "META-INF/container.xml", "<container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles/></container>")
        }),
        f("RSC-004", 3, false, |x| {
            set(x, "META-INF/encryption.xml", ENCRYPTION)
        }),
        f("OPF-002", 3, true, |x| {
            rep(
                x,
                "META-INF/container.xml",
                "OEBPS/content.opf",
                "OEBPS/missing.opf",
            )
        }),
        f("RSC-016", 3, true, |x| {
            rep(x, "OEBPS/ch2.xhtml", "</body>", "</bod>")
        }),
        f("OPF-001", 3, true, |x| {
            rep(x, OPF, "version=\"3.0\"", "version=\"4.0\"")
        }),
        f("OPF-901", 3, true, |x| {
            rep(
                x,
                OPF,
                &format!(">{UID}</dc:identifier>"),
                "></dc:identifier>",
            )
        }),
        f("OPF-902", 3, true, |x| {
            rep(x, OPF, "<dc:title>Fixture</dc:title>", "")
        }),
        f("OPF-055", 3, false, |x| {
            rep(
                x,
                OPF,
                "<dc:title>Fixture</dc:title>",
                "<dc:title></dc:title>",
            )
        }),
        f("OPF-903", 3, true, |x| {
            rep(x, OPF, "<dc:language>en</dc:language>", "")
        }),
        f("OPF-092", 3, true, |x| {
            rep(
                x,
                OPF,
                "<dc:language>en</dc:language>",
                "<dc:language>not a lang!</dc:language>",
            )
        }),
        f("OPF-048", 3, true, |x| {
            rep(x, OPF, " unique-identifier=\"uid\"", "")
        }),
        f("OPF-030", 3, true, |x| {
            rep(
                x,
                OPF,
                "unique-identifier=\"uid\"",
                "unique-identifier=\"nope\"",
            )
        }),
        f("OPF-904", 3, true, |x| {
            rep(
                x,
                OPF,
                "<meta property=\"dcterms:modified\">2024-01-02T03:04:05Z</meta>",
                "",
            )
        }),
        f("OPF-905", 3, true, |x| {
            rep(x, OPF, "2024-01-02T03:04:05Z", "2024-01-02")
        }),
        f("OPF-906", 3, true, |x| {
            add_item(x, "css", "other.css", "text/css", "", Some(OTHER_CSS))
        }),
        f("OPF-907", 3, true, |x| {
            rep(
                x,
                OPF,
                "</manifest>",
                "<item id=\"bad\" media-type=\"text/css\"/></manifest>",
            )
        }),
        f("OPF-074", 3, true, |x| {
            add_item(x, "css2", "style.css", "text/css", "", None)
        }),
        f("OPF-091", 3, false, |x| {
            add_item(x, "frag", "other.css#x", "text/css", "", None);
            set(x, "OEBPS/other.css", OTHER_CSS);
        }),
        f("OPF-099", 3, true, |x| {
            add_item(
                x,
                "self",
                "content.opf",
                "application/oebps-package+xml",
                "",
                None,
            )
        }),
        f("RSC-001", 3, true, |x| {
            add_item(x, "gone", "gone.css", "text/css", "", None)
        }),
        f("OPF-003", 3, false, |x| set(x, "OEBPS/extra.txt", "loose")),
        f("OPF-029", 3, true, |x| {
            rep(
                x,
                OPF,
                "media-type=\"image/png\"",
                "media-type=\"image/jpeg\"",
            )
        }),
        f("OPF-035", 3, false, |x| {
            let doc = xhtml3("<title>Old</title>", "<p>old</p>");
            add_item(x, "old", "old.xhtml", "text/html", "", Some(doc.as_bytes()));
        }),
        f("OPF-033", 3, true, |x| {
            rep(
                x,
                OPF,
                "<itemref idref=\"ch1\"/>\n<itemref idref=\"ch2\"/>",
                "",
            )
        }),
        f("OPF-049", 3, true, |x| {
            rep(x, OPF, "</spine>", "<itemref idref=\"ghost\"/></spine>")
        }),
        f("OPF-034", 3, true, |x| {
            rep(x, OPF, "</spine>", "<itemref idref=\"ch1\"/></spine>")
        }),
        f("OPF-917", 3, true, |x| {
            rep(
                x,
                OPF,
                "<itemref idref=\"ch2\"/>",
                "<itemref idref=\"ch2\" linear=\"maybe\"/>",
            )
        }),
        f("OPF-918", 3, false, |x| {
            rep(
                x,
                OPF,
                "<itemref idref=\"ch2\"/>",
                "<itemref idref=\"ch2\" linear=\"no\"/>",
            )
        }),
        f("OPF-910", 2, true, |x| {
            rep(x, OPF, "<spine toc=\"ncx\">", "<spine>")
        }),
        f("OPF-050", 2, true, |x| {
            rep(x, OPF, "<spine toc=\"ncx\">", "<spine toc=\"ch1\">")
        }),
        f("OPF-912", 3, true, |x| {
            rep(x, OPF, " properties=\"nav\"", "")
        }),
        f("OPF-913", 3, true, |x| {
            let nav = get_string(x, "OEBPS/nav.xhtml");
            add_item(
                x,
                "nav2",
                "nav2.xhtml",
                "application/xhtml+xml",
                " properties=\"nav\"",
                Some(nav.as_bytes()),
            );
        }),
        f("OPF-012", 3, true, |x| {
            rep(x, OPF, "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>", "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"cover-image\"/>")
        }),
        f("OPF-915", 2, false, |x| {
            rep(
                x,
                OPF,
                "<meta name=\"cover\" content=\"cover\"/>",
                "<meta name=\"cover\" content=\"nope\"/>",
            )
        }),
        f("OPF-027", 3, true, |x| {
            rep(x, OPF, "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>", "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"bogus\"/>")
        }),
        f("OPF-026", 3, true, |x| {
            rep(
                x,
                OPF,
                "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>",
                "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"bad!\"/>",
            )
        }),
        f("OPF-028", 3, true, |x| {
            rep(x, OPF, "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>", "<item id=\"css\" href=\"style.css\" media-type=\"text/css\" properties=\"foo:bar\"/>")
        }),
        f("OPF-014", 3, true, |x| {
            rep(
                x,
                "OEBPS/ch1.xhtml",
                "</body>",
                "<script>var a = 1;</script></body>",
            )
        }),
        f("OPF-015", 3, true, |x| {
            rep(x, OPF, "<item id=\"ch1\" href=\"ch1.xhtml\" media-type=\"application/xhtml+xml\"/>", "<item id=\"ch1\" href=\"ch1.xhtml\" media-type=\"application/xhtml+xml\" properties=\"scripted\"/>")
        }),
        f("OPF-018", 3, false, |x| {
            rep(x, OPF, "<item id=\"ch1\" href=\"ch1.xhtml\" media-type=\"application/xhtml+xml\"/>", "<item id=\"ch1\" href=\"ch1.xhtml\" media-type=\"application/xhtml+xml\" properties=\"remote-resources\"/>")
        }),
        f("OPF-040", 3, true, |x| {
            add_item(
                x,
                "b",
                "b.css",
                "text/css",
                " fallback=\"ghost\"",
                Some(OTHER_CSS),
            )
        }),
        f("OPF-042", 3, true, |x| {
            rep(x, OPF, "</spine>", "<itemref idref=\"cover\"/></spine>")
        }),
        f("OPF-045", 3, true, |x| {
            add_item(
                x,
                "x1",
                "x1.css",
                "text/css",
                " fallback=\"x2\"",
                Some(OTHER_CSS),
            );
            add_item(
                x,
                "x2",
                "x2.css",
                "text/css",
                " fallback=\"x1\"",
                Some(OTHER_CSS),
            );
        }),
        f("OPF-031", 2, false, |x| {
            rep(
                x,
                OPF,
                "href=\"ch1.xhtml\"/></guide>",
                "href=\"nowhere.xhtml\"/></guide>",
            )
        }),
        f("OPF-088", 3, false, |x| {
            rep(
                x,
                "OEBPS/ch1.xhtml",
                "epub:type=\"chapter\"",
                "epub:type=\"made-up\"",
            )
        }),
        f("HTM-901", 3, true, |x| {
            rep(
                x,
                "OEBPS/ch2.xhtml",
                " xmlns=\"http://www.w3.org/1999/xhtml\"",
                "",
            )
        }),
        f("HTM-902", 3, true, |x| {
            rep(x, "OEBPS/ch2.xhtml", "<title>Two</title>", "")
        }),
        f("HTM-903", 3, false, |x| {
            set(x, "OEBPS/ch2.xhtml", "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\"><body><section id=\"sec\"><p>x <a href=\"ch1.xhtml\">back</a></p></section></body></html>")
        }),
        f("HTM-905", 3, true, |x| {
            set(x, "OEBPS/ch2.xhtml", ch2_v3("<section id=\"sec\"><p id=\"sec\">dup <a href=\"ch1.xhtml\">back</a></p></section>"))
        }),
        f("HTM-909", 3, false, |x| {
            set(x, "OEBPS/ch2.xhtml", ch2_v3("<section id=\"sec\"><p aria-labelledby=\"nope\">x <a href=\"ch1.xhtml\">back</a></p></section>"))
        }),
        f("HTM-906", 2, false, |x| {
            rep(x, "OEBPS/ch2.xhtml", "</div>", "<canvas></canvas></div>")
        }),
        f("HTM-907", 3, true, |x| {
            set(x, "OEBPS/ch2.xhtml", ch2_v3("<section id=\"sec\"><p>x <a href=\"ch1.xhtml\">back</a></p><img src=\"images/cover.png\"/></section>"))
        }),
        Fault {
            id: "RSC-006",
            version: 3,
            also: &[],
            oracle_error: false,
            apply: |x| {
                set(x, "OEBPS/ch2.xhtml", ch2_v3("<section id=\"sec\"><p>x <a href=\"ch1.xhtml\">back</a></p><script src=\"http://example.com/a.js\"></script></section>"));
                rep(x, OPF, "<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\"/>", "<item id=\"ch2\" href=\"ch2.xhtml\" media-type=\"application/xhtml+xml\" properties=\"scripted\"/>");
            },
        },
        f("RSC-007", 3, true, |x| {
            rep(
                x,
                "OEBPS/ch2.xhtml",
                "href=\"ch1.xhtml\"",
                "href=\"nofile.xhtml\"",
            )
        }),
        f("RSC-008", 3, true, |x| {
            rep(
                x,
                "OEBPS/ch2.xhtml",
                "href=\"ch1.xhtml\"",
                "href=\"../META-INF/container.xml\"",
            )
        }),
        f("RSC-012", 3, true, |x| {
            rep(
                x,
                "OEBPS/ch2.xhtml",
                "href=\"ch1.xhtml\"",
                "href=\"ch1.xhtml#nonexistent\"",
            )
        }),
        f("RSC-026", 3, false, |x| {
            rep(
                x,
                "OEBPS/ch2.xhtml",
                "href=\"ch1.xhtml\"",
                "href=\"../../outside.xhtml\"",
            )
        }),
        f("ACC-004", 3, false, |x| {
            set(x, "OEBPS/ch2.xhtml", ch2_v3("<section id=\"sec\"><p>x <a href=\"ch1.xhtml\">back</a> <a href=\"ch1.xhtml\"></a></p></section>"))
        }),
        f("ACC-007", 3, false, |x| {
            rep(x, "OEBPS/ch2.xhtml", " lang=\"en\" xml:lang=\"en\"", "")
        }),
        f("NAV-001", 3, true, |x| {
            rep(
                x,
                "OEBPS/nav.xhtml",
                "<li><a href=\"ch2.xhtml\">Two</a></li>",
                "<li><p>Two</p></li>",
            )
        }),
        f("NAV-901", 3, true, |x| {
            rep(
                x,
                "OEBPS/nav.xhtml",
                "epub:type=\"toc\"",
                "epub:type=\"landmarks\"",
            )
        }),
        f("NAV-011", 3, false, |x| {
            rep(
                x,
                "OEBPS/nav.xhtml",
                "<li><a href=\"ch1.xhtml\">One</a><ol><li><a href=\"ch1.xhtml#top\">Top</a></li></ol></li><li><a href=\"ch2.xhtml\">Two</a></li>",
                "<li><a href=\"ch2.xhtml\">Two</a></li><li><a href=\"ch1.xhtml\">One</a></li>",
            )
        }),
        f("NCX-001", 2, true, |x| {
            set(x, "OEBPS/toc.ncx", ncx("urn:uuid:other", NAV_MAP))
        }),
        f("NCX-002", 2, false, |x| {
            rep(x, "OEBPS/toc.ncx", "<meta name=\"dtb:uid\" content=\"urn:uuid:12345678-1234-1234-1234-123456789abc\"/>", "")
        }),
        f("NCX-901", 2, true, |x| {
            set(x, "OEBPS/toc.ncx", ncx(UID, ""))
        }),
        f("NCX-902", 2, true, |x| {
            rep(
                x,
                "OEBPS/toc.ncx",
                "<navPoint id=\"n2\" playOrder=\"3\">",
                "<navPoint id=\"n2\">",
            )
        }),
        f("NCX-903", 2, true, |x| {
            rep(
                x,
                "OEBPS/toc.ncx",
                "<navPoint id=\"n2\" playOrder=\"3\">",
                "<navPoint id=\"n2\" playOrder=\"2\">",
            )
        }),
        f("NCX-904", 2, true, |x| {
            rep(x, "OEBPS/toc.ncx", "<text>Two</text>", "<text></text>")
        }),
        f("CSS-001", 3, false, |x| {
            set(
                x,
                "OEBPS/style.css",
                format!("{STYLE}p {{ direction: rtl; }}\n"),
            )
        }),
        f("CSS-003", 3, true, |x| {
            rep(
                x,
                "OEBPS/style.css",
                "@charset \"UTF-8\"",
                "@charset \"ISO-8859-1\"",
            )
        }),
        f("CSS-008", 3, true, |x| {
            set(x, "OEBPS/style.css", format!("{STYLE}p {{ color: red;\n"))
        }),
        f("CSS-902", 3, false, |x| {
            set(x, "OEBPS/style.css", format!("{STYLE}@foo bar;\n"))
        }),
    ]
}

/// Zip-level faults: `(rule id, bytes of the EPUB)`.
pub fn zip_faults() -> Vec<(&'static str, Vec<u8>)> {
    let base = clean_epub3();
    let build = |mimetype_pos: Option<usize>,
                 method: CompressionMethod,
                 extra: bool,
                 dirs: &[&str]| {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut out);
            let mut rest: Vec<&(String, Vec<u8>)> =
                base.iter().filter(|f| f.0 != "mimetype").collect();
            let mime = &base[0];
            let write_mime = |zip: &mut ZipWriter<&mut Cursor<Vec<u8>>>| {
                if extra {
                    let o = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
                        .compression_method(method);
                    let mut o = o;
                    o.add_extra_field(0x5455, [1u8, 0, 0, 0, 0], false)
                        .expect("extra data");
                    zip.start_file("mimetype", o).expect("entry");
                } else {
                    let o = SimpleFileOptions::default().compression_method(method);
                    zip.start_file("mimetype", o).expect("entry");
                }
                zip.write_all(&mime.1).expect("data");
            };
            let at = mimetype_pos.unwrap_or(0);
            for (i, f) in rest.drain(..).enumerate() {
                if i == at {
                    write_mime(&mut zip);
                }
                zip.start_file(
                    &f.0,
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )
                .expect("entry");
                zip.write_all(&f.1).expect("data");
            }
            if at >= base.len() - 1 {
                write_mime(&mut zip);
            }
            for d in dirs {
                zip.add_directory(*d, SimpleFileOptions::default())
                    .expect("dir");
            }
            zip.finish().expect("zip");
        }
        out.into_inner()
    };
    let mut dup_files = clean_epub3();
    add_item(
        &mut dup_files,
        "dup1",
        "dup1.txt",
        "text/plain",
        "",
        Some(b"one"),
    );
    set(&mut dup_files, "OEBPS/dup2.txt", "two");
    let dup = to_bytes(&dup_files);
    let dup = replace_bytes(&dup, b"OEBPS/dup2.txt", b"OEBPS/dup1.txt");
    vec![
        ("PKG-004", b"this is not a zip file at all".to_vec()),
        ("PKG-005", build(None, CompressionMethod::Stored, true, &[])),
        (
            "PKG-006",
            build(Some(2), CompressionMethod::Stored, false, &[]),
        ),
        (
            "PKG-022",
            build(None, CompressionMethod::Deflated, false, &[]),
        ),
        (
            "PKG-014",
            build(
                None,
                CompressionMethod::Stored,
                false,
                &["OEBPS/", "OEBPS/empty/"],
            ),
        ),
        ("OPF-060", dup),
    ]
}

fn replace_bytes(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let mut out = data.to_vec();
    let mut i = 0;
    let mut n = 0;
    while i + from.len() <= out.len() {
        if &out[i..i + from.len()] == from {
            out[i..i + from.len()].copy_from_slice(to);
            i += from.len();
            n += 1;
        } else {
            i += 1;
        }
    }
    assert!(
        n >= 2,
        "expected the name in the local header and the directory"
    );
    out
}
