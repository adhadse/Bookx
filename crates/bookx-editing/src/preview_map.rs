//! Maps the blocks of the preview document back to the source text (CE-243, CE-244, CE-246, CE-124).
//!
//! The preview parser (`bookx_core::html::parse_html`) works on a DOM that has no source positions, so the
//! map is built from the other side: a forgiving scan of the source cuts it into *segments* (the text and
//! media that sit directly in one block-level element) and the segments are paired in reading order with the
//! blocks of the document by their text. A block that cannot be paired simply has no position. Only one
//! linear pass over the source and one over the blocks are needed.

use std::ops::Range;

use bookx_core::doc::{Block, BlockKind, Doc, Span};

use crate::markup::{Event, decode_entities, scan};

/// Prefix of the anchor names the editor puts on the preview's blocks (`bxs12` = block 12).
pub const ANCHOR_PREFIX: &str = "bxs";

/// Anchor name of block `i`.
pub fn anchor_name(i: usize) -> String {
    format!("{ANCHOR_PREFIX}{i}")
}

/// The block number in an anchor name made by [`anchor_name`].
pub fn block_of_anchor(a: &str) -> Option<usize> {
    a.strip_prefix(ANCHOR_PREFIX)?.parse().ok()
}

/// Text and media directly inside one block-level element of the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Where it begins (the start tag of the element when the content follows it directly, otherwise the
    /// first text) and ends.
    pub range: Range<usize>,
    /// The start tag of the innermost block-level element.
    pub tag: Range<usize>,
    /// Decoded text with whitespace collapsed.
    pub text: String,
    /// Holds an image, an `<hr>` or an SVG.
    pub media: bool,
}

const SKIP: &[&str] = &["head", "script", "style", "title", "noscript", "template"];

const BLOCKS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ul",
    "ol",
    "tr",
    "td",
    "th",
    "table",
    "blockquote",
    "section",
    "article",
    "aside",
    "header",
    "footer",
    "nav",
    "pre",
    "dt",
    "dd",
    "dl",
    "figure",
    "figcaption",
    "body",
    "caption",
    "address",
    "main",
];

struct Cur {
    start: usize,
    tag: Range<usize>,
    text: String,
    media: bool,
}

/// Cuts a markup source into [`Segment`]s in reading order.
pub fn segments(src: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut skip: Vec<String> = Vec::new();
    // open block elements: (name, start tag)
    let mut stack: Vec<(String, Range<usize>)> = Vec::new();
    // the content right after a block start tag begins at that tag
    let mut pending: Option<usize> = None;
    let mut cur: Option<Cur> = None;

    fn finish(out: &mut Vec<Segment>, cur: &mut Option<Cur>, end: usize) {
        if let Some(c) = cur.take() {
            let text = c.text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !text.is_empty() || c.media {
                out.push(Segment {
                    range: c.start..end.max(c.start),
                    tag: c.tag,
                    text,
                    media: c.media,
                });
            }
        }
    }

    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if !skip.is_empty() {
                    continue;
                }
                if SKIP.contains(&name.as_str()) {
                    if !self_closing {
                        skip.push(name);
                    }
                    continue;
                }
                if BLOCKS.contains(&name.as_str()) {
                    finish(&mut out, &mut cur, range.start);
                    if !self_closing {
                        stack.push((name, range.clone()));
                    }
                    pending = Some(range.start);
                } else if name == "hr" {
                    finish(&mut out, &mut cur, range.start);
                    out.push(Segment {
                        range: range.clone(),
                        tag: range.clone(),
                        text: String::new(),
                        media: true,
                    });
                    pending = None;
                } else if matches!(name.as_str(), "img" | "image" | "svg") {
                    let tag = stack.last().map(|s| s.1.clone()).unwrap_or(range.clone());
                    let c = cur.get_or_insert_with(|| Cur {
                        start: pending.take().unwrap_or(range.start),
                        tag,
                        text: String::new(),
                        media: false,
                    });
                    c.media = true;
                } else if name == "br" {
                    if let Some(c) = cur.as_mut() {
                        c.text.push(' ');
                    }
                }
            }
            Event::Close { name, range } => {
                if let Some(top) = skip.last() {
                    if *top == name {
                        skip.pop();
                    }
                    continue;
                }
                if BLOCKS.contains(&name.as_str()) {
                    finish(&mut out, &mut cur, range.start);
                    if let Some(i) = stack.iter().rposition(|s| s.0 == name) {
                        stack.truncate(i);
                    }
                    pending = None;
                }
            }
            Event::Text(r) => {
                if !skip.is_empty() {
                    continue;
                }
                let t = decode_entities(&src[r.clone()]);
                if t.trim().is_empty() {
                    if let Some(c) = cur.as_mut() {
                        c.text.push(' ');
                    }
                    continue;
                }
                let tag = stack.last().map(|s| s.1.clone()).unwrap_or(0..0);
                let c = cur.get_or_insert_with(|| Cur {
                    start: pending.take().unwrap_or(r.start),
                    tag,
                    text: String::new(),
                    media: false,
                });
                c.text.push_str(&t);
            }
            Event::Other(_) => {}
        }
    }
    finish(&mut out, &mut cur, src.len());
    out
}

/// What the preview shows of block `b` as normalised text.
fn block_text(b: &Block) -> String {
    if matches!(b.kind, BlockKind::Rule) {
        return String::new();
    }
    b.plain_text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_media(b: &Block) -> bool {
    matches!(b.kind, BlockKind::Rule) || b.spans.iter().any(|s| matches!(s, Span::Image { .. }))
}

/// Where each block of the document comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    /// One entry per block of the document (`None` = could not be paired).
    pub blocks: Vec<Option<Segment>>,
}

const LOOKAHEAD: usize = 24;

/// Pairs the blocks of `doc` with the segments of `src`.
pub fn map_blocks(src: &str, doc: &Doc) -> SourceMap {
    let segs = segments(src);
    let mut next = 0usize;
    let mut blocks = Vec::with_capacity(doc.blocks.len());
    for b in &doc.blocks {
        let text = block_text(b);
        let media = has_media(b);
        let window = &segs[next.min(segs.len())..(next + LOOKAHEAD).min(segs.len())];
        let found = window
            .iter()
            .position(|s| s.text == text && (!text.is_empty() || s.media))
            .or_else(|| {
                // the preview may leave out a marker or an image alt: containment, nearest first
                if text.is_empty() {
                    return None;
                }
                window.iter().take(6).position(|s| {
                    !s.text.is_empty() && (s.text.contains(&text) || text.contains(&s.text))
                })
            })
            .or_else(|| {
                if text.is_empty() && media {
                    window.iter().position(|s| s.media)
                } else {
                    None
                }
            });
        match found {
            Some(k) => {
                blocks.push(Some(window[k].clone()));
                next += k + 1;
            }
            None => blocks.push(None),
        }
    }
    SourceMap { blocks }
}

impl SourceMap {
    /// The block whose source holds byte `offset`: the last mapped block that starts at or before it.
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        let mut best = None;
        for (i, b) in self.blocks.iter().enumerate() {
            let Some(s) = b else { continue };
            if s.range.start <= offset {
                best = Some(i);
            } else {
                break;
            }
        }
        best
    }

    /// The first byte of block `i`.
    pub fn offset_of(&self, i: usize) -> Option<usize> {
        self.blocks.get(i)?.as_ref().map(|s| s.range.start)
    }

    /// The nearest block at or before `i` that has a position.
    pub fn offset_near(&self, i: usize) -> Option<usize> {
        self.blocks[..=i.min(self.blocks.len().saturating_sub(1))]
            .iter()
            .rev()
            .find_map(|b| b.as_ref().map(|s| s.range.start))
    }
}

/// Puts an anchor [`anchor_name`] in front of every block that has a position, so the canvas can scroll to
/// it and report which block is under a point.
pub fn tag_blocks(doc: &mut Doc, map: &SourceMap) {
    for (i, b) in doc.blocks.iter_mut().enumerate() {
        if matches!(map.blocks.get(i), Some(Some(_))) {
            b.spans.insert(0, Span::Anchor(anchor_name(i)));
        }
    }
}

/// Makes sure the element whose start tag is `tag` has an `id`; returns the new source (when it changed) and
/// the id. A new id is `base`, `base-2`, ... not used in the file.
pub fn ensure_id(src: &str, tag: Range<usize>, base: &str) -> Option<(Option<String>, String)> {
    let open = src.get(tag.clone())?;
    if !open.starts_with('<') {
        return None;
    }
    if let Some(id) = crate::markup::tag_attr(open, "id") {
        if !id.is_empty() {
            return Some((None, id));
        }
    }
    let used = crate::markup::defined_ids(src);
    let mut id = base.to_string();
    let mut n = 1;
    while used.contains(&id) {
        n += 1;
        id = format!("{base}-{n}");
    }
    let new_tag = crate::markup::set_tag_attr(open, "id", &id);
    let mut out = String::with_capacity(src.len() + id.len() + 8);
    out.push_str(&src[..tag.start]);
    out.push_str(&new_tag);
    out.push_str(&src[tag.end..]);
    Some((Some(out), id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::html::parse_html;

    const SRC: &str = "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\n<body>\n<h1 id=\"c1\">Chapter &amp; One</h1>\n<p>First <b>bold</b> para.</p>\n<hr/>\n<div><p>Nested one</p>loose text<p>Nested two</p></div>\n<p><img src=\"a.png\" alt=\"\"/></p>\n<p>Last</p>\n</body></html>";

    #[test]
    fn segments_follow_the_blocks_of_the_source() {
        let s = segments(SRC);
        let texts: Vec<&str> = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Chapter & One",
                "First bold para.",
                "",
                "Nested one",
                "loose text",
                "Nested two",
                "",
                "Last"
            ]
        );
        assert!(s[2].media && s[6].media);
        // a segment that starts right after a block tag starts at the tag
        assert!(SRC[s[1].range.clone()].starts_with("<p>First"));
        assert_eq!(&SRC[s[0].tag.clone()], "<h1 id=\"c1\">");
        // loose text starts at the text itself
        assert!(SRC[s[4].range.start..].starts_with("loose text"));
    }

    #[test]
    fn every_block_of_the_preview_finds_its_source() {
        let doc = parse_html(SRC, "a.xhtml");
        let map = map_blocks(SRC, &doc);
        assert_eq!(map.blocks.len(), doc.blocks.len());
        assert!(map.blocks.iter().all(|b| b.is_some()), "{map:?}");
        let first = map.offset_of(0).unwrap();
        assert!(SRC[first..].starts_with("<h1"));
        let last = map.blocks.len() - 1;
        assert!(SRC[map.offset_of(last).unwrap()..].starts_with("<p>Last"));
    }

    #[test]
    fn offsets_map_to_blocks_and_back() {
        let doc = parse_html(SRC, "a.xhtml");
        let map = map_blocks(SRC, &doc);
        let at = SRC.find("para.").unwrap();
        let b = map.block_at(at).unwrap();
        assert_eq!(doc.blocks[b].plain_text(), "First bold para.");
        assert_eq!(map.block_at(0), None);
        let end = map.block_at(SRC.len()).unwrap();
        assert_eq!(doc.blocks[end].plain_text(), "Last");
        assert_eq!(map.offset_near(1).unwrap(), map.offset_of(1).unwrap());
    }

    #[test]
    fn a_block_that_cannot_be_paired_is_skipped_not_misplaced() {
        let mut doc = parse_html(SRC, "a.xhtml");
        doc.blocks.insert(
            1,
            Block {
                kind: BlockKind::Paragraph,
                spans: vec![Span::Text {
                    text: "generated by the reader".into(),
                    style: Default::default(),
                    link: None,
                }],
                align: Default::default(),
                indent: 0,
                marker: None,
                css: Default::default(),
            },
        );
        let map = map_blocks(SRC, &doc);
        assert!(map.blocks[1].is_none());
        assert!(map.blocks[2].is_some());
        assert!(SRC[map.offset_of(2).unwrap()..].starts_with("<p>First"));
    }

    #[test]
    fn tagging_blocks_adds_anchors_the_canvas_can_find() {
        let mut doc = parse_html(SRC, "a.xhtml");
        let map = map_blocks(SRC, &doc);
        tag_blocks(&mut doc, &map);
        let anchors: Vec<usize> = doc
            .blocks
            .iter()
            .filter_map(|b| match b.spans.first() {
                Some(Span::Anchor(a)) => block_of_anchor(a),
                _ => None,
            })
            .collect();
        assert_eq!(anchors, (0..doc.blocks.len()).collect::<Vec<_>>());
        assert_eq!(block_of_anchor("c1"), None);
        assert_eq!(block_of_anchor(&anchor_name(7)), Some(7));
    }

    #[test]
    fn ensure_id_keeps_an_existing_id_and_adds_a_unique_one() {
        let s = segments(SRC);
        let (new, id) = ensure_id(SRC, s[0].tag.clone(), "x").unwrap();
        assert!(new.is_none());
        assert_eq!(id, "c1");
        let (new, id) = ensure_id(SRC, s[1].tag.clone(), "c1").unwrap();
        assert_eq!(id, "c1-2");
        let new = new.unwrap();
        assert!(new.contains("<p id=\"c1-2\">First"), "{new}");
        assert!(roxmltree::Document::parse(&new).is_ok());
    }

    #[test]
    fn mapping_is_fast_on_a_large_chapter() {
        let mut big = String::from("<html><head></head><body>");
        for i in 0..4000 {
            big.push_str(&format!(
                "<p>Paragraph number {i} with <i>some</i> text.</p>\n"
            ));
        }
        big.push_str("</body></html>");
        let doc = parse_html(&big, "a.xhtml");
        let t = std::time::Instant::now();
        let map = map_blocks(&big, &doc);
        let ms = t.elapsed().as_millis();
        assert!(map.blocks.iter().all(|b| b.is_some()));
        let budget = if cfg!(debug_assertions) { 3000 } else { 100 };
        assert!(ms < budget, "{ms} ms");
    }

    #[test]
    fn inline_markup_does_not_add_spaces_and_documents_may_repeat() {
        let one = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\n<body>\n<h1 id=\"t\">Chapter</h1>\n<p>alpha beta <a href=\"x.xhtml#t\">link</a>.</p>\n<p>gamma</p>\n</body></html>\n";
        let src = one.repeat(3);
        let s = segments(&src);
        assert_eq!(s[1].text, "alpha beta link.");
        let doc = parse_html(&src, "a.xhtml");
        let map = map_blocks(&src, &doc);
        assert_eq!(map.blocks.len(), 9);
        assert!(map.blocks.iter().all(|b| b.is_some()), "{map:?}");
        // the third copy maps into the third copy of the source
        assert!(map.offset_of(8).unwrap() > one.len() * 2);
    }
}
