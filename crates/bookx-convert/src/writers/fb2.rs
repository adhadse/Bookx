//! FictionBook 2 writer.

use anyhow::Result;
use base64::Engine;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use bookx_core::doc::{Block, BlockKind, Span};
use bookx_core::util::{extension, is_external, media_type_for, xml_escape};

#[derive(Debug, Clone, Copy, Default)]
pub struct Fb2Writer;

struct Binaries {
    list: Vec<(String, String, String)>, // (archive path, id, media type)
}

impl Binaries {
    fn id_for(&mut self, book: &OpenBook, path: &str) -> Option<String> {
        if let Some((_, id, _)) = self.list.iter().find(|(p, _, _)| p == path) {
            return Some(id.clone());
        }
        let media = media_type_for(path);
        if !matches!(media, "image/jpeg" | "image/png" | "image/gif") {
            return None;
        }
        book.archive.get(path)?;
        let ext = extension(path);
        let id = format!(
            "img{}.{}",
            self.list.len() + 1,
            if ext == "jpeg" { "jpg".into() } else { ext }
        );
        self.list
            .push((path.to_string(), id.clone(), media.to_string()));
        Some(id)
    }
}

/// `"First Middle Last"` -> `<first-name>`, `<middle-name>`, `<last-name>`.
pub fn author_xml(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => "<author><nickname>Unknown</nickname></author>".into(),
        [only] => format!(
            "<author><last-name>{}</last-name></author>",
            xml_escape(only)
        ),
        [first, rest @ ..] => {
            let (last, middle) = rest.split_last().expect("non-empty");
            let middle = if middle.is_empty() {
                String::new()
            } else {
                format!(
                    "<middle-name>{}</middle-name>",
                    xml_escape(&middle.join(" "))
                )
            };
            format!(
                "<author><first-name>{}</first-name>{middle}<last-name>{}</last-name></author>",
                xml_escape(first),
                xml_escape(last)
            )
        }
    }
}

fn inline(spans: &[Span]) -> Vec<String> {
    let mut lines = vec![String::new()];
    for sp in spans {
        match sp {
            Span::Text { text, style, link } => {
                if text.is_empty() {
                    continue;
                }
                let mut s = xml_escape(&text.replace('\n', " "));
                if style.code {
                    s = format!("<code>{s}</code>");
                }
                if style.sub {
                    s = format!("<sub>{s}</sub>");
                }
                if style.sup {
                    s = format!("<sup>{s}</sup>");
                }
                if style.strike {
                    s = format!("<strikethrough>{s}</strikethrough>");
                }
                if style.italic {
                    s = format!("<emphasis>{s}</emphasis>");
                }
                if style.bold {
                    s = format!("<strong>{s}</strong>");
                }
                if let Some(l) = link {
                    if is_external(l) {
                        s = format!("<a l:href=\"{}\">{s}</a>", xml_escape(l));
                    }
                }
                lines.last_mut().expect("non-empty").push_str(&s);
            }
            Span::Break => lines.push(String::new()),
            Span::Image { .. } | Span::Anchor(_) => {}
        }
    }
    lines
        .into_iter()
        .filter(|l| !crate::markup::plain_text(l).trim().is_empty())
        .collect()
}

fn paragraphs(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| format!("<p>{}</p>\n", l.trim()))
        .collect()
}

fn images_of(book: &OpenBook, bins: &mut Binaries, b: &Block) -> String {
    b.spans
        .iter()
        .filter_map(|s| match s {
            Span::Image { path, .. } => bins.id_for(book, path),
            _ => None,
        })
        .map(|id| format!("<image l:href=\"#{id}\"/>\n"))
        .collect()
}

fn section(book: &OpenBook, bins: &mut Binaries, path: &str) -> Option<String> {
    let doc = book.chapter_doc(path)?;
    let mut title: Option<String> = None;
    let mut body = String::new();
    for (i, b) in doc.blocks.iter().enumerate() {
        let lines = inline(&b.spans);
        let imgs = images_of(book, bins, b);
        match b.kind {
            BlockKind::Heading(_) => {
                let text = lines.join(" ");
                if text.is_empty() {
                    body.push_str(&imgs);
                } else if i == 0 && title.is_none() {
                    title = Some(format!("<title><p>{}</p></title>\n", text.trim()));
                } else {
                    body.push_str(&format!("<subtitle>{}</subtitle>\n", text.trim()));
                    body.push_str(&imgs);
                }
            }
            BlockKind::Rule => body.push_str("<empty-line/>\n"),
            BlockKind::Quote => {
                if !lines.is_empty() {
                    body.push_str(&format!("<cite>\n{}</cite>\n", paragraphs(&lines)));
                }
                body.push_str(&imgs);
            }
            BlockKind::Code | BlockKind::Paragraph => {
                let mut lines = lines;
                if let (Some(m), Some(first)) = (&b.marker, lines.first_mut()) {
                    *first = format!("{} {}", xml_escape(m), first);
                }
                body.push_str(&imgs);
                body.push_str(&paragraphs(&lines));
            }
        }
    }
    if title.is_none() && body.trim().is_empty() {
        return None;
    }
    if body.trim().is_empty() {
        body.push_str("<empty-line/>\n");
    }
    Some(format!(
        "<section>\n{}{}</section>\n",
        title.unwrap_or_default(),
        body
    ))
}

fn wrap_base64(data: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
    b64.as_bytes()
        .chunks(76)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

impl FormatWriter for Fb2Writer {
    fn id(&self) -> &'static str {
        "fb2"
    }
    fn extension(&self) -> &'static str {
        "fb2"
    }

    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let md = book.metadata();
        let mut bins = Binaries { list: Vec::new() };
        let cover_id = book
            .package
            .cover_path()
            .filter(|_| book.cover_bytes().is_some())
            .and_then(|p| bins.id_for(book, &p));

        let mut sections = String::new();
        for path in book.spine_paths() {
            if let Some(s) = section(book, &mut bins, &path) {
                sections.push_str(&s);
            }
        }
        if sections.is_empty() {
            sections.push_str("<section>\n<p></p>\n</section>\n");
        }

        let mut ti = String::from("<genre>unrecognised</genre>\n");
        if md.authors.is_empty() {
            ti.push_str(&author_xml(""));
        }
        for a in &md.authors {
            ti.push_str(&author_xml(a));
        }
        ti.push('\n');
        ti.push_str(&format!(
            "<book-title>{}</book-title>\n",
            xml_escape(if md.title.is_empty() {
                "Untitled"
            } else {
                &md.title
            })
        ));
        let descr = crate::markup::plain_text(&md.description);
        if !descr.trim().is_empty() {
            ti.push_str("<annotation>");
            for l in descr.lines().filter(|l| !l.trim().is_empty()) {
                ti.push_str(&format!("<p>{}</p>", xml_escape(l.trim())));
            }
            ti.push_str("</annotation>\n");
        }
        if !md.tags.is_empty() {
            ti.push_str(&format!(
                "<keywords>{}</keywords>\n",
                xml_escape(&md.tags.join(", "))
            ));
        }
        if !md.published.is_empty() {
            ti.push_str(&format!("<date>{}</date>\n", xml_escape(&md.published)));
        }
        if let Some(id) = &cover_id {
            ti.push_str(&format!(
                "<coverpage><image l:href=\"#{id}\"/></coverpage>\n"
            ));
        }
        ti.push_str(&format!(
            "<lang>{}</lang>\n",
            xml_escape(if md.language.is_empty() {
                "en"
            } else {
                &md.language
            })
        ));
        if !md.series.is_empty() {
            ti.push_str(&format!(
                "<sequence name=\"{}\" number=\"{}\"/>\n",
                xml_escape(&md.series),
                bookx_core::opf::format_index(md.series_index)
            ));
        }

        let mut pi = String::new();
        if !md.publisher.is_empty() {
            pi.push_str(&format!(
                "<publisher>{}</publisher>\n",
                xml_escape(&md.publisher)
            ));
        }
        if let Some(isbn) = md.identifier("isbn") {
            pi.push_str(&format!("<isbn>{}</isbn>\n", xml_escape(isbn)));
        }
        let publish = if pi.is_empty() {
            String::new()
        } else {
            format!("<publish-info>\n{pi}</publish-info>\n")
        };

        let today = chrono::Utc::now().format("%Y-%m-%d");
        let mut out = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\" xmlns:l=\"http://www.w3.org/1999/xlink\">\n<description>\n<title-info>\n{ti}</title-info>\n<document-info>\n<author><nickname>Bookx</nickname></author>\n<program-used>Bookx</program-used>\n<date value=\"{today}\">{today}</date>\n<id>{}</id>\n<version>1.0</version>\n</document-info>\n{publish}</description>\n<body>\n{sections}</body>\n",
            uuid::Uuid::new_v4()
        );
        for (path, id, media) in &bins.list {
            if let Some(data) = book.archive.get(path) {
                out.push_str(&format!(
                    "<binary id=\"{id}\" content-type=\"{media}\">\n{}\n</binary>\n",
                    wrap_base64(data)
                ));
            }
        }
        out.push_str("</FictionBook>\n");
        Ok(out.into_bytes())
    }
}
