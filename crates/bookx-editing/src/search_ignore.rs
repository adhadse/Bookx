//! "Search ignoring HTML markup" (CE-083): the text of a document without its tags, with a map back to the
//! source, so a phrase such as `on a word` is found even when it is split by `<i>…</i>`.
//!
//! The projection:
//! * tags contribute nothing, except block-level tags (`<p>`, `<div>`, `<br/>` …) which contribute one
//!   line break so a phrase cannot silently run from one paragraph into the next;
//! * character references are decoded (`&amp;` is `&`), so `&` finds `&amp;`;
//! * `<head>`, `<script>`, `<style>`, comments and processing instructions are not part of the text.
//!
//! Replacing a match follows one rule: **the replacement is written where the match starts; the matched text in
//! later text runs is deleted, and every tag inside the match is kept.** Searching for `on a word` in
//! `on <i>a</i> word` and replacing it with `X` gives `X<i></i>`. The replacement is escaped (`&`, `<`, `>`).
//! Line breaks contributed by block tags are never deleted, so a match that spans two paragraphs keeps both.

use crate::markup::{decode_with_offsets, scan, Event, BLOCK_TAGS, DEFAULT_SKIP};

#[derive(Debug, Clone, Copy)]
struct PChar {
    /// Byte offset of the character in the projected text.
    proj: usize,
    /// Source byte range of the character (a whole entity for `&amp;`).
    src_start: usize,
    src_end: usize,
    /// Index of the text run in the source (a tag boundary separates runs).
    run: usize,
    /// A line break contributed by a block tag.
    virt: bool,
}

/// The text of a document and the way back to its source.
#[derive(Debug, Clone)]
pub struct Projection {
    pub text: String,
    chars: Vec<PChar>,
    src_len: usize,
}

/// One source edit: replace `start..end` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// A match in the projected text and what to put there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Builds the projection of `src`.
pub fn project(src: &str) -> Projection {
    let mut text = String::new();
    let mut chars: Vec<PChar> = Vec::new();
    let mut skip: Vec<String> = Vec::new();
    let mut run = 0usize;
    let push_break = |text: &mut String, chars: &mut Vec<PChar>, at: usize, run: usize| {
        if !text.is_empty() && !text.ends_with('\n') {
            chars.push(PChar {
                proj: text.len(),
                src_start: at,
                src_end: at,
                run,
                virt: true,
            });
            text.push('\n');
        }
    };
    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if !self_closing && DEFAULT_SKIP.contains(&name.as_str()) {
                    skip.push(name.clone());
                }
                if skip.is_empty() && BLOCK_TAGS.contains(&name.as_str()) {
                    push_break(&mut text, &mut chars, range.start, run);
                }
                run += 1;
            }
            Event::Close { name, range } => {
                if skip.last() == Some(&name) {
                    skip.pop();
                }
                if skip.is_empty() && BLOCK_TAGS.contains(&name.as_str()) {
                    push_break(&mut text, &mut chars, range.start, run);
                }
                run += 1;
            }
            Event::Other(_) => run += 1,
            Event::Text(r) => {
                if !skip.is_empty() {
                    continue;
                }
                let raw = &src[r.clone()];
                let (dec, map) = decode_with_offsets(raw);
                for (i, ch) in dec.char_indices() {
                    chars.push(PChar {
                        proj: text.len(),
                        src_start: r.start + map[i],
                        src_end: r.start + map[i + ch.len_utf8()],
                        run,
                        virt: false,
                    });
                    text.push(ch);
                }
                run += 1;
            }
        }
    }
    Projection {
        text,
        chars,
        src_len: src.len(),
    }
}

impl Projection {
    /// Index of the first projected character at or after byte `b`.
    fn first_at(&self, b: usize) -> usize {
        self.chars.partition_point(|c| c.proj < b)
    }

    /// The source span covering the projected match `a..b` (tags inside the match are inside the span).
    pub fn source_span(&self, a: usize, b: usize) -> Option<(usize, usize)> {
        let (i, j) = (self.first_at(a), self.first_at(b));
        let real: Vec<&PChar> = self.chars[i..j].iter().filter(|c| !c.virt).collect();
        let first = real.first()?;
        let last = real.last()?;
        Some((first.src_start, last.src_end))
    }

    /// The source edits that replace the projected match `a..b` with `replacement` (see the module docs).
    pub fn edits(&self, a: usize, b: usize, replacement: &str) -> Vec<Edit> {
        let esc = escape_text(replacement);
        let (i, j) = (self.first_at(a), self.first_at(b));
        let real: Vec<&PChar> = self.chars[i..j].iter().filter(|c| !c.virt).collect();
        if real.is_empty() {
            if a != b {
                return Vec::new();
            }
            let at = self
                .chars
                .get(i)
                .map(|c| c.src_start)
                .unwrap_or(self.src_len);
            return vec![Edit {
                start: at,
                end: at,
                text: esc,
            }];
        }
        let mut out: Vec<Edit> = Vec::new();
        let mut cur_run = usize::MAX;
        for c in real {
            if c.run == cur_run {
                if let Some(e) = out.last_mut() {
                    e.end = c.src_end;
                }
            } else {
                cur_run = c.run;
                out.push(Edit {
                    start: c.src_start,
                    end: c.src_end,
                    text: if out.is_empty() {
                        esc.clone()
                    } else {
                        String::new()
                    },
                });
            }
        }
        out
    }
}

/// The source text `s..e` after applying the `edits` that lie inside it.
pub fn apply_edits_range(src: &str, edits: &[Edit], s: usize, e: usize) -> String {
    let mut sorted: Vec<&Edit> = edits
        .iter()
        .filter(|x| x.start >= s && x.end <= e)
        .collect();
    sorted.sort_by_key(|x| (x.start, x.end));
    let mut out = String::new();
    let mut last = s;
    for x in sorted {
        if x.start < last {
            continue;
        }
        out.push_str(&src[last..x.start]);
        out.push_str(&x.text);
        last = x.end;
    }
    out.push_str(&src[last..e]);
    out
}

/// Matches (byte ranges in the projected text) of a closure, e.g. a regex search.
pub fn find_source_spans(
    src: &str,
    find: impl FnOnce(&str) -> Vec<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let p = project(src);
    find(&p.text)
        .into_iter()
        .filter_map(|(a, b)| p.source_span(a, b))
        .collect()
}

/// Applies replacements for `matches` (projected ranges, ascending, non overlapping) and returns the new
/// source and the number of matches applied.
pub fn replace_matches(src: &str, matches: impl FnOnce(&str) -> Vec<TextMatch>) -> (String, usize) {
    let p = project(src);
    let ms = matches(&p.text);
    let mut edits: Vec<Edit> = Vec::new();
    let mut n = 0;
    for m in &ms {
        let e = p.edits(m.start, m.end, &m.replacement);
        if !e.is_empty() {
            n += 1;
            edits.extend(e);
        }
    }
    edits.sort_by_key(|e| (e.start, e.end));
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for e in edits {
        if e.start < last {
            continue;
        }
        out.push_str(&src[last..e.start]);
        out.push_str(&e.text);
        last = e.end;
    }
    out.push_str(&src[last..]);
    (out, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(p: &Projection, needle: &str) -> (usize, usize) {
        let a = p.text.find(needle).unwrap();
        (a, a + needle.len())
    }

    #[test]
    fn text_ignores_inline_tags_and_decodes_entities() {
        let p = project("<p>Tom &amp; <i>Jerry</i> <b>ran</b></p>");
        assert_eq!(p.text, "Tom & Jerry ran\n");
    }

    #[test]
    fn block_tags_become_one_line_break_and_head_is_skipped() {
        let p = project(
            "<html><head><title>T</title></head><body><p>a</p><p>b</p><br/>c</body></html>",
        );
        assert_eq!(p.text, "a\nb\nc\n");
        assert!(!p.text.contains('T'));
    }

    #[test]
    fn phrase_across_inline_tags_is_found_with_its_source_span() {
        let src = "<p>so on <i>a</i> word here</p>";
        let p = project(src);
        let (a, b) = find(&p, "on a word");
        let (s, e) = p.source_span(a, b).unwrap();
        assert_eq!(&src[s..e], "on <i>a</i> word");
    }

    #[test]
    fn replacement_is_written_at_the_start_and_inner_tags_stay() {
        let src = "<p>so on <i>a</i> word here</p>";
        let (out, n) = replace_matches(src, |t| {
            let a = t.find("on a word").unwrap();
            vec![TextMatch {
                start: a,
                end: a + 9,
                replacement: "X".into(),
            }]
        });
        assert_eq!(n, 1);
        assert_eq!(out, "<p>so X<i></i> here</p>");
    }

    #[test]
    fn replacement_text_is_escaped() {
        let (out, _) = replace_matches("<p>a <b>b</b></p>", |t| {
            let a = t.find("a b").unwrap();
            vec![TextMatch {
                start: a,
                end: a + 3,
                replacement: "1 < 2 & 3".into(),
            }]
        });
        assert_eq!(out, "<p>1 &lt; 2 &amp; 3<b></b></p>");
    }

    #[test]
    fn entities_are_replaced_as_a_whole() {
        let (out, _) = replace_matches("<p>Tom &amp; Jerry</p>", |t| {
            let a = t.find("& J").unwrap();
            vec![TextMatch {
                start: a,
                end: a + 3,
                replacement: "and J".into(),
            }]
        });
        assert_eq!(out, "<p>Tom and Jerry</p>");
    }

    #[test]
    fn unicode_text_maps_to_byte_offsets() {
        let src = "<p>é<i>à</i>ü</p>";
        let p = project(src);
        let (a, b) = find(&p, "éàü");
        let (s, e) = p.source_span(a, b).unwrap();
        assert_eq!(&src[s..e], "é<i>à</i>ü");
    }

    #[test]
    fn matches_spanning_paragraphs_keep_both_paragraphs() {
        let src = "<p>one</p><p>two</p>";
        let (out, n) = replace_matches(src, |t| {
            let a = t.find("one\ntwo").unwrap();
            vec![TextMatch {
                start: a,
                end: a + 7,
                replacement: "X".into(),
            }]
        });
        assert_eq!(n, 1);
        assert_eq!(out, "<p>X</p><p></p>");
    }

    #[test]
    fn several_matches_in_one_pass() {
        let src = "<p>a<b>b</b> a<i>b</i></p>";
        let (out, n) = replace_matches(src, |t| {
            t.match_indices("ab")
                .map(|(i, _)| TextMatch {
                    start: i,
                    end: i + 2,
                    replacement: "Z".into(),
                })
                .collect()
        });
        assert_eq!(n, 2);
        assert_eq!(out, "<p>Z<b></b> Z<i></i></p>");
    }

    #[test]
    fn empty_match_inserts_and_virtual_only_match_is_ignored() {
        let (out, n) = replace_matches("<p>ab</p>", |_| {
            vec![TextMatch {
                start: 1,
                end: 1,
                replacement: "-".into(),
            }]
        });
        assert_eq!((out.as_str(), n), ("<p>a-b</p>", 1));
        let p = project("<p>a</p><p>b</p>");
        assert!(p.edits(1, 2, "x").is_empty());
    }

    #[test]
    fn apply_edits_range_cuts_the_span() {
        let src = "<p>so on <i>a</i> word here</p>";
        let p = project(src);
        let a = p.text.find("on a word").unwrap();
        let (s, e) = p.source_span(a, a + 9).unwrap();
        let edits = p.edits(a, a + 9, "X");
        assert_eq!(apply_edits_range(src, &edits, s, e), "X<i></i>");
    }

    #[test]
    fn find_source_spans_maps_every_hit() {
        let src = "<p>x <b>y</b> x<i>y</i></p>";
        let spans = find_source_spans(src, |t| {
            t.match_indices("xy").map(|(i, _)| (i, i + 2)).collect()
        });
        assert_eq!(spans.len(), 1);
        assert_eq!(&src[spans[0].0..spans[0].1], "x<i>y");
    }
}
