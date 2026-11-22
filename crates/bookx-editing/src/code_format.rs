//! Formatting and insertion actions of the code view without GTK: bold/italic/... toggles (CE-350), split a tag at the
//! caret (CE-352), the markup the Insert Image and Insert Hyperlink dialogs produce (CE-354) and closing a tag when
//! `</` is typed (CE-348). Everything returns a [`TagEdit`] over byte offsets.

use std::ops::Range;

use crate::code_nav::attr_spans;
use crate::markup::{Event, remove_tag_attr, scan};
use crate::tags::{TagEdit, innermost_open, pairs};

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Underline,
    Strike,
    Superscript,
    Subscript,
    Code,
}

impl Format {
    pub const ALL: [Format; 7] = [
        Format::Bold,
        Format::Italic,
        Format::Underline,
        Format::Strike,
        Format::Superscript,
        Format::Subscript,
        Format::Code,
    ];

    /// The element written when the format is applied.
    pub fn tag(self) -> &'static str {
        match self {
            Format::Bold => "b",
            Format::Italic => "i",
            Format::Underline => "u",
            Format::Strike => "s",
            Format::Superscript => "sup",
            Format::Subscript => "sub",
            Format::Code => "code",
        }
    }

    /// Every element that already means this format.
    fn names(self) -> &'static [&'static str] {
        match self {
            Format::Bold => &["b", "strong"],
            Format::Italic => &["i", "em"],
            Format::Underline => &["u"],
            Format::Strike => &["s", "del", "strike"],
            Format::Superscript => &["sup"],
            Format::Subscript => &["sub"],
            Format::Code => &["code", "tt"],
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Format::Bold => "bold",
            Format::Italic => "italic",
            Format::Underline => "underline",
            Format::Strike => "strike",
            Format::Superscript => "sup",
            Format::Subscript => "sub",
            Format::Code => "code",
        }
    }

    pub fn from_id(id: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// The selection starts or ends inside a tag, or crosses element boundaries.
    Unbalanced,
    /// The caret is inside a tag, a comment or a script.
    NotText,
}

/// Whether `s` is a sequence of complete elements and text (every tag opened inside is closed inside).
pub fn is_balanced(s: &str) -> bool {
    let mut stack: Vec<String> = Vec::new();
    for e in scan(s) {
        match e {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if !s[range].ends_with('>') {
                    return false;
                }
                if !self_closing && !VOID.contains(&name.as_str()) {
                    stack.push(name);
                }
            }
            Event::Close { name, .. } if stack.pop().as_deref() != Some(name.as_str()) => {
                return false;
            }
            _ => {}
        }
    }
    stack.is_empty()
}

pub(crate) fn inside_markup(src: &str, pos: usize) -> bool {
    scan(src).into_iter().any(|e| match e {
        Event::Open { range, .. } | Event::Close { range, .. } | Event::Other(range) => {
            pos > range.start && pos < range.end
        }
        Event::Text(_) => false,
    })
}

/// Applies `format` to the selection, or removes it again (CE-350):
/// * the selection is the content (or the whole) of an element of that format: the element goes away;
/// * the selection lies inside such an element: the element is closed before and reopened after the selection;
/// * otherwise the selection is wrapped. An empty selection inserts the empty pair with the caret inside.
pub fn toggle_format(src: &str, sel: Range<usize>, f: Format) -> Result<TagEdit, FormatError> {
    if sel.start > sel.end
        || sel.end > src.len()
        || !src.is_char_boundary(sel.start)
        || !src.is_char_boundary(sel.end)
    {
        return Err(FormatError::NotText);
    }
    if inside_markup(src, sel.start) || inside_markup(src, sel.end) {
        return Err(FormatError::NotText);
    }
    let matching: Vec<_> = pairs(src)
        .into_iter()
        .filter(|p| f.names().contains(&p.name.as_str()) && p.close.is_some())
        .collect();
    // unwrap: the selection is exactly the element or its content
    for p in &matching {
        let close = p.close.clone().unwrap();
        let content = p.open.end..close.start;
        if !sel.is_empty() && (sel == content || sel == p.whole()) {
            let inner = src[content].to_string();
            let at = p.open.start;
            return Ok(TagEdit {
                range: p.whole(),
                select: at..at + inner.len(),
                replacement: inner,
            });
        }
    }
    // innermost element of this format around the selection
    let around = matching
        .iter()
        .filter(|p| {
            let c = p.close.clone().unwrap();
            p.open.end <= sel.start && sel.end <= c.start
        })
        .max_by_key(|p| p.open.start);
    if let Some(p) = around {
        let close = p.close.clone().unwrap();
        if sel.is_empty() {
            let inner = src[p.open.end..close.start].to_string();
            let at = p.open.start;
            return Ok(TagEdit {
                range: p.whole(),
                select: at..at + inner.len(),
                replacement: inner,
            });
        }
        let ok = is_balanced(&src[p.open.end..sel.start])
            && is_balanced(&src[sel.clone()])
            && is_balanced(&src[sel.end..close.start]);
        if !ok {
            return Err(FormatError::Unbalanced);
        }
        let open_text = &src[p.open.clone()];
        let close_text = &src[close.clone()];
        let selected = &src[sel.clone()];
        if sel.start == p.open.end {
            // the start of the element: drop its start tag, reopen after the selection
            let rep = format!("{selected}{open_text}");
            let at = p.open.start;
            return Ok(TagEdit {
                range: p.open.start..sel.end,
                select: at..at + selected.len(),
                replacement: rep,
            });
        }
        if sel.end == close.start {
            let rep = format!("{close_text}{selected}");
            let at = sel.start + close_text.len();
            return Ok(TagEdit {
                range: sel.start..close.end,
                select: at..at + selected.len(),
                replacement: rep,
            });
        }
        let rep = format!("{close_text}{selected}{open_text}");
        let at = sel.start + close_text.len();
        return Ok(TagEdit {
            range: sel.clone(),
            select: at..at + selected.len(),
            replacement: rep,
        });
    }
    if !is_balanced(&src[sel.clone()]) {
        return Err(FormatError::Unbalanced);
    }
    let tag = f.tag();
    let open = format!("<{tag}>");
    let inner = &src[sel.clone()];
    let a = sel.start + open.len();
    Ok(TagEdit {
        range: sel.clone(),
        replacement: format!("{open}{inner}</{tag}>"),
        select: a..a + inner.len(),
    })
}

/// Formats found around the caret or inside the selection start: which of the toggles are on (for menu state).
pub fn formats_at(src: &str, pos: usize) -> Vec<Format> {
    let all = pairs(src);
    Format::ALL
        .into_iter()
        .filter(|f| {
            all.iter().any(|p| {
                f.names().contains(&p.name.as_str())
                    && p.close
                        .as_ref()
                        .is_some_and(|c| p.open.end <= pos && pos <= c.start)
            })
        })
        .collect()
}

/// Splits the element around the caret in two (CE-352): `<p>a|b</p>` becomes `<p>a</p><p>b</p>`. The copy of
/// the start tag loses its `id`. The caret ends between the two elements' second start tag and its content.
pub fn split_tag(src: &str, pos: usize) -> Option<TagEdit> {
    if pos > src.len() || !src.is_char_boundary(pos) || inside_markup(src, pos) {
        return None;
    }
    let p = pairs(src)
        .into_iter()
        .filter(|p| {
            p.close
                .as_ref()
                .is_some_and(|c| p.open.end <= pos && pos <= c.start)
        })
        .max_by_key(|p| p.open.start)?;
    let close = p.close.clone()?;
    let open_copy = remove_tag_attr(&src[p.open.clone()], "id");
    let rep = format!(
        "</{}>{}",
        src[close.start + 2..close.end - 1].trim(),
        open_copy
    );
    let end = pos + rep.len();
    Some(TagEdit {
        range: pos..pos,
        replacement: rep,
        select: end..end,
    })
}

/// `&`, `<`, `>` and `"` escaped for text and double-quoted attribute values.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `<img src="…" alt="…"/>`; `alt` is always written (an empty one marks a decorative image).
pub fn image_markup(src_href: &str, alt: &str, title: &str) -> String {
    let mut s = format!("<img src=\"{}\" alt=\"{}\"", escape(src_href), escape(alt));
    if !title.trim().is_empty() {
        s.push_str(&format!(" title=\"{}\"", escape(title.trim())));
    }
    s.push_str("/>");
    s
}

/// `<a href="…" title="…">text</a>`.
pub fn link_markup(href: &str, text: &str, title: &str) -> String {
    let mut s = format!("<a href=\"{}\"", escape(href));
    if !title.trim().is_empty() {
        s.push_str(&format!(" title=\"{}\"", escape(title.trim())));
    }
    s.push_str(&format!(">{}</a>", escape(text)));
    s
}

/// Inserts an image after the selection (or at the caret).
pub fn insert_image(
    src: &str,
    sel: Range<usize>,
    href: &str,
    alt: &str,
    title: &str,
) -> Option<TagEdit> {
    if sel.end > src.len() || inside_markup(src, sel.end) {
        return None;
    }
    let m = image_markup(href, alt, title);
    let at = sel.end;
    Some(TagEdit {
        range: at..at,
        select: at + m.len()..at + m.len(),
        replacement: m,
    })
}

/// Makes a hyperlink: with a selection and no `text` the selected markup becomes the link text, otherwise
/// the selection is replaced by `text`.
pub fn insert_link(
    src: &str,
    sel: Range<usize>,
    href: &str,
    text: Option<&str>,
    title: &str,
) -> Result<TagEdit, FormatError> {
    if sel.end > src.len() || inside_markup(src, sel.start) || inside_markup(src, sel.end) {
        return Err(FormatError::NotText);
    }
    // inside a link already: change its address (and title) instead of nesting a second link
    let enclosing = pairs(src)
        .into_iter()
        .filter(|p| {
            p.name == "a"
                && p.close
                    .as_ref()
                    .is_some_and(|c| p.open.end <= sel.start && sel.end <= c.start)
        })
        .max_by_key(|p| p.open.start);
    if let Some(p) = enclosing {
        let mut new_open = crate::markup::set_tag_attr(&src[p.open.clone()], "href", href);
        if !title.trim().is_empty() {
            new_open = crate::markup::set_tag_attr(&new_open, "title", title.trim());
        }
        let content = p.close.as_ref().map_or(0, |c| c.start - p.open.end);
        let a = p.open.start + new_open.len();
        return Ok(TagEdit {
            range: p.open,
            select: a..a + content,
            replacement: new_open,
        });
    }
    let mut open = format!("<a href=\"{}\"", escape(href));
    if !title.trim().is_empty() {
        open.push_str(&format!(" title=\"{}\"", escape(title.trim())));
    }
    open.push('>');
    let (inner, select_inner) = match text {
        Some(t) if !t.is_empty() => (escape(t), true),
        _ if !sel.is_empty() => {
            if !is_balanced(&src[sel.clone()]) {
                return Err(FormatError::Unbalanced);
            }
            (src[sel.clone()].to_string(), true)
        }
        _ => (String::new(), false),
    };
    let replacement = format!("{open}{inner}</a>");
    let a = sel.start + open.len();
    Ok(TagEdit {
        range: sel,
        select: if select_inner {
            a..a + inner.len()
        } else {
            a..a
        },
        replacement,
    })
}

/// After `</` was typed at `pos`: inserts the name and `>` of the innermost open element (CE-348).
pub fn auto_close(src: &str, pos: usize) -> Option<TagEdit> {
    if pos < 2 || pos > src.len() || !src.is_char_boundary(pos) || !src[..pos].ends_with("</") {
        return None;
    }
    // not inside a comment, CDATA or the text of a script/style
    let before = pos - 2;
    let skip = scan(src).into_iter().any(|e| match e {
        Event::Other(r) => before > r.start && before < r.end,
        _ => false,
    });
    if skip {
        return None;
    }
    // a script/style body is one text event; its open tag is the previous event
    let mut prev_raw = false;
    for e in scan(src) {
        match e {
            Event::Open { name, range, .. } => {
                prev_raw = (name == "script" || name == "style") && range.end <= before;
            }
            Event::Text(r) => {
                if prev_raw && before >= r.start && before <= r.end {
                    return None;
                }
            }
            Event::Close { range, .. } => {
                if range.start >= before {
                    break;
                }
                prev_raw = false;
            }
            _ => {}
        }
    }
    let name = innermost_open(src, before)?;
    // the closing tag of that element is there already (typing `>` of the open tag may have added it)
    if src[pos..].starts_with(&format!("</{name}>")) {
        return None;
    }
    let rep = format!("{name}>");
    let end = pos + rep.len();
    Some(TagEdit {
        range: pos..pos,
        replacement: rep,
        select: end..end,
    })
}

/// What closing a tag on `</` left behind: the caret (character offset) stands after the inserted `p>` and the
/// user, who is typing `</p>` anyway, would otherwise end up with `</p>p>`. The typed characters that match what
/// was inserted are swallowed instead (like typing over an auto-inserted bracket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub caret: usize,
    pub rest: String,
}

/// Decides what a key press does with the pending completion: `true` when the typed character is swallowed.
/// Any other key (or the caret having moved) ends the completion.
pub fn type_over(pending: &mut Option<Pending>, caret: usize, typed: Option<char>) -> bool {
    let Some(p) = pending.as_mut() else {
        return false;
    };
    match typed {
        Some(c) if caret == p.caret && p.rest.starts_with(c) => {
            p.rest.drain(..c.len_utf8());
            if p.rest.is_empty() {
                *pending = None;
            }
            true
        }
        _ => {
            *pending = None;
            false
        }
    }
}

/// The `href` of the link around `pos`, if any (used to pre-fill the hyperlink dialog).
pub fn link_at(src: &str, pos: usize) -> Option<String> {
    let p = pairs(src)
        .into_iter()
        .filter(|p| {
            p.name == "a"
                && p.close
                    .as_ref()
                    .is_some_and(|c| p.open.start <= pos && pos <= c.end)
        })
        .max_by_key(|p| p.open.start)?;
    attr_spans(src, p.open)
        .into_iter()
        .find(|a| a.name == "href")
        .map(|a| src[a.value].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, sel: Range<usize>, f: Format) -> String {
        toggle_format(src, sel, f).unwrap().apply(src)
    }

    fn rng(s: &str, needle: &str) -> Range<usize> {
        let a = s.find(needle).unwrap();
        a..a + needle.len()
    }

    #[test]
    fn wrap_selection_and_empty_selection() {
        let s = "<p>hello world</p>";
        assert_eq!(
            run(s, rng(s, "world"), Format::Bold),
            "<p>hello <b>world</b></p>"
        );
        assert_eq!(
            run(s, rng(s, "hello"), Format::Superscript),
            "<p><sup>hello</sup> world</p>"
        );
        let e = toggle_format(s, 5..5, Format::Italic).unwrap();
        assert_eq!(e.apply(s), "<p>he<i></i>llo world</p>");
        assert_eq!(e.select, 5 + 3..5 + 3, "the caret sits between the tags");
    }

    #[test]
    fn toggling_off_exact_content_whole_element_and_equivalents() {
        let s = "<p>a <b>bold</b> <strong>strong</strong></p>";
        assert_eq!(
            run(s, rng(s, "bold"), Format::Bold),
            "<p>a bold <strong>strong</strong></p>"
        );
        assert_eq!(
            run(s, rng(s, "<b>bold</b>"), Format::Bold),
            "<p>a bold <strong>strong</strong></p>"
        );
        let strong = rng(s, "strong</strong>");
        let content = strong.start..strong.start + 6;
        assert_eq!(
            run(s, content, Format::Bold),
            "<p>a <b>bold</b> strong</p>",
            "<strong> counts as bold"
        );
    }

    #[test]
    fn toggling_off_part_of_an_element_splits_it() {
        let s = "<p><b>one two three</b></p>";
        assert_eq!(
            run(s, rng(s, "two"), Format::Bold),
            "<p><b>one </b>two<b> three</b></p>"
        );
        assert_eq!(
            run(s, rng(s, "one"), Format::Bold),
            "<p>one<b> two three</b></p>"
        );
        assert_eq!(
            run(s, rng(s, "three"), Format::Bold),
            "<p><b>one two </b>three</p>"
        );
        let e = toggle_format(s, rng(s, "two"), Format::Bold).unwrap();
        assert_eq!(
            &e.apply(s)[e.select.clone()],
            "two",
            "the selection stays selected"
        );
        // attributes of the original element survive the reopen
        let c = "<p><b class=\"x\">abc</b></p>";
        let b = c.find("abc").unwrap() + 1;
        assert_eq!(
            run(c, b..b + 1, Format::Bold),
            "<p><b class=\"x\">a</b>b<b class=\"x\">c</b></p>"
        );
    }

    #[test]
    fn caret_inside_a_formatted_element_removes_it() {
        let s = "<p><i>slanted</i></p>";
        assert_eq!(run(s, 8..8, Format::Italic), "<p>slanted</p>");
    }

    #[test]
    fn unbalanced_selections_are_refused() {
        let s = "<p>a <i>b</i> c</p>";
        assert_eq!(
            toggle_format(s, 6..9, Format::Bold),
            Err(FormatError::NotText),
            "starts inside a tag"
        );
        let t = "<p>a <i>b</i></p><p>c</p>";
        let r = rng(t, "b</i></p><p>c");
        assert_eq!(
            toggle_format(t, r, Format::Bold),
            Err(FormatError::Unbalanced)
        );
        assert_eq!(
            toggle_format(t, 40..50, Format::Bold),
            Err(FormatError::NotText)
        );
        // nested content is fine
        let n = "<p>x <i>y</i> z</p>";
        assert_eq!(
            run(n, rng(n, "x <i>y</i> z"), Format::Bold),
            "<p><b>x <i>y</i> z</b></p>"
        );
        // a split must not cut through a child element
        let m = "<p><b>a <i>b c</i> d</b></p>";
        let r = rng(m, "b c");
        assert_eq!(
            toggle_format(m, r, Format::Bold),
            Err(FormatError::Unbalanced)
        );
    }

    #[test]
    fn comments_unicode_and_every_format() {
        let s = "<p>é<!-- c --> ü</p>";
        let r = rng(s, "é<!-- c --> ü");
        assert_eq!(run(s, r, Format::Code), "<p><code>é<!-- c --> ü</code></p>");
        assert_eq!(
            toggle_format(s, 7..8, Format::Bold),
            Err(FormatError::NotText),
            "inside a comment"
        );
        for f in Format::ALL {
            let w = run("<p>x</p>", 3..4, f);
            assert_eq!(w, format!("<p><{0}>x</{0}></p>", f.tag()));
            assert_eq!(Format::from_id(f.id()), Some(f));
        }
        assert_eq!(
            formats_at("<p><b><i>x</i></b></p>", 10),
            vec![Format::Bold, Format::Italic]
        );
        assert_eq!(formats_at("<p>x</p>", 4), vec![]);
    }

    #[test]
    fn split_tag_at_the_caret() {
        let s = "<p id=\"a\" class=\"c\">first second</p>";
        let e = split_tag(s, s.find("second").unwrap()).unwrap();
        assert_eq!(
            e.apply(s),
            "<p id=\"a\" class=\"c\">first </p><p class=\"c\">second</p>"
        );
        assert_eq!(&e.apply(s)[e.select.start..], "second</p>");
        let n = "<div><p>ab<i>cd</i></p></div>";
        assert_eq!(
            split_tag(n, n.find("cd").unwrap() + 1).unwrap().apply(n),
            "<div><p>ab<i>c</i><i>d</i></p></div>"
        );
        assert!(split_tag("<p>x</p>", 1).is_none(), "inside a tag");
        assert!(split_tag("plain", 2).is_none());
        assert!(split_tag("<br/>", 5).is_none());
        assert_eq!(
            split_tag("<h:p>é|</h:p>", 7)
                .map(|e| e.apply("<h:p>é|</h:p>"))
                .as_deref(),
            Some("<h:p>é</h:p><h:p>|</h:p>")
        );
    }

    #[test]
    fn insert_markup_is_escaped() {
        assert_eq!(
            image_markup("../Images/a b.png", "A \"cat\" & dog", ""),
            "<img src=\"../Images/a b.png\" alt=\"A &quot;cat&quot; &amp; dog\"/>"
        );
        assert_eq!(
            image_markup("a.png", "", "T"),
            "<img src=\"a.png\" alt=\"\" title=\"T\"/>"
        );
        assert_eq!(
            link_markup("c2.xhtml#t", "A <b>", " Hi "),
            "<a href=\"c2.xhtml#t\" title=\"Hi\">A &lt;b&gt;</a>"
        );
        let s = "<p>see here now</p>";
        let e = insert_link(s, rng(s, "here"), "x.xhtml", None, "").unwrap();
        assert_eq!(e.apply(s), "<p>see <a href=\"x.xhtml\">here</a> now</p>");
        assert_eq!(&e.apply(s)[e.select.clone()], "here");
        let e = insert_link(s, rng(s, "here"), "x.xhtml", Some("there"), "").unwrap();
        assert_eq!(e.apply(s), "<p>see <a href=\"x.xhtml\">there</a> now</p>");
        let e = insert_link(s, 6..6, "x", None, "").unwrap();
        assert_eq!(e.apply(s), "<p>see<a href=\"x\"></a> here now</p>");
        assert_eq!(
            insert_link(s, 2..9, "x", None, ""),
            Err(FormatError::NotText)
        );
        // a link inside a link is not made: the address of the surrounding one changes
        let l = "<p><a href=\"old.xhtml\" class=\"c\">word</a></p>";
        let w = l.find("word").unwrap();
        let e = insert_link(l, w..w + 4, "new.xhtml#x", Some("ignored"), "T").unwrap();
        assert_eq!(
            e.apply(l),
            "<p><a href=\"new.xhtml#x\" class=\"c\" title=\"T\">word</a></p>"
        );
        assert_eq!(&e.apply(l)[e.select.clone()], "word");
        let e = insert_link(l, w + 1..w + 1, "n.xhtml", None, "").unwrap();
        assert_eq!(
            e.apply(l),
            "<p><a href=\"n.xhtml\" class=\"c\">word</a></p>"
        );
        let i = insert_image(s, 3..3, "a.png", "alt", "").unwrap();
        assert_eq!(
            i.apply(s),
            "<p><img src=\"a.png\" alt=\"alt\"/>see here now</p>"
        );
        assert!(insert_image(s, 1..1, "a.png", "", "").is_none());
        assert_eq!(
            link_at("<p><a href=\"u.html#x\">t</a></p>", 15).as_deref(),
            Some("u.html#x")
        );
        assert_eq!(link_at("<p>t</p>", 4), None);
    }

    #[test]
    fn type_over_swallows_the_rest_of_a_closing_tag() {
        let mut p = Some(Pending {
            caret: 10,
            rest: "div>".into(),
        });
        assert!(type_over(&mut p, 10, Some('d')));
        assert!(type_over(&mut p, 10, Some('i')));
        assert!(!type_over(&mut p, 10, Some('x')), "another letter ends it");
        assert_eq!(p, None);
        assert!(!type_over(&mut p, 10, Some('v')), "nothing pending");
        let mut p = Some(Pending {
            caret: 10,
            rest: "p>".into(),
        });
        assert!(!type_over(&mut p, 11, Some('p')), "the caret moved away");
        assert_eq!(p, None);
        let mut p = Some(Pending {
            caret: 3,
            rest: "é>".into(),
        });
        assert!(type_over(&mut p, 3, Some('é')));
        assert!(type_over(&mut p, 3, Some('>')));
        assert_eq!(p, None, "all swallowed");
        let mut p = Some(Pending {
            caret: 3,
            rest: "p>".into(),
        });
        assert!(!type_over(&mut p, 3, None), "a non-character key ends it");
        assert_eq!(p, None);
    }

    #[test]
    fn auto_close_on_slash() {
        let s = "<div><p>text</";
        assert_eq!(auto_close(s, s.len()).unwrap().apply(s), "<div><p>text</p>");
        let s = "<div><p>a</p></";
        assert_eq!(
            auto_close(s, s.len()).unwrap().apply(s),
            "<div><p>a</p></div>"
        );
        let s = "<p>a <br/> <img src=\"x\"> </";
        assert_eq!(
            auto_close(s, s.len()).unwrap().apply(s),
            "<p>a <br/> <img src=\"x\"> </p>",
            "void elements are skipped"
        );
        assert!(auto_close("text</", 6).is_none(), "nothing open");
        assert!(auto_close("<p><!-- </", 10).is_none(), "inside a comment");
        assert!(
            auto_close("<script>if (a </", 16).is_none(),
            "inside a script"
        );
        assert!(auto_close("<p>x<", 5).is_none());
        assert!(
            auto_close("<p>x</</p>", 6).is_none(),
            "the closing tag follows already"
        );
        let s = "<body><p>x</</body>";
        assert_eq!(
            auto_close(s, 12).unwrap().apply(s),
            "<body><p>x</p></body>",
            "another closing tag may follow"
        );
        let s = "<p>é</";
        assert_eq!(auto_close(s, s.len()).unwrap().apply(s), "<p>é</p>");
        let mid = "<p>a</ b</p>";
        assert_eq!(auto_close(mid, 6).unwrap().apply(mid), "<p>a</p> b</p>");
    }
}
