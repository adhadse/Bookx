//! Footnote popups (R5, R55, F3), without GTK: given the source of the chapter a link points into
//! and the fragment, find the note text to show in a popover instead of jumping there.
//!
//! A target counts as a note when it is marked as one (`epub:type` or `role` footnote, endnote,
//! rearnote, note), or is a list item, or its id/class says so (`fn1`, `footnote-3`, `note`), and
//! its text is short. Anything else (a heading, a chapter, a long section) is followed normally.

use scraper::{ElementRef, Html};

/// Longest note shown in a popup; longer targets are navigated to.
pub const MAX_CHARS: usize = 1200;

/// Containers a note text lives in, when the link points at a small anchor inside it.
const BLOCKS: [&str; 9] = [
    "p",
    "li",
    "aside",
    "div",
    "section",
    "dd",
    "blockquote",
    "td",
    "article",
];

fn is_marked_note(el: ElementRef) -> bool {
    let v = el.value();
    let marked = |s: &str| {
        let s = s.to_ascii_lowercase();
        s.split_whitespace().any(|t| {
            matches!(
                t.trim_start_matches("doc-"),
                "footnote" | "endnote" | "rearnote" | "note" | "footnotes" | "endnotes"
            )
        })
    };
    v.attr("epub:type").is_some_and(marked)
        || v.attr("role").is_some_and(marked)
        || v.attr("type").is_some_and(marked)
}

fn named_like_note(el: ElementRef) -> bool {
    let v = el.value();
    let id = v.attr("id").unwrap_or("").to_ascii_lowercase();
    let class = v.attr("class").unwrap_or("").to_ascii_lowercase();
    let idish = id.starts_with("fn")
        || id.starts_with("note")
        || id.starts_with("endnote")
        || id.contains("footnote");
    idish || class.contains("footnote") || class.split_whitespace().any(|c| c == "note")
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text of the note without its number in front and the "back to the text" mark behind.
fn clean(text: &str) -> String {
    let mut t = collapse(text);
    // a trailing back link: ↩ ↑ ⏎ ↵ or [back]
    loop {
        let trimmed = t
            .trim_end_matches(|c: char| {
                matches!(
                    c,
                    '↩' | '↑' | '⏎' | '↵' | '⤴' | '^' | '\u{fe0e}' | '\u{fe0f}'
                ) || c.is_whitespace()
            })
            .to_string();
        if trimmed == t {
            break;
        }
        t = trimmed;
    }
    // a leading marker: "1", "1.", "[1]", "(1)", "*", "†"
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    if i < chars.len() && matches!(chars[i], '[' | '(') {
        i += 1;
    }
    let digits_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i > digits_start {
        let opened = digits_start > 0;
        if i < chars.len() && matches!(chars[i], ']' | ')' | '.' | ':') {
            i += 1;
        } else if !opened {
            // "1984 was a year": a number without a marker is part of the text
            return t;
        }
        let rest: String = chars[i..].iter().collect();
        let rest = rest.trim_start().to_string();
        if !rest.is_empty() {
            return rest;
        }
    } else if !chars.is_empty() && matches!(chars[0], '*' | '†' | '‡') {
        let rest: String = chars[1..].iter().collect();
        let rest = rest.trim_start().to_string();
        if !rest.is_empty() {
            return rest;
        }
    }
    t
}

/// The note text for the element with id `frag` in `source`, or `None` when the target is not a
/// note and the link should be followed.
pub fn preview(source: &str, frag: &str) -> Option<String> {
    if frag.is_empty() {
        return None;
    }
    let html = Html::parse_document(source);
    let target = html
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
        .find(|e| e.value().attr("id") == Some(frag))?;
    // the container: the target itself or the block around a small anchor
    let mut container = target;
    if !BLOCKS.contains(&target.value().name()) {
        container = target
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|a| BLOCKS.contains(&a.value().name()))?;
    }
    let chain: Vec<ElementRef> = std::iter::once(target)
        .chain(target.ancestors().filter_map(ElementRef::wrap).take(4))
        .collect();
    let note_like = chain
        .iter()
        .any(|e| is_marked_note(*e) || named_like_note(*e))
        || container.value().name() == "li"
        || container
            .ancestors()
            .filter_map(ElementRef::wrap)
            .take(2)
            .any(|a| a.value().name() == "li" || is_marked_note(a));
    if !note_like {
        return None;
    }
    let raw: String = container.text().collect::<Vec<_>>().join(" ");
    let text = clean(&raw);
    if text.is_empty() || text.chars().count() > MAX_CHARS {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(body: &str) -> String {
        format!(
            r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>t</title></head><body>{body}</body></html>"#
        )
    }

    #[test]
    fn an_epub_footnote_aside_is_previewed_without_its_number_and_back_link() {
        let src = page(
            r##"<p>Text<a epub:type="noteref" href="#n1">1</a></p>
            <aside epub:type="footnote" id="n1"><p>1. The note text, short and sweet. <a href="#r1">↩</a></p></aside>"##,
        );
        assert_eq!(
            preview(&src, "n1").unwrap(),
            "The note text, short and sweet."
        );
    }

    #[test]
    fn list_items_and_named_anchors_are_notes() {
        let src = page(
            r##"<ol><li id="fn2">Second note. <a href="#r2">↑</a></li></ol>
            <p><a id="fn3"></a>[3] Anchor note text.</p>
            <div class="footnote" id="x">* A starred note</div>"##,
        );
        assert_eq!(preview(&src, "fn2").unwrap(), "Second note.");
        assert_eq!(preview(&src, "fn3").unwrap(), "Anchor note text.");
        assert_eq!(preview(&src, "x").unwrap(), "A starred note");
    }

    #[test]
    fn headings_chapters_and_plain_paragraphs_are_followed() {
        let src = page(
            r##"<h2 id="part2">Part Two</h2><p id="intro">Just a paragraph of the story.</p>"##,
        );
        assert_eq!(preview(&src, "part2"), None);
        assert_eq!(preview(&src, "intro"), None);
        assert_eq!(preview(&src, "missing"), None);
        assert_eq!(preview(&src, ""), None);
    }

    #[test]
    fn a_long_target_is_not_a_popup() {
        let long = "word ".repeat(400);
        let src = page(&format!(
            r##"<aside epub:type="footnote" id="n"><p>{long}</p></aside>"##
        ));
        assert_eq!(preview(&src, "n"), None);
    }

    #[test]
    fn roles_from_the_dpub_aria_module_count() {
        let src = page(r##"<section id="e" role="doc-endnote"><p>Endnote body.</p></section>"##);
        assert_eq!(preview(&src, "e").unwrap(), "Endnote body.");
    }

    #[test]
    fn cleaning_keeps_text_that_merely_starts_with_a_number() {
        assert_eq!(clean("1984 was a year"), "1984 was a year");
        assert_eq!(clean("12. Twelve"), "Twelve");
        assert_eq!(clean("(4) Four ↩"), "Four");
        assert_eq!(clean("7"), "7");
    }
}
