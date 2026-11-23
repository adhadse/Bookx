//! KEPUB (Kobo) writer: EPUB whose text is wrapped in `koboSpan` elements.

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

use super::epub::{EpubVersion, repack};
use crate::markup::{Tok, body_range, tag_info, tokenize};

#[derive(Debug, Clone, Copy)]
pub struct KepubWriter {
    pub version: EpubVersion,
}

impl Default for KepubWriter {
    /// EPUB 2 packaging: Kobo devices read the NCX and it is the most widely tested path.
    fn default() -> Self {
        KepubWriter {
            version: EpubVersion::V2,
        }
    }
}

impl FormatWriter for KepubWriter {
    fn id(&self) -> &'static str {
        "kepub"
    }
    fn extension(&self) -> &'static str {
        "kepub.epub"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let mut arc = repack(book, self.version)?;
        for path in book.spine_paths() {
            if let Some(text) = arc.get_string(&path) {
                arc.set_string(&path, &koboify_html(&text));
            }
        }
        arc.to_bytes()
    }
}

const TEXT_BLOCKS: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "td",
    "th",
    "dd",
    "dt",
    "blockquote",
    "div",
    "figcaption",
    "caption",
    "section",
    "article",
];

const NO_SPANS: &[&str] = &["script", "style", "svg", "math", "pre", "title", "head"];

/// Splits raw (entity-escaped) text after sentence terminators followed by white space.
pub fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        if matches!(c, '.' | '!' | '?' | '\u{2026}') {
            let mut j = i + 1;
            while j < chars.len()
                && matches!(
                    chars[j].1,
                    '"' | '\'' | '\u{201d}' | '\u{2019}' | ')' | ']' | '\u{bb}'
                )
            {
                j += 1;
            }
            if j < chars.len() && chars[j].1.is_whitespace() {
                while j < chars.len() && chars[j].1.is_whitespace() {
                    j += 1;
                }
                let end = chars.get(j).map(|c| c.0).unwrap_or(text.len());
                if end > start {
                    out.push(&text[start..end]);
                    start = end;
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Wraps the sentences of an XHTML document in `<span class="koboSpan" id="kobo.P.S">` and adds
/// the `book-columns`/`book-inner` wrapper divs. Documents that already have spans are kept.
pub fn koboify_html(html: &str) -> String {
    if html.contains("koboSpan") {
        return html.to_string();
    }
    let Some((bs, be)) = body_range(html) else {
        return html.to_string();
    };
    let body = &html[bs..be];
    let mut out = String::with_capacity(html.len() * 2);
    out.push_str(&html[..bs]);
    out.push_str("<div id=\"book-columns\"><div id=\"book-inner\">");
    let mut paragraph = 1usize;
    let mut sentence = 0usize;
    let mut skip = 0usize;
    for t in tokenize(body) {
        match t {
            Tok::Tag(tag) => {
                out.push_str(tag);
                if let Some(info) = tag_info(tag) {
                    if NO_SPANS.contains(&info.name.as_str()) && !info.self_closing {
                        if info.closing {
                            skip = skip.saturating_sub(1);
                        } else {
                            skip += 1;
                        }
                    }
                    if !info.closing
                        && !info.self_closing
                        && TEXT_BLOCKS.contains(&info.name.as_str())
                        && sentence > 0
                    {
                        paragraph += 1;
                        sentence = 0;
                    }
                }
            }
            Tok::Text(text) => {
                if skip > 0 || text.trim().is_empty() {
                    out.push_str(text);
                    continue;
                }
                for piece in split_sentences(text) {
                    if piece.trim().is_empty() {
                        out.push_str(piece);
                    } else {
                        sentence += 1;
                        out.push_str(&format!(
                            "<span class=\"koboSpan\" id=\"kobo.{paragraph}.{sentence}\">{piece}</span>"
                        ));
                    }
                }
            }
        }
    }
    out.push_str("</div></div>");
    out.push_str(&html[be..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_split_after_terminators() {
        assert_eq!(
            split_sentences("One. Two! Three?  Four"),
            vec!["One. ", "Two! ", "Three?  ", "Four"]
        );
        assert_eq!(
            split_sentences("He said \"stop.\" Then left."),
            vec!["He said \"stop.\" ", "Then left."]
        );
        assert_eq!(split_sentences("3.14 is pi"), vec!["3.14 is pi"]);
        assert_eq!(split_sentences("Wait… what"), vec!["Wait… ", "what"]);
        assert_eq!(split_sentences(""), Vec::<&str>::new());
        assert_eq!(split_sentences("tail. "), vec!["tail. "]);
    }

    #[test]
    fn spans_and_wrappers_are_added() {
        let html = "<html><body><h1>Title</h1><p>One. Two <i>it</i>.</p></body></html>";
        let out = koboify_html(html);
        assert!(out.contains("<body><div id=\"book-columns\"><div id=\"book-inner\">"));
        assert!(out.contains("</div></div></body>"));
        assert!(out.contains("<span class=\"koboSpan\" id=\"kobo.1.1\">Title</span>"));
        assert!(out.contains("id=\"kobo.2.1\">One. </span>"));
        assert!(out.contains("id=\"kobo.2.2\">Two </span>"));
        assert!(out.contains("<i><span class=\"koboSpan\" id=\"kobo.2.3\">it</span></i>"));
    }

    #[test]
    fn already_kepub_and_bodyless_documents_are_kept() {
        let k =
            "<html><body><p><span class=\"koboSpan\" id=\"kobo.1.1\">x</span></p></body></html>";
        assert_eq!(koboify_html(k), k);
        assert_eq!(koboify_html("<p>no body</p>"), "<p>no body</p>");
    }

    #[test]
    fn pre_script_style_and_whitespace_are_not_wrapped() {
        let out = koboify_html(
            "<html><head><title>T</title></head><body><pre>code. here</pre><p>a</p> <p> </p><script>x()</script></body></html>",
        );
        assert!(!out.contains(">code. here</span>"));
        assert!(!out.contains("T</span>"));
        assert!(!out.contains(">x()</span>"));
        assert_eq!(out.matches("koboSpan\"").count(), 1);
    }
}
