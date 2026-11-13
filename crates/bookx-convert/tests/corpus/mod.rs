//! Synthetic source documents for the conversion tests, one per feature a converter can trip on.
//! Everything is generated in code (no network, no third party files) and deterministic.

#![allow(dead_code)]

use std::io::{Cursor, Write};

use crate::common::{jpeg, png};

pub struct Sample {
    /// Short stable name (also the matrix row label).
    pub name: &'static str,
    /// File name including the extension that selects the reader.
    pub file: String,
    pub bytes: Vec<u8>,
}

fn sample(name: &'static str, file: &str, bytes: Vec<u8>) -> Sample {
    Sample {
        name,
        file: file.to_string(),
        bytes,
    }
}

const WORDS: &[&str] = &[
    "river", "lantern", "autumn", "whisper", "copper", "harbour", "violet", "meadow", "thunder",
    "marble", "garden", "compass", "feather", "winter", "silver", "orchard", "candle", "shadow",
    "bridge", "falcon", "ember", "willow", "granite", "voyage", "mirror", "saddle", "anchor",
    "velvet", "pebble", "horizon", "harvest", "lullaby", "citadel", "monsoon", "tapestry",
    "journey", "quartz", "cobweb", "sparrow", "glacier",
];

/// Deterministic prose: `n` words, sentence-cased with punctuation, starting at `seed`.
pub fn prose(seed: u32, n: usize) -> String {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(12345);
    let mut out = String::new();
    let mut start = true;
    for i in 0..n {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        let w = WORDS[(x >> 16) as usize % WORDS.len()];
        if start {
            let mut c = w.chars();
            out.extend(c.next().unwrap().to_uppercase());
            out.push_str(c.as_str());
            start = false;
        } else {
            out.push_str(w);
        }
        if i % 11 == 10 {
            out.push_str(". ");
            start = true;
        } else if i % 5 == 4 {
            out.push_str(", ");
        } else {
            out.push(' ');
        }
    }
    out.trim_end().trim_end_matches(',').to_string() + "."
}

pub fn zip_stored_first(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default();
    for (i, (name, data)) in files.iter().enumerate() {
        let opt = if i == 0 && *name == "mimetype" {
            stored
        } else {
            deflated
        };
        z.start_file(*name, opt).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

const XHTML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>t</title><link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/></head><body>\n";

fn page(body: &str) -> Vec<u8> {
    format!("{XHTML_HEAD}{body}\n</body></html>").into_bytes()
}

struct EpubSpec {
    version: u8,
    title: &'static str,
    lang: &'static str,
    extra_meta: String,
    /// (file, bytes, media type, properties)
    items: Vec<(String, Vec<u8>, &'static str, &'static str)>,
    spine: Vec<String>,
    nav: Option<String>,
    ncx: Option<String>,
    cover_meta: Option<String>,
    guide: String,
    rtl: bool,
}

fn build_epub(s: EpubSpec) -> Vec<u8> {
    let mut manifest = String::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, (f, data, mt, props)) in s.items.iter().enumerate() {
        let props = if props.is_empty() {
            String::new()
        } else {
            format!(" properties=\"{props}\"")
        };
        manifest.push_str(&format!(
            "<item id=\"i{i}\" href=\"{f}\" media-type=\"{mt}\"{props}/>\n"
        ));
        files.push((format!("OEBPS/{f}"), data.clone()));
    }
    if s.nav.is_some() {
        manifest
            .push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    }
    if s.ncx.is_some() {
        manifest.push_str(
            "<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n",
        );
    }
    let spine: String = s
        .spine
        .iter()
        .map(|f| {
            let idx = s.items.iter().position(|i| &i.0 == f).unwrap();
            format!("<itemref idref=\"i{idx}\"/>")
        })
        .collect();
    let toc_attr = if s.ncx.is_some() { " toc=\"ncx\"" } else { "" };
    let dir = if s.rtl {
        " page-progression-direction=\"rtl\""
    } else {
        ""
    };
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"{v}.0\" unique-id=\"bid\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\"><dc:identifier id=\"bid\">urn:uuid:12345678-1234-1234-1234-123456789abc</dc:identifier><dc:title>{t}</dc:title><dc:language>{l}</dc:language><dc:creator opf:role=\"aut\">Ada Lovelace</dc:creator>{x}{c}<meta property=\"dcterms:modified\">2020-01-01T00:00:00Z</meta></metadata><manifest>{m}</manifest><spine{ta}{dir}>{sp}</spine>{g}</package>",
        v = s.version,
        t = s.title,
        l = s.lang,
        x = s.extra_meta,
        c = s.cover_meta.unwrap_or_default(),
        m = manifest,
        ta = toc_attr,
        sp = spine,
        g = s.guide
    );
    let mut all: Vec<(&str, Vec<u8>)> = vec![
        ("mimetype", b"application/epub+zip".to_vec()),
        (
            "META-INF/container.xml",
            b"<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>".to_vec(),
        ),
        ("OEBPS/content.opf", opf.into_bytes()),
    ];
    if let Some(n) = s.nav {
        all.push(("OEBPS/nav.xhtml", n.into_bytes()));
    }
    if let Some(n) = s.ncx {
        all.push(("OEBPS/toc.ncx", n.into_bytes()));
    }
    let owned: Vec<(String, Vec<u8>)> = files;
    let mut refs: Vec<(&str, Vec<u8>)> = all;
    for (n, d) in &owned {
        refs.push((n.as_str(), d.clone()));
    }
    zip_stored_first(&refs)
}

fn nav_doc(entries: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body><nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1><ol>{entries}</ol></nav></body></html>")
}

fn ncx_doc(points: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"urn:uuid:12345678-1234-1234-1234-123456789abc\"/></head><docTitle><text>t</text></docTitle><navMap>{points}</navMap></ncx>")
}

const CSS: &str = "body{margin:1em} p{text-indent:1em;margin:0} .c{text-align:center} .big{font-size:1.5em;font-weight:bold} blockquote{margin:1em 2em;font-style:italic} p:first-letter{font-size:2em} h1{page-break-before:always}";

fn css() -> Vec<u8> {
    CSS.as_bytes().to_vec()
}

fn xhtml_item(f: &str, body: &str) -> (String, Vec<u8>, &'static str, &'static str) {
    (f.to_string(), page(body), "application/xhtml+xml", "")
}

fn epub3_features() -> Sample {
    let c1 = format!(
        "<h1 id=\"c1\">The First Chapter</h1>\n<p>{} A note<a epub:type=\"noteref\" href=\"c2.xhtml#fn1\" id=\"r1\">1</a> sits here and <a href=\"c2.xhtml#sec\">a forward link</a>.</p>\n<p class=\"c\"><em>Centered emphasis</em> and <strong>strong</strong> with <sup>sup</sup>, <sub>sub</sub>, <code>mono text</code>.</p>\n<blockquote><p>A quoted line of wisdom.</p></blockquote>\n<ul><li>alpha<ul><li>alpha one</li><li>alpha two</li></ul></li><li>beta</li><li>gamma<ol><li>deep first</li><li>deep second</li></ol></li></ul>\n<hr/>\n<p>Entities: caf&#233; &amp; cr&#232;me &#8212; &#x2603; snowman &lt;tag&gt; and &#169; sign.</p>",
        prose(1, 60)
    );
    let c2 = format!(
        "<h1 id=\"c2\">The Second Chapter</h1>\n<h2 id=\"sec\">A Section</h2>\n<p>{}</p>\n<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>Alpha</td><td>1</td></tr><tr><td>Beta</td><td>2</td></tr></tbody></table>\n<p><img src=\"images/pic.png\" alt=\"A picture\"/></p>\n<pre>preformatted   spacing\n  kept here</pre>\n<aside epub:type=\"footnote\" id=\"fn1\"><p>The footnote text <a href=\"c1.xhtml#r1\">back</a>.</p></aside>",
        prose(2, 80)
    );
    let c3 = format!(
        "<h1 id=\"c3\">The Third Chapter</h1>\n<h2>Sub one</h2><p>{}</p><h3 id=\"deep\">Deeper</h3><p>{}</p><h2>Sub two</h2><p>{}</p>",
        prose(3, 40),
        prose(4, 40),
        prose(5, 40)
    );
    let nav = nav_doc("<li><a href=\"c1.xhtml\">The First Chapter</a></li><li><a href=\"c2.xhtml\">The Second Chapter</a><ol><li><a href=\"c2.xhtml#sec\">A Section</a></li></ol></li><li><a href=\"c3.xhtml\">The Third Chapter</a><ol><li><a href=\"c3.xhtml#deep\">Deeper</a></li></ol></li>");
    let items = vec![
        xhtml_item("c1.xhtml", &c1),
        xhtml_item("c2.xhtml", &c2),
        xhtml_item("c3.xhtml", &c3),
        ("style.css".into(), css(), "text/css", ""),
        ("images/pic.png".into(), png(60, 40), "image/png", ""),
        (
            "images/cover.jpg".into(),
            jpeg(300, 450),
            "image/jpeg",
            "cover-image",
        ),
    ];
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "Feature Book: A Test",
        lang: "en",
        extra_meta: "<dc:creator opf:role=\"aut\">Grace Hopper</dc:creator><dc:identifier opf:scheme=\"ISBN\">9781234567897</dc:identifier><dc:publisher>Bookx Press</dc:publisher><meta name=\"calibre:series\" content=\"Test Saga\"/><meta name=\"calibre:series_index\" content=\"2\"/>".into(),
        items,
        spine: vec!["c1.xhtml".into(), "c2.xhtml".into(), "c3.xhtml".into()],
        nav: Some(nav),
        ncx: None,
        cover_meta: Some("<meta name=\"cover\" content=\"i5\"/>".into()),
        guide: String::new(),
        rtl: false,
    });
    sample("epub3-features", "epub3-features.epub", bytes)
}

fn epub2_ncx() -> Sample {
    let mk = |i: u32, h: &str| {
        format!(
            "<h1 id=\"h{i}\">{h}</h1><p>{}</p><h2 id=\"s{i}\">Sub of {h}</h2><p>{}</p>",
            prose(10 + i, 50),
            prose(20 + i, 30)
        )
    };
    let items = vec![
        xhtml_item("part1.xhtml", &mk(1, "Part One")),
        xhtml_item("part2.xhtml", &mk(2, "Part Two")),
        xhtml_item("part3.xhtml", &mk(3, "Part Three")),
        ("style.css".into(), css(), "text/css", ""),
        ("cover.jpg".into(), jpeg(240, 360), "image/jpeg", ""),
    ];
    let ncx = ncx_doc("<navPoint id=\"n1\" playOrder=\"1\"><navLabel><text>Part One</text></navLabel><content src=\"part1.xhtml\"/><navPoint id=\"n1a\" playOrder=\"2\"><navLabel><text>Sub of Part One</text></navLabel><content src=\"part1.xhtml#s1\"/></navPoint></navPoint><navPoint id=\"n2\" playOrder=\"3\"><navLabel><text>Part Two</text></navLabel><content src=\"part2.xhtml\"/></navPoint><navPoint id=\"n3\" playOrder=\"4\"><navLabel><text>Part Three</text></navLabel><content src=\"part3.xhtml\"/><navPoint id=\"n3a\" playOrder=\"5\"><navLabel><text>Sub of Part Three</text></navLabel><content src=\"part3.xhtml#s3\"/></navPoint></navPoint>");
    let bytes = build_epub(EpubSpec {
        version: 2,
        title: "Old School NCX",
        lang: "en-GB",
        extra_meta: String::new(),
        items,
        spine: vec![
            "part1.xhtml".into(),
            "part2.xhtml".into(),
            "part3.xhtml".into(),
        ],
        nav: None,
        ncx: Some(ncx),
        cover_meta: Some("<meta name=\"cover\" content=\"i4\"/>".into()),
        guide: "<guide><reference type=\"cover\" title=\"Cover\" href=\"cover.jpg\"/></guide>"
            .into(),
        rtl: false,
    });
    sample("epub2-ncx", "epub2-ncx.epub", bytes)
}

fn epub_rtl() -> Sample {
    let ar = "كان يا ما كان في قديم الزمان ملك عادل يحكم مدينة جميلة على ضفة النهر";
    let he = "היה היה פעם מלך צדיק ששלט בעיר יפה על גדת הנהר";
    let c1 = format!("<h1 id=\"a\">الفصل الأول</h1><p dir=\"rtl\">{ar}.</p><p dir=\"rtl\">{he}.</p><p>{ar} {he}</p>");
    let c2 = format!("<h1 id=\"b\">الفصل الثاني</h1><p dir=\"rtl\">{he}. {ar}.</p><p dir=\"rtl\">رقم 123 داخل النص.</p>");
    let items = vec![
        xhtml_item("a.xhtml", &c1),
        xhtml_item("b.xhtml", &c2),
        ("style.css".into(), css(), "text/css", ""),
    ];
    let nav = nav_doc(
        "<li><a href=\"a.xhtml\">الفصل الأول</a></li><li><a href=\"b.xhtml\">الفصل الثاني</a></li>",
    );
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "كتاب الاختبار",
        lang: "ar",
        extra_meta: String::new(),
        items,
        spine: vec!["a.xhtml".into(), "b.xhtml".into()],
        nav: Some(nav),
        ncx: None,
        cover_meta: None,
        guide: String::new(),
        rtl: true,
    });
    sample("epub-rtl", "epub-rtl.epub", bytes)
}

fn epub_ruby_cjk() -> Sample {
    let c1 = "<h1 id=\"j\">第一章</h1><p><ruby>漢字<rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby>を読む。吾輩は猫である。名前はまだ無い。</p><p>どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。</p>";
    let c2 = "<h1 id=\"k\">第二章</h1><p>한국어 텍스트도 포함됩니다。中文文本也在这里。</p>";
    let items = vec![
        xhtml_item("j.xhtml", c1),
        xhtml_item("k.xhtml", c2),
        ("style.css".into(), css(), "text/css", ""),
    ];
    let nav =
        nav_doc("<li><a href=\"j.xhtml\">第一章</a></li><li><a href=\"k.xhtml\">第二章</a></li>");
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "吾輩は猫である",
        lang: "ja",
        extra_meta: String::new(),
        items,
        spine: vec!["j.xhtml".into(), "k.xhtml".into()],
        nav: Some(nav),
        ncx: None,
        cover_meta: None,
        guide: String::new(),
        rtl: false,
    });
    sample("epub-ruby-cjk", "epub-ruby-cjk.epub", bytes)
}

fn epub_svg_cover() -> Sample {
    let cover_page = "<div style=\"text-align:center\"><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" viewBox=\"0 0 300 450\" width=\"100%\" height=\"100%\" preserveAspectRatio=\"none\"><image width=\"300\" height=\"450\" xlink:href=\"cover.jpg\"/></svg></div>";
    let c1 = format!("<h1 id=\"c\">Only Chapter</h1><p>{}</p>", prose(30, 60));
    let items = vec![
        xhtml_item("cover.xhtml", cover_page),
        xhtml_item("c.xhtml", &c1),
        (
            "cover.jpg".into(),
            jpeg(300, 450),
            "image/jpeg",
            "cover-image",
        ),
        ("style.css".into(), css(), "text/css", ""),
    ];
    let nav = nav_doc("<li><a href=\"c.xhtml\">Only Chapter</a></li>");
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "SVG Cover Book",
        lang: "en",
        extra_meta: String::new(),
        items,
        spine: vec!["cover.xhtml".into(), "c.xhtml".into()],
        nav: Some(nav),
        ncx: None,
        cover_meta: Some("<meta name=\"cover\" content=\"i2\"/>".into()),
        guide: String::new(),
        rtl: false,
    });
    sample("epub-svg-cover", "epub-svg-cover.epub", bytes)
}

fn epub_broken_xhtml() -> Sample {
    // tag soup: unclosed p, mismatched tags, bare ampersands, HTML entities not declared in XML
    let body = format!(
        "<h1 id=\"x\">Messy Chapter</h1>\n<p>First paragraph with a bare & ampersand and&nbsp;a non-breaking space &copy; &hellip; done\n<p>Second paragraph <b>bold <i>both</b> italic?</i> and text after.\n<p>{}\n<br>\n<img src=\"images/pic.png\" alt=\"inline\">\n<ul><li>one<li>two<li>three</ul>",
        prose(40, 40)
    );
    let raw = format!("<html><head><title>x</title></head><body>{body}</body></html>");
    let items = vec![
        (
            "messy.xhtml".to_string(),
            raw.into_bytes(),
            "application/xhtml+xml",
            "",
        ),
        ("images/pic.png".into(), png(30, 30), "image/png", ""),
    ];
    let nav = nav_doc("<li><a href=\"messy.xhtml#x\">Messy Chapter</a></li>");
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "Broken XHTML",
        lang: "en",
        extra_meta: String::new(),
        items,
        spine: vec!["messy.xhtml".into()],
        nav: Some(nav),
        ncx: None,
        cover_meta: None,
        guide: String::new(),
        rtl: false,
    });
    sample("epub-broken-xhtml", "epub-broken-xhtml.epub", bytes)
}

fn epub_long_toc() -> Sample {
    let mut items = Vec::new();
    let mut spine = Vec::new();
    let mut nav = String::new();
    for i in 1..=300u32 {
        let f = format!("ch{i:03}.xhtml");
        items.push(xhtml_item(
            &f,
            &format!("<h1 id=\"h\">Chapter {i}</h1><p>{}</p>", prose(100 + i, 25)),
        ));
        nav.push_str(&format!("<li><a href=\"{f}\">Chapter {i}</a></li>"));
        spine.push(f);
    }
    items.push(("style.css".into(), css(), "text/css", ""));
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "Three Hundred Chapters",
        lang: "en",
        extra_meta: String::new(),
        items,
        spine,
        nav: Some(nav_doc(&nav)),
        ncx: None,
        cover_meta: None,
        guide: String::new(),
        rtl: false,
    });
    sample("epub-long-toc", "epub-long-toc.epub", bytes)
}

fn epub_css_edge() -> Sample {
    let css_text = "@charset \"utf-8\"; body{font-family:Georgia,serif;line-height:1.4} .dropcap::first-letter{font-size:3em;float:left} p.indent{text-indent:2em} .sc{font-variant:small-caps} .rot{transform:rotate(3deg)} @media amzn-mobi{p{margin:0}} @media (max-width:600px){p{font-size:90%}} h1{font:bold 2em/1.1 serif;margin:0 0 1em} .u{text-decoration:underline} .s{text-decoration:line-through} a[href^=http]{color:#00f} table{border-collapse:collapse} td{border:1px solid #000;padding:2px}";
    let body = format!(
        "<h1 id=\"t\">Styled Chapter</h1><p class=\"dropcap\">{}</p><p class=\"indent sc\">Small caps and <span class=\"u\">underline</span> and <span class=\"s\">strike</span>.</p><p style=\"color:red;font-size:20px\">Inline style paragraph.</p><div class=\"rot\">Rotated block of words.</div><table><tr><td rowspan=\"2\">tall</td><td colspan=\"2\">wide</td></tr><tr><td>x</td><td>y</td></tr></table>",
        prose(50, 30)
    );
    let items = vec![
        xhtml_item("s.xhtml", &body),
        (
            "style.css".into(),
            css_text.as_bytes().to_vec(),
            "text/css",
            "",
        ),
    ];
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "CSS Edge Cases",
        lang: "en",
        extra_meta: String::new(),
        items,
        spine: vec!["s.xhtml".into()],
        nav: Some(nav_doc("<li><a href=\"s.xhtml\">Styled Chapter</a></li>")),
        ncx: None,
        cover_meta: None,
        guide: String::new(),
        rtl: false,
    });
    sample("epub-css-edge", "epub-css-edge.epub", bytes)
}

fn epub_big_image() -> Sample {
    let body = format!(
        "<h1 id=\"i\">Pictures</h1><p>{}</p><p><img src=\"big.png\" alt=\"big\"/></p><p><img src=\"wide.jpg\" alt=\"wide\"/></p>",
        prose(60, 20)
    );
    let items = vec![
        xhtml_item("p.xhtml", &body),
        ("big.png".into(), png(2400, 1800), "image/png", ""),
        ("wide.jpg".into(), jpeg(4000, 300), "image/jpeg", ""),
        ("style.css".into(), css(), "text/css", ""),
    ];
    let bytes = build_epub(EpubSpec {
        version: 3,
        title: "Huge Images",
        lang: "en",
        extra_meta: String::new(),
        items,
        spine: vec!["p.xhtml".into()],
        nav: Some(nav_doc("<li><a href=\"p.xhtml\">Pictures</a></li>")),
        ncx: None,
        cover_meta: None,
        guide: String::new(),
        rtl: false,
    });
    sample("epub-big-image", "epub-big-image.epub", bytes)
}

fn fb2_full() -> Sample {
    use base64::Engine;
    let b64 = |d: &[u8]| base64::engine::general_purpose::STANDARD.encode(d);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\" xmlns:l=\"http://www.w3.org/1999/xlink\"><description><title-info><genre>sf</genre><author><first-name>Ivan</first-name><middle-name>Petrovich</middle-name><last-name>Ivanov</last-name></author><book-title>Таинственный остров</book-title><annotation><p>An annotation with <emphasis>emphasis</emphasis>.</p></annotation><date value=\"2001-02-03\">2001</date><coverpage><image l:href=\"#cover.jpg\"/></coverpage><lang>ru</lang><sequence name=\"Saga FB2\" number=\"3\"/></title-info><document-info><author><nickname>scanner</nickname></author><date>2020-01-01</date><id>fb2-test-id-1</id><version>1.0</version></document-info><publish-info><publisher>Издательство</publisher><isbn>978-5-17-123456-7</isbn></publish-info></description><body><title><p>Таинственный остров</p></title><epigraph><p>An epigraph line.</p><text-author>Someone</text-author></epigraph><section id=\"s1\"><title><p>Глава первая</p></title><p>{p1} Сноска<a l:href=\"#n1\" type=\"note\">[1]</a> здесь.</p><empty-line/><p>{p2}</p><poem><title><p>Poem title</p></title><stanza><v>First verse line</v><v>Second verse line</v></stanza><stanza><v>Third verse</v><v>Fourth verse</v></stanza><text-author>Poet</text-author></poem><cite><p>A cited paragraph.</p><text-author>Cited</text-author></cite><p><image l:href=\"#pic.png\"/></p></section><section id=\"s2\"><title><p>Глава вторая</p></title><section id=\"s2a\"><title><p>Подглава</p></title><p>{p3} <strong>Strong</strong> <emphasis>em</emphasis> <strikethrough>struck</strikethrough> <sub>sub</sub><sup>sup</sup> <code>code</code></p></section><table><tr><th>H1</th><th>H2</th></tr><tr><td>c1</td><td>c2</td></tr></table></section></body><body name=\"notes\"><title><p>Notes</p></title><section id=\"n1\"><title><p>1</p></title><p>The note body text.</p></section></body><binary id=\"cover.jpg\" content-type=\"image/jpeg\">{cov}</binary><binary id=\"pic.png\" content-type=\"image/png\">{pic}</binary></FictionBook>",
        p1 = prose(70, 30),
        p2 = prose(71, 30),
        p3 = prose(72, 30),
        cov = b64(&jpeg(200, 300)),
        pic = b64(&png(50, 50)),
    );
    sample("fb2-full", "fb2-full.fb2", xml.into_bytes())
}

fn fb2_koi8() -> Sample {
    // declared windows-1251, Russian text
    let xml = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\n<FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\"><description><title-info><genre>prose</genre><author><first-name>Лев</first-name><last-name>Толстой</last-name></author><book-title>Война и мир</book-title><lang>ru</lang></title-info></description><body><section><title><p>Часть первая</p></title><p>Все смешалось в доме Облонских. Жена узнала, что муж был в связи с бывшею в их доме француженкою-гувернанткой.</p></section></body></FictionBook>";
    let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(xml);
    sample("fb2-cp1251", "fb2-cp1251.fb2", bytes.into_owned())
}

fn zip_files(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    zip_stored_first(files)
}

fn docx_full() -> Sample {
    let ct = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>";
    let core = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Docx Feature Test</dc:title><dc:creator>Word Author</dc:creator><dc:language>en-US</dc:language></cp:coreProperties>";
    let drels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdS\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rIdF\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes\" Target=\"footnotes.xml\"/><Relationship Id=\"rIdI\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image1.png\"/><Relationship Id=\"rIdL\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"https://example.org/page\" TargetMode=\"External\"/></Relationships>";
    let styles = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/></w:style><w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/></w:style></w:styles>";
    let fns = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:footnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id=\"1\"><w:p><w:r><w:t>Footnote body words here.</w:t></w:r></w:p></w:footnote></w:footnotes>";
    let p = |style: &str, text: &str| {
        format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>")
    };
    let doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{h1}<w:p><w:r><w:t xml:space=\"preserve\">{t1} </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t xml:space=\"preserve\"> and </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>italic</w:t></w:r><w:r><w:t xml:space=\"preserve\"> and </w:t></w:r><w:r><w:rPr><w:u w:val=\"single\"/></w:rPr><w:t>underlined</w:t></w:r><w:r><w:footnoteReference w:id=\"1\"/></w:r><w:r><w:t xml:space=\"preserve\"> then a </w:t></w:r><w:hyperlink r:id=\"rIdL\"><w:r><w:t>web link</w:t></w:r></w:hyperlink><w:r><w:t>.</w:t></w:r></w:p>{h2}<w:p><w:r><w:t>{t2}</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"3000\"/><w:gridCol w:w=\"3000\"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Cell A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Cell B1</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>Cell A2</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Cell B2</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:drawing><wp:inline><wp:extent cx=\"952500\" cy=\"952500\"/><wp:docPr id=\"1\" name=\"Picture 1\" descr=\"A docx picture\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:blipFill><a:blip r:embed=\"rIdI\"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>{l1}{l2}{h1b}<w:p><w:r><w:t>{t3}</w:t></w:r></w:p></w:body></w:document>",
        h1 = p("Heading1", "First Heading"),
        h2 = p("Heading2", "Second Level"),
        h1b = p("Heading1", "Another Heading"),
        l1 = "<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr><w:r><w:t>list item one</w:t></w:r></w:p>",
        l2 = "<w:p><w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr><w:r><w:t>list item two</w:t></w:r></w:p>",
        t1 = prose(80, 25),
        t2 = prose(81, 25),
        t3 = prose(82, 25),
    );
    let bytes = zip_files(&[
        ("[Content_Types].xml", ct.as_bytes().to_vec()),
        ("_rels/.rels", rels.as_bytes().to_vec()),
        ("docProps/core.xml", core.as_bytes().to_vec()),
        ("word/_rels/document.xml.rels", drels.as_bytes().to_vec()),
        ("word/document.xml", doc.into_bytes()),
        ("word/styles.xml", styles.as_bytes().to_vec()),
        ("word/footnotes.xml", fns.as_bytes().to_vec()),
        ("word/media/image1.png", png(50, 50)),
    ]);
    sample("docx-full", "docx-full.docx", bytes)
}

fn odt_full() -> Sample {
    let manifest = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\"><manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"Pictures/p1.png\" manifest:media-type=\"image/png\"/></manifest:manifest>";
    let meta = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\"><office:meta><dc:title>ODT Feature Test</dc:title><dc:creator>Writer Author</dc:creator><dc:language>de</dc:language></office:meta></office:document-meta>";
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\"><office:automatic-styles><style:style style:name=\"B\" style:family=\"text\"><style:text-properties fo:font-weight=\"bold\"/></style:style></office:automatic-styles><office:body><office:text><text:h text:outline-level=\"1\">Erstes Kapitel</text:h><text:p>{t1} Mit <text:span text:style-name=\"B\">fett</text:span> und Umlauten äöüß.</text:p><text:h text:outline-level=\"2\">Unterkapitel</text:h><text:p>{t2}</text:p><text:list><text:list-item><text:p>Punkt eins</text:p></text:list-item><text:list-item><text:p>Punkt zwei</text:p></text:list-item></text:list><table:table><table:table-row><table:table-cell><text:p>Zelle A1</text:p></table:table-cell><table:table-cell><text:p>Zelle B1</text:p></table:table-cell></table:table-row></table:table><text:p><draw:frame><draw:image xlink:href=\"Pictures/p1.png\"/></draw:frame></text:p><text:h text:outline-level=\"1\">Zweites Kapitel</text:h><text:p>{t3} <text:a xlink:href=\"https://example.org/\">Link</text:a></text:p></office:text></office:body></office:document-content>",
        t1 = prose(90, 25),
        t2 = prose(91, 25),
        t3 = prose(92, 25)
    );
    let bytes = zip_files(&[
        (
            "mimetype",
            b"application/vnd.oasis.opendocument.text".to_vec(),
        ),
        ("META-INF/manifest.xml", manifest.as_bytes().to_vec()),
        ("meta.xml", meta.as_bytes().to_vec()),
        ("content.xml", content.into_bytes()),
        ("Pictures/p1.png", png(40, 40)),
    ]);
    sample("odt-full", "odt-full.odt", bytes)
}

fn rtf_full() -> Sample {
    let rtf = format!(
        "{{\\rtf1\\ansi\\ansicpg1252\\deff0{{\\fonttbl{{\\f0\\fswiss Arial;}}{{\\f1\\fmodern Courier New;}}}}{{\\info{{\\title RTF Feature Test}}{{\\author Rich Author}}}}\\pard\\b\\fs36 First Heading\\b0\\fs24\\par\\pard {t1} \\b bold\\b0  and \\i italic\\i0  and \\ul underline\\ulnone . Caf\\'e9 na\\'efve \\u8212? dash \\u8364? euro \\u26085?\\u26412? kanji.\\par {t2}\\par{{\\pard\\f1 monospace line\\par}}\\trowd\\cellx3000\\cellx6000\\pard\\intbl Cell one\\cell Cell two\\cell\\row\\trowd\\cellx3000\\cellx6000\\pard\\intbl Cell three\\cell Cell four\\cell\\row\\pard\\b\\fs36 Second Heading\\b0\\fs24\\par {t3}\\par}}",
        t1 = prose(95, 25),
        t2 = prose(96, 25),
        t3 = prose(97, 25)
    );
    sample("rtf-full", "rtf-full.rtf", rtf.into_bytes())
}

fn txt_samples() -> Vec<Sample> {
    let body = format!(
        "Chapter 1\n\n{}\n\n{}\n\nChapter 2\n\n{}\n\nCafé naïve résumé — “quoted” text… done.\n\n* * *\n\n{}\n",
        prose(110, 40),
        prose(111, 40),
        prose(112, 40),
        prose(113, 20)
    );
    let mut v = vec![sample(
        "txt-utf8",
        "txt-utf8.txt",
        body.clone().into_bytes(),
    )];
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend(body.as_bytes());
    v.push(sample("txt-utf8-bom", "txt-utf8-bom.txt", bom));
    let mut u16le = vec![0xFF, 0xFE];
    for u in body.encode_utf16() {
        u16le.extend(u.to_le_bytes());
    }
    v.push(sample("txt-utf16le", "txt-utf16le.txt", u16le));
    let (cp, _, _) = encoding_rs::WINDOWS_1252.encode(&body);
    v.push(sample("txt-cp1252", "txt-cp1252.txt", cp.into_owned()));
    let ru = "Глава 1\n\nВсе смешалось в доме Облонских. Жена узнала, что муж был в связи с бывшею в их доме француженкою-гувернанткой, и объявила мужу, что не может жить с ним в одном доме.\n\nГлава 2\n\nПоложение это продолжалось уже третий день и мучительно чувствовалось и самими супругами, и всеми членами семьи.\n";
    let (cp, _, _) = encoding_rs::WINDOWS_1251.encode(ru);
    v.push(sample("txt-cp1251", "txt-cp1251.txt", cp.into_owned()));
    let hard = format!(
        "{}\r\n",
        prose(114, 60)
            .split(' ')
            .collect::<Vec<_>>()
            .chunks(9)
            .map(|c| c.join(" "))
            .collect::<Vec<_>>()
            .join("\r\n")
    );
    v.push(sample(
        "txt-hardwrap-crlf",
        "txt-hardwrap-crlf.txt",
        hard.into_bytes(),
    ));
    v
}

fn markdown() -> Sample {
    let md = format!(
        "---\ntitle: Markdown Feature Test\nauthor: Mark Down\n---\n\n# First Heading\n\n{}\n\n## Second Level\n\nA paragraph with *emphasis*, **strong**, `code`, and a [link](https://example.org) plus a footnote.[^1]\n\n- item one\n  - nested a\n  - nested b\n- item two\n\n1. first\n2. second\n\n> A block quote line.\n\n| Col A | Col B |\n|-------|-------|\n| a1    | b1    |\n| a2    | b2    |\n\n```\ncode block line\n```\n\n# Last Heading\n\n{}\n\n[^1]: The footnote text.\n",
        prose(120, 40),
        prose(121, 40)
    );
    sample("markdown", "markdown.md", md.into_bytes())
}

fn html_resources() -> Vec<Sample> {
    let html = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>HTML Feature Test</title><link rel=\"stylesheet\" href=\"css/site.css\"><meta name=\"author\" content=\"Web Author\"></head><body><h1 id=\"top\">HTML Heading</h1><p>{} <a href=\"#end\">jump to end</a></p><p><img src=\"img/pic.png\" alt=\"relative picture\"></p><h2>Middle</h2><p>{}</p><ul><li>one</li><li>two</li></ul><h1 id=\"end\">Final Heading</h1><p>{}</p></body></html>",
        prose(130, 40),
        prose(131, 40),
        prose(132, 40)
    );
    // an .html alone has its relative resources missing; the zipped form is the HTMLZ below
    let htmlz = zip_files(&[
        ("index.html", html.clone().into_bytes()),
        ("css/site.css", b"h1{color:#333} p{margin:0 0 1em}".to_vec()),
        ("img/pic.png", png(40, 40)),
        (
            "metadata.opf",
            b"<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>HTML Feature Test</dc:title><dc:creator>Web Author</dc:creator><dc:language>en</dc:language></metadata></package>".to_vec(),
        ),
    ]);
    vec![
        sample("html-single", "html-single.html", html.into_bytes()),
        sample("htmlz-resources", "htmlz-resources.htmlz", htmlz),
    ]
}

fn cbz() -> Sample {
    let files: Vec<(String, Vec<u8>)> = (1..=5)
        .map(|i| (format!("page{i:02}.png"), png(120 + i * 3, 180)))
        .collect();
    let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, d)| (n.as_str(), d.clone())).collect();
    sample("cbz", "cbz.cbz", zip_files(&refs))
}

/// All synthetic documents.
pub fn all() -> Vec<Sample> {
    let mut v = vec![
        epub3_features(),
        epub2_ncx(),
        epub_rtl(),
        epub_ruby_cjk(),
        epub_svg_cover(),
        epub_broken_xhtml(),
        epub_long_toc(),
        epub_css_edge(),
        epub_big_image(),
        fb2_full(),
        fb2_koi8(),
        docx_full(),
        odt_full(),
        rtf_full(),
        markdown(),
        cbz(),
    ];
    v.extend(txt_samples());
    v.extend(html_resources());
    v
}

/// The subset that is small and fast (used by the CI-safe tests).
pub fn by_name(name: &str) -> Sample {
    all()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no corpus sample {name}"))
}
