//! Writers for flat formats: TXT, Markdown, HTML, TXTZ and HTMLZ.

use std::collections::HashMap;
use std::io::{Cursor, Write};

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use bookx_core::convert::{export_book, ExportFormat};
use bookx_core::util::{
    extension, is_external, media_type_for, parent_dir, resolve_href, xml_escape,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::markup::{body_range, escape_attr, get_attr, set_attr, tag_info, tokenize, Tok};

macro_rules! plain_writer {
    ($name:ident, $id:literal, $ext:literal, $fmt:expr) => {
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;
        impl FormatWriter for $name {
            fn id(&self) -> &'static str {
                $id
            }
            fn extension(&self) -> &'static str {
                $ext
            }
            fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
                Ok(export_book(book, $fmt).into_bytes())
            }
        }
    };
}

plain_writer!(TxtWriter, "txt", "txt", ExportFormat::Txt);
plain_writer!(MarkdownWriter, "md", "md", ExportFormat::Markdown);
plain_writer!(HtmlWriter, "html", "html", ExportFormat::Html);

pub(crate) fn zip_files(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut out);
        for (name, data) in files {
            let method = if matches!(extension(name).as_str(), "jpg" | "jpeg" | "png" | "gif") {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(method),
            )?;
            zip.write_all(data)?;
        }
        zip.finish()?;
    }
    Ok(out.into_inner())
}

/// `(id, href, media-type)` of the files a flat package contains.
pub(crate) fn flat_opf(
    book: &OpenBook,
    files: &[(&str, &str, &str)],
    cover_id: Option<&str>,
) -> String {
    let md = book.metadata();
    let mut m = String::new();
    m.push_str(&format!(
        "    <dc:title>{}</dc:title>\n",
        xml_escape(if md.title.is_empty() {
            "Untitled"
        } else {
            &md.title
        })
    ));
    for a in &md.authors {
        m.push_str(&format!(
            "    <dc:creator opf:role=\"aut\">{}</dc:creator>\n",
            xml_escape(a)
        ));
    }
    m.push_str(&format!(
        "    <dc:language>{}</dc:language>\n",
        xml_escape(if md.language.is_empty() {
            "en"
        } else {
            &md.language
        })
    ));
    if !md.publisher.is_empty() {
        m.push_str(&format!(
            "    <dc:publisher>{}</dc:publisher>\n",
            xml_escape(&md.publisher)
        ));
    }
    if !md.description.is_empty() {
        m.push_str(&format!(
            "    <dc:description>{}</dc:description>\n",
            xml_escape(&md.description)
        ));
    }
    for t in &md.tags {
        m.push_str(&format!("    <dc:subject>{}</dc:subject>\n", xml_escape(t)));
    }
    if !md.published.is_empty() {
        m.push_str(&format!(
            "    <dc:date>{}</dc:date>\n",
            xml_escape(&md.published)
        ));
    }
    for i in &md.identifiers {
        let scheme = if i.scheme.is_empty() {
            "uuid"
        } else {
            &i.scheme
        };
        m.push_str(&format!(
            "    <dc:identifier opf:scheme=\"{}\">{}</dc:identifier>\n",
            xml_escape(scheme),
            xml_escape(&i.value)
        ));
    }
    if !md.series.is_empty() {
        m.push_str(&format!(
            "    <meta name=\"calibre:series\" content=\"{}\"/>\n    <meta name=\"calibre:series_index\" content=\"{}\"/>\n",
            xml_escape(&md.series),
            bookx_core::opf::format_index(md.series_index)
        ));
    }
    if let Some(c) = cover_id {
        m.push_str(&format!(
            "    <meta name=\"cover\" content=\"{}\"/>\n",
            xml_escape(c)
        ));
    }
    let man: String = files
        .iter()
        .map(|(id, href, media)| {
            format!(
                "    <item id=\"{id}\" href=\"{}\" media-type=\"{media}\"/>\n",
                xml_escape(href)
            )
        })
        .collect();
    let first = files.first().map(|f| f.0).unwrap_or("content");
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"uid\">\n  <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n    <dc:identifier id=\"uid\" opf:scheme=\"uuid\">{}</dc:identifier>\n{m}  </metadata>\n  <manifest>\n{man}  </manifest>\n  <spine>\n    <itemref idref=\"{first}\"/>\n  </spine>\n</package>\n",
        uuid::Uuid::new_v4()
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TxtzWriter;

impl FormatWriter for TxtzWriter {
    fn id(&self) -> &'static str {
        "txtz"
    }
    fn extension(&self) -> &'static str {
        "txtz"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let txt = export_book(book, ExportFormat::Txt);
        let mut files: Vec<(String, Vec<u8>)> = vec![("index.txt".into(), txt.into_bytes())];
        let mut listing: Vec<(&str, &str, &str)> = vec![("text", "index.txt", "text/plain")];
        let cover_name;
        let mut cover_id = None;
        if let (Some(path), Some(bytes)) = (book.package.cover_path(), book.cover_bytes()) {
            cover_name = format!("cover.{}", extension(&path));
            files.push((cover_name.clone(), bytes.to_vec()));
            listing.push(("cover", &cover_name, media_type_for(&path)));
            cover_id = Some("cover");
        }
        files.push((
            "metadata.opf".into(),
            flat_opf(book, &listing, cover_id).into_bytes(),
        ));
        zip_files(&files)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HtmlzWriter;

struct Rewriter<'a> {
    book: &'a OpenBook,
    spine: Vec<String>,
    images: Vec<(String, String)>, // archive path -> name in zip
    by_path: HashMap<String, String>,
}

impl Rewriter<'_> {
    fn image_name(&mut self, path: &str) -> Option<String> {
        if let Some(n) = self.by_path.get(path) {
            return Some(n.clone());
        }
        self.book.archive.get(path)?;
        let name = format!("images/img{}.{}", self.images.len() + 1, extension(path));
        self.by_path.insert(path.to_string(), name.clone());
        self.images.push((path.to_string(), name.clone()));
        Some(name)
    }

    fn rewrite_tag(&mut self, tag: &str, doc_path: &str, doc_index: usize) -> String {
        let Some(info) = tag_info(tag) else {
            return tag.to_string();
        };
        if info.closing {
            return tag.to_string();
        }
        let base = parent_dir(doc_path);
        let mut out = tag.to_string();
        for attr in ["src", "href", "xlink:href"] {
            let Some(value) = get_attr(&out, attr) else {
                continue;
            };
            if value.is_empty() || is_external(&value) {
                continue;
            }
            let value = crate::markup::decode_entities(&value);
            let (path, frag) = resolve_href(base, &value);
            let new = if info.name == "a" && attr == "href" {
                let target = if path.is_empty() {
                    doc_path.to_string()
                } else {
                    path
                };
                match self.spine.iter().position(|p| *p == target) {
                    Some(i) => match frag {
                        Some(f) => format!("#{f}"),
                        None => format!("#bookx-file-{}", i + 1),
                    },
                    None => continue,
                }
            } else if info.name == "link" {
                continue;
            } else {
                match self.image_name(&path) {
                    Some(n) => n,
                    None => continue,
                }
            };
            let _ = doc_index;
            out = if attr == "xlink:href" {
                set_attr(&out, "xlink:href", &escape_attr(&new))
            } else {
                set_attr(&out, attr, &escape_attr(&new))
            };
        }
        out
    }
}

impl FormatWriter for HtmlzWriter {
    fn id(&self) -> &'static str {
        "htmlz"
    }
    fn extension(&self) -> &'static str {
        "htmlz"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let md = book.metadata();
        let mut rw = Rewriter {
            book,
            spine: book.spine_paths(),
            images: Vec::new(),
            by_path: HashMap::new(),
        };
        let mut body = String::new();
        for (i, path) in book.spine_paths().iter().enumerate() {
            let Some(text) = book.archive.get_string(path) else {
                continue;
            };
            let inner = match body_range(&text) {
                Some((s, e)) => &text[s..e],
                None => text.as_str(),
            };
            body.push_str(&format!("<div id=\"bookx-file-{}\">\n", i + 1));
            for t in tokenize(inner) {
                match t {
                    Tok::Tag(tag) => body.push_str(&rw.rewrite_tag(tag, path, i)),
                    Tok::Text(s) => body.push_str(s),
                }
            }
            body.push_str("\n</div>\n");
        }
        let css: String = book
            .package
            .manifest
            .iter()
            .filter(|i| i.media_type == "text/css")
            .filter_map(|i| book.archive.get_string(&book.package.item_path(i)))
            .collect::<Vec<_>>()
            .join("\n");
        let html = format!(
            "<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"{}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n{body}</body>\n</html>\n",
            xml_escape(if md.language.is_empty() { "en" } else { &md.language }),
            xml_escape(&md.title)
        );

        let mut files: Vec<(String, Vec<u8>)> = vec![
            ("index.html".into(), html.into_bytes()),
            ("style.css".into(), css.into_bytes()),
        ];
        let mut listing: Vec<(&str, &str, &str)> =
            vec![("html", "index.html", "application/xhtml+xml")];
        let cover_name;
        let mut cover_id = None;
        if let (Some(path), Some(bytes)) = (book.package.cover_path(), book.cover_bytes()) {
            cover_name = format!("cover.{}", extension(&path));
            files.push((cover_name.clone(), bytes.to_vec()));
            listing.push(("cover", &cover_name, media_type_for(&path)));
            cover_id = Some("cover");
        }
        for (path, name) in &rw.images {
            if let Some(data) = book.archive.get(path) {
                files.push((name.clone(), data.to_vec()));
            }
        }
        files.push((
            "metadata.opf".into(),
            flat_opf(book, &listing, cover_id).into_bytes(),
        ));
        zip_files(&files)
    }
}
