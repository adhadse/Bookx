//! Format conversion: EPUB -> TXT / Markdown / HTML and TXT / Markdown / HTML -> EPUB.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::archive::Archive;
use super::book::OpenBook;
use super::doc::*;
use super::html::parse_html;
use super::opf::{Identifier, Metadata};
use super::util::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Txt,
    Markdown,
    Html,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [Self::Txt, Self::Markdown, Self::Html];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Txt => "Plain text (TXT)",
            Self::Markdown => "Markdown (MD)",
            Self::Html => "HTML",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Markdown => "md",
            Self::Html => "html",
        }
    }
}

pub fn export_book(book: &OpenBook, fmt: ExportFormat) -> String {
    let md = book.metadata();
    let mut out = String::new();
    match fmt {
        ExportFormat::Txt => {
            out.push_str(&format!("{}\n{}\n\n", md.title, md.authors_display()));
        }
        ExportFormat::Markdown => {
            out.push_str(&format!("---\ntitle: \"{}\"\n", md.title.replace('"', "'")));
            out.push_str(&format!(
                "author: \"{}\"\n---\n\n",
                md.authors_display().replace('"', "'")
            ));
        }
        ExportFormat::Html => {
            out.push_str(&format!(
                "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"/><title>{}</title>\n<style>body{{max-width:42em;margin:2em auto;font:1.1em/1.6 serif;padding:0 1em}}blockquote{{margin-left:2em;color:#555}}pre{{background:#f4f4f4;padding:.5em;overflow:auto}}</style></head><body>\n",
                xml_escape(&md.title)
            ));
        }
    }
    for path in book.spine_paths() {
        let Some(doc) = book.chapter_doc(&path) else {
            continue;
        };
        match fmt {
            ExportFormat::Txt => out.push_str(&doc.to_text()),
            ExportFormat::Markdown => out.push_str(&doc.to_markdown()),
            ExportFormat::Html => out.push_str(&doc_to_xhtml(&doc)),
        }
        out.push('\n');
    }
    if fmt == ExportFormat::Html {
        out.push_str("</body></html>\n");
    }
    out
}

fn span_html(sp: &Span, out: &mut String) {
    match sp {
        Span::Text { text, style, link } => {
            let mut s = xml_escape(text);
            if style.code {
                s = format!("<code>{s}</code>");
            }
            if style.bold {
                s = format!("<strong>{s}</strong>");
            }
            if style.italic {
                s = format!("<em>{s}</em>");
            }
            if style.underline {
                s = format!("<u>{s}</u>");
            }
            if style.strike {
                s = format!("<del>{s}</del>");
            }
            if style.sup {
                s = format!("<sup>{s}</sup>");
            }
            if style.sub {
                s = format!("<sub>{s}</sub>");
            }
            if let Some(l) = link {
                s = format!("<a href=\"{}\">{s}</a>", xml_escape(l));
            }
            out.push_str(&s);
        }
        Span::Break => out.push_str("<br/>"),
        Span::Image { .. } | Span::Anchor(_) => {}
    }
}

/// Serialises blocks as XHTML-compatible markup (no `<html>` wrapper).
pub fn doc_to_xhtml(doc: &Doc) -> String {
    let mut out = String::new();
    for b in &doc.blocks {
        let mut inner = String::new();
        for sp in &b.spans {
            span_html(sp, &mut inner);
        }
        let style = match b.align {
            Align::Center => " style=\"text-align:center\"",
            Align::End => " style=\"text-align:right\"",
            Align::Start => "",
        };
        match b.kind {
            BlockKind::Rule => out.push_str("<hr/>\n"),
            BlockKind::Heading(n) => {
                let n = n.clamp(1, 6);
                out.push_str(&format!("<h{n}{style}>{inner}</h{n}>\n"))
            }
            BlockKind::Code => out.push_str(&format!("<pre>{inner}</pre>\n")),
            BlockKind::Quote => out.push_str(&format!("<blockquote><p>{inner}</p></blockquote>\n")),
            BlockKind::Paragraph => {
                if let Some(m) = &b.marker {
                    out.push_str(&format!("<p class=\"li\">{} {inner}</p>\n", xml_escape(m)));
                } else {
                    out.push_str(&format!("<p{style}>{inner}</p>\n"));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

pub struct Chapter {
    pub title: String,
    pub body: String,
}

pub fn is_importable(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("txt" | "md" | "markdown" | "html" | "htm" | "xhtml")
    )
}

pub fn import_to_epub(path: &Path) -> Result<Archive> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut md = Metadata {
        title: stem,
        language: "en".into(),
        ..Default::default()
    };
    let chapters = match ext.as_str() {
        "txt" => text_chapters(&text),
        "md" | "markdown" => markdown_chapters(&text, &mut md),
        "html" | "htm" | "xhtml" => {
            let doc = parse_html(&text, "import.html");
            let title = crate::util::parse_html_document(&text)
                .select(&scraper::Selector::parse("title").unwrap())
                .next()
                .map(|t| t.text().collect::<String>().trim().to_string())
                .filter(|t| !t.is_empty());
            if let Some(t) = title {
                md.title = t;
            }
            vec![Chapter {
                title: md.title.clone(),
                body: doc_to_xhtml(&doc),
            }]
        }
        _ => bail!("unsupported import format: .{ext}"),
    };
    Ok(create_epub(&md, &chapters, None))
}

fn text_chapters(text: &str) -> Vec<Chapter> {
    let heading =
        regex::Regex::new(r"(?i)^\s*(chapter|part|book|prologue|epilogue)\b.{0,60}$").unwrap();
    let mut chapters: Vec<(String, Vec<&str>)> = vec![("Beginning".into(), vec![])];
    for line in text.lines() {
        if heading.is_match(line) && line.trim().len() > 2 {
            chapters.push((line.trim().to_string(), vec![]));
        } else {
            chapters.last_mut().unwrap().1.push(line);
        }
    }
    let mut out = Vec::new();
    for (i, (title, lines)) in chapters.into_iter().enumerate() {
        let mut body = String::new();
        let mut para: Vec<&str> = Vec::new();
        let flush = |para: &mut Vec<&str>, body: &mut String| {
            if !para.is_empty() {
                body.push_str(&format!(
                    "<p>{}</p>\n",
                    xml_escape(&para.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" "))
                ));
                para.clear();
            }
        };
        for l in lines {
            if l.trim().is_empty() {
                flush(&mut para, &mut body);
            } else {
                para.push(l);
            }
        }
        flush(&mut para, &mut body);
        if body.is_empty() && i == 0 {
            continue;
        }
        let title = if i == 0 {
            "Beginning".to_string()
        } else {
            title
        };
        if i > 0 {
            body = format!("<h2>{}</h2>\n{}", xml_escape(&title), body);
        }
        out.push(Chapter { title, body });
    }
    if out.is_empty() {
        out.push(Chapter {
            title: "Text".into(),
            body: "<p></p>".into(),
        });
    }
    out
}

fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut html_out = String::new();
    let parser = Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    html::push_html(&mut html_out, parser);
    // pulldown emits HTML5; normalise the void tags so the result is valid XHTML.
    html_out.replace("<hr>", "<hr/>").replace("<br>", "<br/>")
}

fn markdown_chapters(text: &str, md: &mut Metadata) -> Vec<Chapter> {
    let mut body = text;
    // Optional front matter.
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some((front, after)) = rest.split_once("\n---") {
            for line in front.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim().trim_matches('"').to_string();
                    match k.trim() {
                        "title" => md.title = v,
                        "author" => md.authors = vec![v],
                        "language" | "lang" => md.language = v,
                        _ => {}
                    }
                }
            }
            body = after.trim_start_matches('\n');
        }
    }
    let mut chunks: Vec<(String, String)> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if !in_fence && line.starts_with("# ") {
            chunks.push((line[2..].trim().to_string(), String::new()));
        }
        if chunks.is_empty() {
            chunks.push((md.title.clone(), String::new()));
        }
        let last = chunks.last_mut().unwrap();
        last.1.push_str(line);
        last.1.push('\n');
    }
    chunks
        .into_iter()
        .filter(|(_, b)| !b.trim().is_empty())
        .map(|(title, b)| Chapter {
            title,
            body: markdown_to_html(&b),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// EPUB creation
// ---------------------------------------------------------------------------

pub fn create_epub(md: &Metadata, chapters: &[Chapter], cover: Option<(Vec<u8>, &str)>) -> Archive {
    let mut a = Archive::default();
    a.set("mimetype", b"application/epub+zip".to_vec());
    a.set_string(
        "META-INF/container.xml",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n  <rootfiles>\n    <rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n  </rootfiles>\n</container>\n",
    );
    let uuid = uuid::Uuid::new_v4();
    let lang = if md.language.is_empty() {
        "en"
    } else {
        &md.language
    };

    a.set_string(
        "OEBPS/style.css",
        "body { font-family: serif; line-height: 1.5; margin: 5%; }\nh1, h2, h3 { line-height: 1.2; }\np { margin: 0 0 .8em 0; text-align: justify; }\nblockquote { margin: 1em 2em; font-style: italic; }\npre, code { font-family: monospace; }\n",
    );

    let mut manifest = String::new();
    let mut spine = String::new();
    let mut nav_items = String::new();
    let mut ncx_points = String::new();
    for (i, ch) in chapters.iter().enumerate() {
        let n = i + 1;
        let file = format!("chapter-{n}.xhtml");
        let xhtml = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"{lang}\">\n<head>\n  <meta charset=\"utf-8\"/>\n  <title>{t}</title>\n  <link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n{body}</body>\n</html>\n",
            t = xml_escape(&ch.title),
            body = ch.body
        );
        a.set_string(&format!("OEBPS/{file}"), &xhtml);
        manifest.push_str(&format!(
            "    <item id=\"ch{n}\" href=\"{file}\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        spine.push_str(&format!("    <itemref idref=\"ch{n}\"/>\n"));
        nav_items.push_str(&format!(
            "      <li><a href=\"{file}\">{}</a></li>\n",
            xml_escape(&ch.title)
        ));
        ncx_points.push_str(&format!(
            "    <navPoint id=\"np{n}\" playOrder=\"{n}\"><navLabel><text>{}</text></navLabel><content src=\"{file}\"/></navPoint>\n",
            xml_escape(&ch.title)
        ));
    }

    let mut cover_meta = String::new();
    if let Some((data, ext)) = cover {
        let ext = if ext == "jpeg" { "jpg" } else { ext };
        let file = format!("images/cover.{ext}");
        a.set(&format!("OEBPS/{file}"), data);
        manifest.push_str(&format!(
            "    <item id=\"cover-image\" href=\"{file}\" media-type=\"{}\" properties=\"cover-image\"/>\n",
            media_type_for(&file)
        ));
        cover_meta = "    <meta name=\"cover\" content=\"cover-image\"/>\n".into();
    }

    a.set_string(
        "OEBPS/nav.xhtml",
        &format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n<head><title>Contents</title></head>\n<body>\n  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n    <ol>\n{nav_items}    </ol>\n  </nav>\n</body>\n</html>\n"
        ),
    );
    a.set_string(
        "OEBPS/toc.ncx",
        &format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n  <head>\n    <meta name=\"dtb:uid\" content=\"urn:uuid:{uuid}\"/>\n    <meta name=\"dtb:depth\" content=\"1\"/>\n  </head>\n  <docTitle><text>{}</text></docTitle>\n  <navMap>\n{ncx_points}  </navMap>\n</ncx>\n",
            xml_escape(&md.title)
        ),
    );

    let mut meta_xml = String::new();
    meta_xml.push_str(&format!(
        "    <dc:identifier id=\"bookid\">urn:uuid:{uuid}</dc:identifier>\n    <dc:title>{}</dc:title>\n    <dc:language>{}</dc:language>\n",
        xml_escape(&md.title),
        xml_escape(lang)
    ));
    for au in &md.authors {
        meta_xml.push_str(&format!(
            "    <dc:creator opf:role=\"aut\">{}</dc:creator>\n",
            xml_escape(au)
        ));
    }
    if !md.publisher.is_empty() {
        meta_xml.push_str(&format!(
            "    <dc:publisher>{}</dc:publisher>\n",
            xml_escape(&md.publisher)
        ));
    }
    for id in md.identifiers.iter().filter(|i| i.xml_id.is_none()) {
        let Identifier { scheme, value, .. } = id;
        meta_xml.push_str(&format!(
            "    <dc:identifier opf:scheme=\"{}\">{}</dc:identifier>\n",
            xml_escape(scheme),
            xml_escape(value)
        ));
    }
    meta_xml.push_str(&cover_meta);
    meta_xml.push_str(&format!(
        "    <meta property=\"dcterms:modified\">{}</meta>\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    ));

    a.set_string(
        "OEBPS/content.opf",
        &format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n{meta_xml}  </metadata>\n  <manifest>\n    <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n    <item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n    <item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n{manifest}  </manifest>\n  <spine toc=\"ncx\">\n{spine}  </spine>\n</package>\n"
        ),
    );
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::OpenBook;

    #[test]
    fn markdown_roundtrip() {
        let mut md = Metadata {
            title: "T".into(),
            ..Default::default()
        };
        let ch = markdown_chapters("# One\n\nHello *there*\n\n# Two\n\n- a\n- b\n", &mut md);
        assert_eq!(ch.len(), 2);
        let arc = create_epub(&md, &ch, None);
        let bytes = arc.to_bytes().unwrap();
        let book = OpenBook::from_archive(Archive::from_bytes(bytes).unwrap()).unwrap();
        assert_eq!(book.spine_paths().len(), 2);
        assert_eq!(book.toc().len(), 2);
        let txt = export_book(&book, ExportFormat::Txt);
        assert!(txt.contains("Hello there"));
    }
}
