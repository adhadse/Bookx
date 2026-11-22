//! Sending a book together with the reader's own highlights, notes and bookmarks (bug 2).
//!
//! Two things travel with the book when the option is on:
//! * an *Annotations* chapter appended to the EPUB before it is converted, so every device and every
//!   format (KEPUB, AZW3, MOBI, plain EPUB) shows them at the end of the book and in its contents;
//! * for readers that open EPUB/KEPUB (Kobo, PocketBook, Boox, ...): a KOReader sidecar
//!   (`<book>.sdr/metadata.epub.lua`) whose bookmark list holds the same notes. KOReader positions are
//!   crengine xpointers that cannot be derived from a character offset without its DOM, so the
//!   entries point at the start of the chapter (`/body/DocFragment[N]`) and carry the text and note.
//!
//! No GTK in here, so it is unit tested.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::library::{Bookmark, Highlight};
use bookx_core::opf::SpineItem;
use bookx_core::toc::{self, TocEntry};
use bookx_core::util::{file_name, join_path, xml_escape};

/// Manifest id and file name of the appended chapter.
pub const APPENDIX_ID: &str = "bookx-annotations";
const APPENDIX_FILE: &str = "bookx-annotations.xhtml";
/// Title of the appended chapter in the contents.
pub const APPENDIX_TITLE: &str = "My notes";

/// The annotations of one book, as stored in the library.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notes {
    pub highlights: Vec<Highlight>,
    pub bookmarks: Vec<Bookmark>,
}

impl Notes {
    pub fn new(highlights: &[Highlight], bookmarks: &[Bookmark]) -> Self {
        Notes {
            highlights: highlights
                .iter()
                .filter(|h| !crate::pdf::is_pdf(h))
                .cloned()
                .collect(),
            bookmarks: bookmarks.to_vec(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.highlights.is_empty() && self.bookmarks.is_empty()
    }
}

/// Title of every spine item (from the contents, else the file name).
pub fn chapter_titles(book: &OpenBook) -> Vec<String> {
    let flat = toc::flatten(&book.toc());
    book.spine_paths()
        .iter()
        .map(|p| {
            flat.iter()
                .find(|t| t.href.split('#').next() == Some(p.as_str()))
                .map(|t| t.title.clone())
                .unwrap_or_else(|| file_name(p).to_string())
        })
        .collect()
}

fn chapter_name(chapters: &[String], spine: usize) -> String {
    chapters
        .get(spine)
        .cloned()
        .unwrap_or_else(|| format!("Section {}", spine + 1))
}

/// The XHTML of the appendix page.
pub fn appendix_xhtml(title: &str, lang: &str, chapters: &[String], notes: &Notes) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<h1>{}</h1>\n<p class=\"bookx-annotations-for\">{}</p>\n",
        xml_escape(APPENDIX_TITLE),
        xml_escape(title)
    ));
    let mut by_chapter: BTreeMap<usize, (Vec<&Highlight>, Vec<&Bookmark>)> = BTreeMap::new();
    for h in &notes.highlights {
        by_chapter.entry(h.spine).or_default().0.push(h);
    }
    for b in &notes.bookmarks {
        by_chapter.entry(b.spine).or_default().1.push(b);
    }
    for (spine, (mut hl, mut bm)) in by_chapter {
        hl.sort_by_key(|h| h.start);
        bm.sort_by(|a, b| a.frac.total_cmp(&b.frac));
        body.push_str(&format!(
            "<h2>{}</h2>\n",
            xml_escape(&chapter_name(chapters, spine))
        ));
        for h in hl {
            body.push_str(&format!(
                "<blockquote><p>{}</p></blockquote>\n",
                xml_escape(&h.text).replace('\n', "<br/>")
            ));
            if !h.note.trim().is_empty() {
                body.push_str(&format!(
                    "<p><em>Note:</em> {}</p>\n",
                    xml_escape(&h.note).replace('\n', "<br/>")
                ));
            }
        }
        if !bm.is_empty() {
            body.push_str("<ul>\n");
            for b in bm {
                let label = if b.label.trim().is_empty() {
                    "Bookmark"
                } else {
                    b.label.trim()
                };
                body.push_str(&format!(
                    "<li>Bookmark: {} ({}%)</li>\n",
                    xml_escape(label),
                    (b.frac.clamp(0.0, 1.0) * 100.0).round() as u32
                ));
            }
            body.push_str("</ul>\n");
        }
    }
    let lang = if lang.is_empty() { "en" } else { lang };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{l}\" lang=\"{l}\">\n<head><meta charset=\"utf-8\"/><title>{t}</title></head>\n<body>\n{body}</body>\n</html>\n",
        l = xml_escape(lang),
        t = xml_escape(APPENDIX_TITLE),
    )
}

/// Returns the EPUB with an *Annotations* chapter appended to the spine and the contents. A book without
/// annotations comes back unchanged; running it twice refreshes the chapter instead of adding another.
pub fn embed_appendix(epub: &[u8], notes: &Notes) -> Result<Vec<u8>> {
    if notes.is_empty() {
        return Ok(epub.to_vec());
    }
    let mut book = OpenBook::from_archive(Archive::from_bytes(epub.to_vec())?)
        .context("reading the book to add its annotations")?;
    let chapters = chapter_titles(&book);
    let existing = book.package.item_by_id(APPENDIX_ID).cloned();
    let path = match &existing {
        Some(i) => book.package.item_path(i),
        None => {
            let mut p = join_path(book.package.base_dir(), APPENDIX_FILE);
            let mut n = 1;
            while book.archive.contains(&p) {
                n += 1;
                p = join_path(
                    book.package.base_dir(),
                    &format!("bookx-annotations-{n}.xhtml"),
                );
            }
            p
        }
    };
    let xhtml = appendix_xhtml(
        &book.metadata().title,
        &book.metadata().language,
        &chapters,
        notes,
    );
    book.archive.set_string(&path, &xhtml);
    if existing.is_none() {
        let pkg = book.package.clone();
        pkg.add_manifest_item(
            &mut book.archive,
            APPENDIX_ID,
            &path,
            "application/xhtml+xml",
            "",
        )?;
        book.reload_package()?;
        let mut spine = book.package.spine.clone();
        spine.push(SpineItem {
            idref: APPENDIX_ID.into(),
            linear: true,
        });
        let pkg = book.package.clone();
        pkg.set_spine(&mut book.archive, &spine)?;
        book.reload_package()?;
        let mut entries: Vec<TocEntry> = book.toc();
        if !entries.is_empty() {
            entries.push(TocEntry {
                title: APPENDIX_TITLE.into(),
                href: path.clone(),
                children: vec![],
            });
            let pkg = book.package.clone();
            toc::write_toc(&mut book.archive, &pkg, &entries)?;
            book.reload_package()?;
        }
    }
    book.archive.to_bytes()
}

fn lua_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\{:03}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The content of KOReader's `metadata.<ext>.lua` with the notes as bookmarks (see the module docs).
pub fn koreader_sidecar(title: &str, chapters: &[String], notes: &Notes) -> String {
    struct Entry {
        spine: usize,
        order: f64,
        text: String,
        note: String,
        when: String,
        /// KOReader's `color` and `drawer` words; none for a bookmark.
        look: Option<(String, String)>,
    }
    let fmt = |d: &chrono::DateTime<chrono::Utc>| d.format("%Y-%m-%d %H:%M:%S").to_string();
    let mut entries: Vec<Entry> = Vec::new();
    for h in &notes.highlights {
        entries.push(Entry {
            spine: h.spine,
            order: h.start as f64,
            text: h.text.clone(),
            note: h.note.clone(),
            when: fmt(&h.created),
            look: Some((
                bookx_core::devstyle::to_koreader_color(&h.color),
                bookx_core::devstyle::to_koreader_drawer(&h.style),
            )),
        });
    }
    for b in &notes.bookmarks {
        entries.push(Entry {
            spine: b.spine,
            order: b.frac * 1.0e9,
            text: if b.label.trim().is_empty() {
                "Bookmark".into()
            } else {
                b.label.clone()
            },
            note: String::new(),
            when: fmt(&b.created),
            look: None,
        });
    }
    entries.sort_by_key(|a| (a.spine, a.order.to_bits()));
    let mut s = String::from("-- bookx annotations for KOReader\nreturn {\n");
    s.push_str(&format!(
        "    [\"doc_props\"] = {{\n        [\"title\"] = {},\n    }},\n",
        lua_str(title)
    ));
    s.push_str("    [\"bookmarks\"] = {\n");
    for (i, e) in entries.iter().enumerate() {
        let own_note = if e.note.is_empty() {
            String::new()
        } else {
            format!("            [\"note\"] = {},\n", lua_str(&e.note))
        };
        let look = match &e.look {
            Some((c, d)) => format!(
                "            [\"color\"] = {},\n            [\"drawer\"] = {},\n",
                lua_str(c),
                lua_str(d)
            ),
            None => String::new(),
        };
        s.push_str(&format!(
            "        [{}] = {{\n            [\"chapter\"] = {},\n{look}            [\"datetime\"] = {},\n{own_note}            [\"notes\"] = {},\n            [\"page\"] = {},\n            [\"text\"] = {},\n        }},\n",
            i + 1,
            lua_str(&chapter_name(chapters, e.spine)),
            lua_str(&e.when),
            lua_str(if e.note.is_empty() { &e.text } else { &e.note }),
            lua_str(&format!("/body/DocFragment[{}]", e.spine + 1)),
            lua_str(&e.text),
        ));
    }
    s.push_str("    },\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::convert::{create_epub, Chapter};
    use bookx_core::opf::Metadata;
    use chrono::TimeZone;

    fn epub() -> Vec<u8> {
        let md = Metadata {
            title: "Noted".into(),
            authors: vec!["Ann".into()],
            language: "en".into(),
            ..Default::default()
        };
        let ch = vec![
            Chapter {
                title: "One".into(),
                body: "<h1>One</h1><p>First.</p>".into(),
            },
            Chapter {
                title: "Two".into(),
                body: "<h1>Two</h1><p>Second.</p>".into(),
            },
        ];
        create_epub(&md, &ch, None).to_bytes().unwrap()
    }

    fn notes() -> Notes {
        let created = chrono::Utc.with_ymd_and_hms(2026, 5, 6, 7, 8, 9).unwrap();
        Notes {
            highlights: vec![
                Highlight {
                    id: "h2".into(),
                    spine: 1,
                    start: 3,
                    end: 9,
                    color: "yellow".into(),
                    style: String::new(),
                    note: "my \"note\"\nline two".into(),
                    text: "Second <b> & more".into(),
                    created,
                },
                Highlight {
                    id: "h1".into(),
                    spine: 0,
                    start: 0,
                    end: 4,
                    color: "yellow".into(),
                    style: String::new(),
                    note: String::new(),
                    text: "First".into(),
                    created,
                },
            ],
            bookmarks: vec![Bookmark {
                id: "b".into(),
                spine: 1,
                frac: 0.5,
                label: "Halfway".into(),
                created,
            }],
        }
    }

    #[test]
    fn no_annotations_leave_the_book_untouched() {
        let e = epub();
        assert_eq!(embed_appendix(&e, &Notes::default()).unwrap(), e);
    }

    #[test]
    fn the_appendix_is_in_the_spine_and_the_contents() {
        let out = embed_appendix(&epub(), &notes()).unwrap();
        let book = OpenBook::from_archive(Archive::from_bytes(out).unwrap()).unwrap();
        let spine = book.spine_paths();
        assert!(spine.last().unwrap().ends_with("bookx-annotations.xhtml"));
        assert_eq!(spine.len(), 3);
        let toc = toc::flatten(&book.toc());
        assert_eq!(toc.last().unwrap().title, APPENDIX_TITLE);
        assert_eq!(toc.last().unwrap().href, *spine.last().unwrap());
        let page = book.chapter_source(spine.last().unwrap()).unwrap();
        for want in [
            "First",
            "Second &lt;b&gt; &amp; more",
            "my &quot;note&quot;<br/>line two",
            "Halfway (50%)",
            "<h2>One</h2>",
            "<h2>Two</h2>",
        ] {
            assert!(page.contains(want), "{want} missing in {page}");
        }
        assert!(page.find("<h2>One</h2>").unwrap() < page.find("<h2>Two</h2>").unwrap());
        let opts = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        roxmltree::Document::parse_with_options(&page, opts)
            .unwrap_or_else(|e| panic!("{e}: {page}"));
    }

    #[test]
    fn embedding_twice_refreshes_instead_of_duplicating() {
        let once = embed_appendix(&epub(), &notes()).unwrap();
        let mut more = notes();
        more.bookmarks.clear();
        let twice = embed_appendix(&once, &more).unwrap();
        let book = OpenBook::from_archive(Archive::from_bytes(twice).unwrap()).unwrap();
        assert_eq!(book.spine_paths().len(), 3);
        let page = book
            .chapter_source(book.spine_paths().last().unwrap())
            .unwrap();
        assert!(!page.contains("Halfway"));
        let n = toc::flatten(&book.toc())
            .iter()
            .filter(|t| t.title == APPENDIX_TITLE)
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn the_appendix_survives_conversion_to_the_device_formats() {
        let out = embed_appendix(&epub(), &notes()).unwrap();
        for id in ["kepub", "epub"] {
            let t = crate::export::target(id).unwrap();
            let bytes = crate::export::convert_epub_bytes(&out, "noted.epub", t).unwrap();
            let z = Archive::from_bytes(bytes).unwrap();
            let all: String = z
                .names()
                .iter()
                .filter(|n| n.ends_with("html"))
                .filter_map(|n| z.get_string(n))
                .collect();
            assert!(all.contains("Halfway"), "{id}");
        }
        let t = crate::export::target("azw3").unwrap();
        let bytes = crate::export::convert_epub_bytes(&out, "noted.epub", t).unwrap();
        let back = bookx_formats::read_any("noted.azw3", &bytes).unwrap();
        let back = OpenBook::from_archive(back).unwrap();
        let text: String = back
            .spine_paths()
            .iter()
            .filter_map(|p| back.chapter_source(p))
            .collect();
        assert!(text.contains("Halfway"), "{text}");
    }

    #[test]
    fn sidecar_is_lua_with_escaped_strings_in_reading_order() {
        let chapters = vec!["One".to_string(), "Two".to_string()];
        let lua = koreader_sidecar("Noted", &chapters, &notes());
        assert!(lua.starts_with("-- bookx annotations for KOReader\nreturn {"));
        assert!(lua.contains("[\"notes\"] = \"my \\\"note\\\"\\nline two\""));
        assert!(lua.contains("[\"datetime\"] = \"2026-05-06 07:08:09\""));
        assert!(lua.contains("[\"page\"] = \"/body/DocFragment[2]\""));
        assert!(lua.contains("[\"chapter\"] = \"Two\""));
        let first = lua.find("\"First\"").unwrap();
        let second = lua.find("Second <b>").unwrap();
        let half = lua.find("Halfway").unwrap();
        assert!(first < second && second < half);
        assert_eq!(lua.matches('{').count(), lua.matches('}').count());
    }

    #[test]
    fn lua_strings_escape_control_characters() {
        assert_eq!(lua_str("a\\b\"c\u{1}"), "\"a\\\\b\\\"c\\001\"");
    }

    #[test]
    fn pdf_page_highlights_are_not_sent_as_epub_notes() {
        let created = chrono::Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let hl = |id: &str, style: &str| Highlight {
            id: id.into(),
            spine: 3,
            start: 0,
            end: 4,
            color: "yellow".into(),
            style: style.into(),
            note: String::new(),
            text: "text".into(),
            created,
        };
        let notes = Notes::new(
            &[hl("a", ""), hl("b", "pdf|background|1.0,2.0,3.0,4.0")],
            &[],
        );
        assert_eq!(notes.highlights.len(), 1);
        assert_eq!(notes.highlights[0].id, "a");
    }
}
