//! Exporting highlights, notes and bookmarks.

use serde_json::json;

use super::library::Book;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationFormat {
    Json,
    Markdown,
    Text,
}

impl AnnotationFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "md",
            Self::Text => "txt",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Markdown => "Markdown",
            Self::Text => "Plain text",
        }
    }
}

pub fn export_annotations(book: &Book, chapters: &[String], fmt: AnnotationFormat) -> String {
    let chapter = |i: usize| {
        chapters
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("Section {}", i + 1))
    };
    match fmt {
        AnnotationFormat::Json => {
            let v = json!({
                "title": book.meta.title,
                "authors": book.meta.authors,
                "highlights": book.highlights.iter().map(|h| json!({
                    "chapter": chapter(h.spine),
                    "text": h.text,
                    "note": h.note,
                    "color": h.color,
                    "style": h.style,
                    "created": h.created,
                })).collect::<Vec<_>>(),
                "bookmarks": book.bookmarks.iter().map(|b| json!({
                    "chapter": chapter(b.spine),
                    "label": b.label,
                    "position": b.frac,
                    "created": b.created,
                })).collect::<Vec<_>>(),
            });
            serde_json::to_string_pretty(&v).unwrap_or_default()
        }
        AnnotationFormat::Markdown => {
            let mut s = format!(
                "# {}\n\n*{}*\n\n",
                book.meta.title,
                book.meta.authors_display()
            );
            if !book.highlights.is_empty() {
                s.push_str("## Highlights\n\n");
            }
            let mut last = usize::MAX;
            let mut hl: Vec<_> = book.highlights.iter().collect();
            hl.sort_by_key(|h| (h.spine, h.start));
            for h in hl {
                if h.spine != last {
                    s.push_str(&format!("### {}\n\n", chapter(h.spine)));
                    last = h.spine;
                }
                s.push_str(&format!("> {}\n\n", h.text.replace('\n', "\n> ")));
                if !h.note.is_empty() {
                    s.push_str(&format!("{}\n\n", h.note));
                }
            }
            if !book.bookmarks.is_empty() {
                s.push_str("## Bookmarks\n\n");
                for b in &book.bookmarks {
                    s.push_str(&format!(
                        "- {} — {} ({:.0}%)\n",
                        chapter(b.spine),
                        b.label,
                        b.frac * 100.0
                    ));
                }
            }
            s
        }
        AnnotationFormat::Text => {
            let mut s = format!("{}\n{}\n\n", book.meta.title, book.meta.authors_display());
            let mut hl: Vec<_> = book.highlights.iter().collect();
            hl.sort_by_key(|h| (h.spine, h.start));
            for h in hl {
                s.push_str(&format!("[{}] {}\n", chapter(h.spine), h.text));
                if !h.note.is_empty() {
                    s.push_str(&format!("  Note: {}\n", h.note));
                }
                s.push('\n');
            }
            for b in &book.bookmarks {
                s.push_str(&format!("Bookmark: {} — {}\n", chapter(b.spine), b.label));
            }
            s
        }
    }
}
