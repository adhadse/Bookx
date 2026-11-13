//! Plain text of books, chapter by chapter, with a small cache.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use bookx_core::book::OpenBook;
use bookx_core::formats::Target;
use bookx_core::library::Book;
use bookx_core::toc;

use crate::app::{fail, lock, ToolError};

#[derive(Debug, Clone)]
pub struct Chapter {
    pub index: usize,
    pub path: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct BookText {
    pub chapters: Vec<Chapter>,
    /// Table of contents entries: (depth, title, chapter index when it points into the spine).
    pub toc: Vec<(usize, String, Option<usize>)>,
}

impl BookText {
    pub fn chars(&self) -> usize {
        self.chapters.iter().map(|c| c.text.chars().count()).sum()
    }
}

/// The text of a block list the way the reader numbers characters: paragraphs joined by a line
/// break, whitespace runs collapsed.
fn canonical(blocks: impl Iterator<Item = String>) -> String {
    let mut out = String::new();
    for b in blocks {
        let line = b.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
    }
    out
}

pub fn extract(path: &Path) -> Result<BookText, ToolError> {
    let book = OpenBook::open(path)
        .map_err(|_| ToolError("the book has no EPUB version whose text can be read".into()))?;
    let spine = book.spine_paths();
    let flat = toc::flatten(&book.toc());
    let index_of = |href: &str| {
        let p = href.split('#').next().unwrap_or(href);
        spine.iter().position(|s| s == p)
    };
    let toc_entries: Vec<(usize, String, Option<usize>)> = flat
        .iter()
        .map(|t| (t.depth, t.title.trim().to_string(), index_of(&t.href)))
        .collect();
    let mut chapters = Vec::with_capacity(spine.len());
    for (i, p) in spine.iter().enumerate() {
        let text = book
            .chapter_doc(p)
            .map(|d| canonical(d.blocks.iter().map(|b| b.plain_text())))
            .unwrap_or_default();
        let title = toc_entries
            .iter()
            .find(|(_, t, idx)| *idx == Some(i) && !t.is_empty())
            .map(|(_, t, _)| t.clone())
            .unwrap_or_else(|| format!("Section {}", i + 1));
        chapters.push(Chapter {
            index: i,
            path: p.clone(),
            title,
            text,
        });
    }
    Ok(BookText {
        chapters,
        toc: toc_entries,
    })
}

pub struct TextCache {
    entries: Mutex<VecDeque<(u64, u64, Arc<BookText>)>>,
}

impl Default for TextCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextCache {
    pub fn new() -> TextCache {
        TextCache {
            entries: Mutex::new(VecDeque::new()),
        }
    }

    /// The text of `book`, refused for protected books. Cached by id and modification time.
    pub fn get(&self, root: &Path, book: &Book) -> Result<Arc<BookText>, ToolError> {
        if !book.drm.is_empty() {
            return fail(format!(
                "this book is copy protected ({}); its text is not available to assistants",
                book.drm
            ));
        }
        let stamp = book.modified.timestamp_millis() as u64 ^ book.size.rotate_left(32);
        {
            let c = lock(&self.entries);
            if let Some((_, _, t)) = c.iter().find(|(id, s, _)| *id == book.id && *s == stamp) {
                return Ok(t.clone());
            }
        }
        let path = root.join(book.file_for(Target::Reader));
        let t = Arc::new(extract(&path)?);
        let mut c = lock(&self.entries);
        c.retain(|(id, _, _)| *id != book.id);
        c.push_back((book.id, stamp, t.clone()));
        while c.len() > 4 {
            c.pop_front();
        }
        Ok(t)
    }
}

/// `max` characters of `text` from character `from`, cut at a word boundary when one is close.
/// Returns the piece and the offset where the next piece starts (`None` at the end).
pub fn slice_chars(text: &str, from: usize, max: usize) -> (String, Option<usize>) {
    let total = text.chars().count();
    if from >= total {
        return (String::new(), None);
    }
    let mut end = (from + max).min(total);
    let chars: Vec<char> = text.chars().skip(from).take(end - from + 1).collect();
    if end < total {
        let window = chars.len().saturating_sub(1);
        if let Some(back) = chars[..window]
            .iter()
            .rev()
            .take(120)
            .position(|c| c.is_whitespace())
        {
            end -= back;
        }
    }
    let piece: String = text.chars().skip(from).take(end - from).collect();
    (piece, (end < total).then_some(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_cut_at_words_and_report_the_rest() {
        let t = "alpha beta gamma delta";
        let (a, next) = slice_chars(t, 0, 8);
        assert_eq!(a, "alpha ");
        assert_eq!(next, Some(6));
        let (b, next) = slice_chars(t, 6, 100);
        assert_eq!(b, "beta gamma delta");
        assert_eq!(next, None);
        assert_eq!(slice_chars(t, 99, 5), (String::new(), None));
        // no whitespace nearby: a hard cut
        let (c, n) = slice_chars("abcdefghij", 0, 4);
        assert_eq!((c.as_str(), n), ("abcd", Some(4)));
    }

    #[test]
    fn canonical_joins_paragraphs_and_collapses_space() {
        let s = canonical(["  a   b ", "", "c\nd"].into_iter().map(String::from));
        assert_eq!(s, "a b\nc d");
    }
}
