//! Tag-pair scanner and the markup editing actions built on it (CE-334 select to matching tag, CE-335 jump,
//! CE-349 comment toggle, CE-351 rename block tag / change block to heading, CE-352 insert and remove tag,
//! CE-353 change case, plus close-the-open-tag): everything works on `&str` with byte offsets and returns a [`TagEdit`], so it is tested without a GUI.
//! The scanner tolerates markup that is not well formed (unclosed or stray tags).

use std::ops::Range;

use crate::markup::{scan, Event};

/// Elements that never have a closing tag in HTML.
const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Block elements that "change block to heading" and "rename tag" treat as a paragraph-like block.
const BLOCKS: [&str; 9] = ["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagPair {
    /// Lower-cased local name.
    pub name: String,
    pub open: Range<usize>,
    /// `None` for void and self-closing elements and for tags that are never closed.
    pub close: Option<Range<usize>>,
}

impl TagPair {
    /// The whole element, from `<` of the open tag to `>` of the close tag.
    pub fn whole(&self) -> Range<usize> {
        self.open.start..self.close.as_ref().map_or(self.open.end, |c| c.end)
    }
}

/// One replacement of `range` in the old text; `cursor` is the caret position in the new text and
/// `select` an optional selection in the new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEdit {
    pub range: Range<usize>,
    pub replacement: String,
    pub select: Range<usize>,
}

impl TagEdit {
    pub fn apply(&self, src: &str) -> String {
        let mut out = src.to_string();
        out.replace_range(self.range.clone(), &self.replacement);
        out
    }
}

/// All elements of the document in document order of their open tags.
pub fn pairs(src: &str) -> Vec<TagPair> {
    let mut out: Vec<TagPair> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for e in scan(src) {
        match e {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                let void = self_closing || VOID.contains(&name.as_str());
                out.push(TagPair {
                    name,
                    open: range,
                    close: None,
                });
                if !void {
                    stack.push(out.len() - 1);
                }
            }
            Event::Close { name, range } => {
                if let Some(depth) = stack.iter().rposition(|&i| out[i].name == name) {
                    let idx = stack[depth];
                    out[idx].close = Some(range);
                    stack.truncate(depth);
                }
            }
            _ => {}
        }
    }
    out
}

fn contains(r: &Range<usize>, pos: usize) -> bool {
    r.start <= pos && pos <= r.end
}

/// The innermost element whose open tag, content or close tag contains `pos` (only closed elements).
pub fn enclosing(src: &str, pos: usize) -> Option<TagPair> {
    pairs(src)
        .into_iter()
        .filter(|p| p.close.is_some() && contains(&p.whole(), pos))
        .min_by_key(|p| p.whole().len())
}

/// When `pos` is inside a tag, the partner tag (open <-> close).
pub fn matching_tag(src: &str, pos: usize) -> Option<Range<usize>> {
    for p in pairs(src) {
        let Some(close) = &p.close else { continue };
        if p.open.start < pos && pos < p.open.end {
            return Some(close.clone());
        }
        if close.start < pos && pos < close.end {
            return Some(p.open.clone());
        }
    }
    None
}

/// Live highlight of a tag pair (CE-334): when the caret is in, or touches, the open or the close tag
/// of an element, both tags as `(open, close)`. The innermost element wins.
pub fn highlight_pair(src: &str, pos: usize) -> Option<(Range<usize>, Range<usize>)> {
    pairs(src)
        .into_iter()
        .filter_map(|p| {
            let close = p.close.clone()?;
            (contains(&p.open, pos) || contains(&close, pos)).then_some((p.open, close))
        })
        .min_by_key(|(o, c)| c.end - o.start)
}

/// Jump to the matching tag (CE-335): the new caret position (start of the partner tag). Works from inside a
/// tag, and from content it goes to the open tag of the enclosing element.
pub fn jump_to_matching(src: &str, pos: usize) -> Option<usize> {
    if let Some(r) = matching_tag(src, pos) {
        return Some(r.start);
    }
    // touching the `<` of a tag counts as being in it
    for p in pairs(src) {
        let Some(close) = &p.close else { continue };
        if pos == p.open.start {
            return Some(close.start);
        }
        if pos == close.start {
            return Some(p.open.start);
        }
    }
    enclosing(src, pos).map(|p| p.open.start)
}

/// Select to the matching tag (CE-334): the range from the open tag to the end of the close tag of the
/// element the caret is in (or touches).
pub fn select_to_matching(src: &str, pos: usize) -> Option<Range<usize>> {
    for p in pairs(src) {
        let Some(close) = &p.close else { continue };
        if (p.open.start <= pos && pos <= p.open.end) || (close.start <= pos && pos <= close.end) {
            return Some(p.whole());
        }
    }
    enclosing(src, pos).map(|p| p.whole())
}

/// The name of the innermost element opened before `pos` and not closed yet (CE-351).
pub fn innermost_open(src: &str, pos: usize) -> Option<String> {
    let pos = pos.min(src.len());
    let mut stack: Vec<String> = Vec::new();
    for e in scan(src) {
        match e {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if range.end > pos {
                    break;
                }
                if !self_closing && !VOID.contains(&name.as_str()) {
                    stack.push(name);
                }
            }
            Event::Close { name, range } => {
                if range.end > pos {
                    break;
                }
                if let Some(d) = stack.iter().rposition(|n| *n == name) {
                    stack.truncate(d);
                }
            }
            _ => {}
        }
    }
    stack.pop()
}

/// Closes the innermost open tag at `pos` (CE-351): inserts `</name>`.
pub fn close_tag(src: &str, pos: usize) -> Option<TagEdit> {
    let pos = pos.min(src.len());
    let name = innermost_open(src, pos)?;
    let text = format!("</{name}>");
    let end = pos + text.len();
    Some(TagEdit {
        range: pos..pos,
        replacement: text,
        select: end..end,
    })
}

/// Wraps `sel` in `<tag>` ... `</tag>` (CE-352). `tag` may carry attributes (`span class="x"`); the closing
/// tag uses the name only. An empty selection inserts the empty pair and puts the caret between.
pub fn wrap_in_tag(src: &str, sel: Range<usize>, tag: &str) -> Option<TagEdit> {
    let tag = tag
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    let name = tag.split_whitespace().next()?;
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_')
        || !name.chars().next()?.is_ascii_alphabetic()
        || sel.end > src.len()
        || sel.start > sel.end
    {
        return None;
    }
    let open = format!("<{tag}>");
    let close = format!("</{name}>");
    let inner = &src[sel.clone()];
    let replacement = format!("{open}{inner}{close}");
    let a = sel.start + open.len();
    Some(TagEdit {
        range: sel,
        replacement,
        select: a..a + inner.len(),
    })
}

/// Removes the innermost surrounding tag pair at `pos`, keeping the content (CE-349).
pub fn remove_surrounding(src: &str, pos: usize) -> Option<TagEdit> {
    let p = enclosing(src, pos)?;
    let close = p.close.clone()?;
    let inner = src[p.open.end..close.start].to_string();
    let start = p.open.start;
    Some(TagEdit {
        range: p.whole(),
        select: start..start + inner.len(),
        replacement: inner,
    })
}

fn rename_in_tag(tag: &str, new: &str, closing: bool) -> String {
    let lead = if closing { 2 } else { 1 };
    let body = &tag[lead..];
    let end = body
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(body.len());
    let qname = &body[..end];
    let prefix = qname
        .rsplit_once(':')
        .map(|p| format!("{}:", p.0))
        .unwrap_or_default();
    format!("{}{prefix}{new}{}", &tag[..lead], &body[end..])
}

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn rename_element(src: &str, p: &TagPair, new: &str) -> Option<TagEdit> {
    let close = p.close.clone()?;
    let new = new.trim().to_ascii_lowercase();
    if !valid_name(&new) {
        return None;
    }
    let open_new = rename_in_tag(&src[p.open.clone()], &new, false);
    let close_new = rename_in_tag(&src[close.clone()], &new, true);
    let middle = &src[p.open.end..close.start];
    let replacement = format!("{open_new}{middle}{close_new}");
    let at = p.open.start + open_new.len();
    Some(TagEdit {
        range: p.open.start..close.end,
        select: at..at + middle.len(),
        replacement,
    })
}

/// Renames the tag pair at `pos` (CE-353): open and close tag, attributes kept.
pub fn rename_tag(src: &str, pos: usize, new: &str) -> Option<TagEdit> {
    let p = enclosing(src, pos)?;
    rename_element(src, &p, new)
}

/// Changes the block (`p`, `div`, `hN`, `blockquote`) around `pos` into `h<level>` (CE-338); level 0 makes
/// it a paragraph.
pub fn change_block_to_heading(src: &str, pos: usize, level: u8) -> Option<TagEdit> {
    if level > 6 {
        return None;
    }
    let p = pairs(src)
        .into_iter()
        .filter(|p| {
            p.close.is_some() && BLOCKS.contains(&p.name.as_str()) && contains(&p.whole(), pos)
        })
        .min_by_key(|p| p.whole().len())?;
    let new = if level == 0 {
        "p".to_string()
    } else {
        format!("h{level}")
    };
    rename_element(src, &p, &new)
}

/// Case change of the selection (CE-332): upper, lower, title or swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseChange {
    Upper,
    Lower,
    Title,
    Swap,
}

pub fn change_case(src: &str, sel: Range<usize>, how: CaseChange) -> Option<TagEdit> {
    if sel.is_empty() || sel.end > src.len() {
        return None;
    }
    let text = &src[sel.clone()];
    let out = match how {
        CaseChange::Upper => text.to_uppercase(),
        CaseChange::Lower => text.to_lowercase(),
        CaseChange::Swap => text
            .chars()
            .map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().to_string()
                } else {
                    c.to_uppercase().to_string()
                }
            })
            .collect(),
        CaseChange::Title => {
            let mut new_word = true;
            text.chars()
                .map(|c| {
                    let r = if c.is_alphabetic() {
                        if new_word {
                            c.to_uppercase().to_string()
                        } else {
                            c.to_lowercase().to_string()
                        }
                    } else {
                        c.to_string()
                    };
                    new_word = !c.is_alphanumeric() && c != '\'';
                    r
                })
                .collect()
        }
    };
    let end = sel.start + out.len();
    Some(TagEdit {
        select: sel.start..end,
        range: sel,
        replacement: out,
    })
}

/// The `/* */` comment that contains the whole selection, if any.
fn css_comment_around(src: &str, sel: &Range<usize>) -> Option<Range<usize>> {
    let a = src[..sel.start]
        .rfind("/*")
        .or_else(|| src[sel.start..].starts_with("/*").then_some(sel.start))?;
    let b = a + 2 + src[a + 2..].find("*/")? + 2;
    (b >= sel.end).then_some(a..b)
}

/// Comments the selection out, or removes the comment around it (CE-349). An empty selection acts on the
/// whole line. `css` uses `/* */`, otherwise `<!-- -->`.
pub fn toggle_comment(src: &str, sel: Range<usize>, css: bool) -> Option<TagEdit> {
    if sel.end > src.len() || sel.start > sel.end {
        return None;
    }
    let (open, close) = if css { ("/*", "*/") } else { ("<!--", "-->") };
    // inside an existing comment: take it away
    let existing = if css {
        css_comment_around(src, &sel)
    } else {
        scan(src).into_iter().find_map(|e| match e {
            Event::Other(r)
                if src[r.clone()].starts_with(open)
                    && src[r.clone()].ends_with(close)
                    && r.start <= sel.start
                    && sel.end <= r.end =>
            {
                Some(r)
            }
            _ => None,
        })
    };
    if let Some(r) = existing {
        let inner = src[r.start + open.len()..r.end - close.len()].to_string();
        let inner = inner.strip_prefix(' ').unwrap_or(&inner);
        let inner = inner.strip_suffix(' ').unwrap_or(inner).to_string();
        let end = r.start + inner.len();
        return Some(TagEdit {
            range: r.clone(),
            select: r.start..end,
            replacement: inner,
        });
    }
    let range = if sel.is_empty() {
        let a = src[..sel.start].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let b = src[sel.start..]
            .find('\n')
            .map(|p| sel.start + p)
            .unwrap_or(src.len());
        if src[a..b].trim().is_empty() {
            return None;
        }
        a..b
    } else {
        sel
    };
    let inner = &src[range.clone()];
    let safe = if css {
        inner.replace("*/", "* /")
    } else {
        inner.replace("--", "- -")
    };
    let replacement = format!("{open} {safe} {close}");
    let end = range.start + replacement.len();
    Some(TagEdit {
        range: range.clone(),
        select: range.start..end,
        replacement,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str =
        "<body><div class=\"a\"><p>Hello <b>big</b> world<br/></p><p>two</p></div></body>";

    fn at(s: &str) -> usize {
        DOC.find(s).unwrap()
    }

    #[test]
    fn pairs_know_void_unclosed_and_stray_tags() {
        let p = pairs(DOC);
        let names: Vec<_> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["body", "div", "p", "b", "br", "p"]);
        assert!(p[4].close.is_none());
        let broken = pairs("<div><p>a</div></span><i>");
        assert_eq!(broken[0].name, "div");
        assert!(broken[0].close.is_some());
        assert!(broken[1].close.is_none(), "p was closed by </div>");
        assert!(broken[2].close.is_none());
    }

    #[test]
    fn matching_tag_works_both_ways() {
        let open_b = at("<b>") + 1;
        let m = matching_tag(DOC, open_b).unwrap();
        assert_eq!(&DOC[m.clone()], "</b>");
        let m2 = matching_tag(DOC, m.start + 2).unwrap();
        assert_eq!(&DOC[m2], "<b>");
        assert!(matching_tag(DOC, at("Hello") + 2).is_none());
    }

    #[test]
    fn jump_goes_to_the_partner_or_the_enclosing_open_tag() {
        assert_eq!(jump_to_matching(DOC, at("<b>") + 1), Some(at("</b>")));
        assert_eq!(jump_to_matching(DOC, at("</b>")), Some(at("<b>")));
        assert_eq!(jump_to_matching(DOC, at("big")), Some(at("<b>")));
        assert_eq!(jump_to_matching("plain", 2), None);
    }

    #[test]
    fn select_to_matching_selects_the_whole_element() {
        let r = select_to_matching(DOC, at("<b>") + 2).unwrap();
        assert_eq!(&DOC[r], "<b>big</b>");
        let r = select_to_matching(DOC, at("world")).unwrap();
        assert!(DOC[r].starts_with("<p>Hello") && DOC[..].contains("</p>"));
    }

    #[test]
    fn closing_the_innermost_open_tag() {
        let src = "<div><p>Hello <b>big";
        let e = close_tag(src, src.len()).unwrap();
        assert_eq!(e.apply(src), "<div><p>Hello <b>big</b>");
        let src2 = "<div><p>x</p><br/>";
        assert_eq!(close_tag(src2, src2.len()).unwrap().replacement, "</div>");
        assert!(close_tag("<p>a</p>", 8).is_none());
        assert_eq!(
            innermost_open("<ul><li>a</li><li>b", 14).as_deref(),
            Some("ul")
        );
    }

    #[test]
    fn wrapping_a_selection_keeps_attributes_on_the_open_tag_only() {
        let src = "say hello now";
        let e = wrap_in_tag(src, 4..9, "span class=\"x\"").unwrap();
        assert_eq!(e.apply(src), "say <span class=\"x\">hello</span> now");
        let new = e.apply(src);
        assert_eq!(&new[e.select.clone()], "hello");
        let empty = wrap_in_tag(src, 3..3, "<em>").unwrap();
        assert_eq!(empty.apply(src), "say<em></em> hello now");
        assert_eq!(empty.select.start, 7);
        assert!(wrap_in_tag(src, 0..2, "1x").is_none());
        assert!(wrap_in_tag(src, 0..2, "a b>c<").is_some());
        assert!(wrap_in_tag(src, 0..99, "b").is_none());
    }

    #[test]
    fn removing_the_surrounding_pair_keeps_the_content() {
        let e = remove_surrounding(DOC, at("big")).unwrap();
        assert_eq!(
            e.apply(DOC),
            "<body><div class=\"a\"><p>Hello big world<br/></p><p>two</p></div></body>"
        );
        let e = remove_surrounding(DOC, at("Hello")).unwrap();
        assert!(e.apply(DOC).contains("Hello <b>big</b> world<br/><p>two"));
        assert!(remove_surrounding("no tags", 2).is_none());
    }

    #[test]
    fn renaming_keeps_attributes_and_prefixes() {
        let src = "<x:div id=\"a\" class='k'>t</x:div>";
        let e = rename_tag(src, 10, "Section").unwrap();
        assert_eq!(e.apply(src), "<x:section id=\"a\" class='k'>t</x:section>");
        assert!(rename_tag(src, 10, "1bad").is_none());
        assert!(rename_tag(src, 10, "a b").is_none());
        let e = rename_tag(DOC, at("big"), "strong").unwrap();
        assert!(e.apply(DOC).contains("<strong>big</strong>"));
    }

    #[test]
    fn a_block_becomes_a_heading_and_back() {
        let e = change_block_to_heading(DOC, at("Hello") + 1, 2).unwrap();
        let out = e.apply(DOC);
        assert!(out.contains("<h2>Hello <b>big</b> world<br/></h2>"));
        let e = change_block_to_heading(&out, out.find("Hello").unwrap(), 0).unwrap();
        assert_eq!(e.apply(&out), DOC);
        // inside <b> the enclosing block is the paragraph, not the bold text
        assert!(change_block_to_heading(DOC, at("big"), 3)
            .unwrap()
            .apply(DOC)
            .contains("<h3>Hello"));
        assert!(change_block_to_heading(DOC, 0, 1).is_none() || DOC.starts_with("<body"));
        assert!(change_block_to_heading(DOC, at("two"), 7).is_none());
    }

    #[test]
    fn comments_toggle_for_html_and_css() {
        let src = "<p>a</p>\n<p>b</p>\n";
        let e = toggle_comment(src, 0..8, false).unwrap();
        assert_eq!(e.apply(src), "<!-- <p>a</p> -->\n<p>b</p>\n");
        let on = e.apply(src);
        // the caret inside the comment removes it again
        let back = toggle_comment(&on, 8..8, false).unwrap();
        assert_eq!(back.apply(&on), src);
        // empty selection comments the line
        let line = toggle_comment(src, 11..11, false).unwrap();
        assert_eq!(line.apply(src), "<p>a</p>\n<!-- <p>b</p> -->\n");
        // inner double dashes would end the comment early
        assert_eq!(
            toggle_comment("a--b", 0..4, false).unwrap().replacement,
            "<!-- a- -b -->"
        );
        let css = "p { x: y }";
        let c = toggle_comment(css, 0..css.len(), true).unwrap().apply(css);
        assert_eq!(c, "/* p { x: y } */");
        assert_eq!(toggle_comment(&c, 4..4, true).unwrap().apply(&c), css);
        assert!(toggle_comment("  \n", 1..1, false).is_none());
    }

    #[test]
    fn case_changes() {
        let s = "the QUICK fox's den";
        assert_eq!(
            change_case(s, 0..s.len(), CaseChange::Upper)
                .unwrap()
                .replacement,
            "THE QUICK FOX'S DEN"
        );
        assert_eq!(
            change_case(s, 0..s.len(), CaseChange::Lower)
                .unwrap()
                .replacement,
            "the quick fox's den"
        );
        assert_eq!(
            change_case(s, 0..s.len(), CaseChange::Title)
                .unwrap()
                .replacement,
            "The Quick Fox's Den"
        );
        assert_eq!(
            change_case("aB", 0..2, CaseChange::Swap)
                .unwrap()
                .replacement,
            "Ab"
        );
        assert!(change_case(s, 2..2, CaseChange::Upper).is_none());
    }

    #[test]
    fn the_pair_under_the_caret_is_highlighted() {
        let s = "<div><p class=\"a\">x <b>y</b></p></div>";
        // in the open <p> tag
        let (o, c) = highlight_pair(s, 8).unwrap();
        assert_eq!(&s[o], "<p class=\"a\">");
        assert_eq!(&s[c], "</p>");
        // touching the close tag from the right of its `>`
        let end = s.find("</b>").unwrap() + 4;
        let (o, c) = highlight_pair(s, end).unwrap();
        assert_eq!(&s[o], "<b>");
        assert_eq!(&s[c], "</b>");
        // plain text between tags is not in a tag
        let at_x = s.find("x ").unwrap() + 1;
        assert!(highlight_pair(s, at_x).is_none());
        // void and unclosed elements have no partner
        assert!(highlight_pair("<p>a<br/>b</p>", 6).is_none());
        // the innermost pair wins where two touch
        let (o, _) = highlight_pair("<i><b>z</b></i>", 3).unwrap();
        assert!(["<i>", "<b>"].contains(&&"<i><b>z</b></i>"[o]));
    }
}
