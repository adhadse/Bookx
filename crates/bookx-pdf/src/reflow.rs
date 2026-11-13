//! Reflow: the text of a PDF in reading order as paragraphs and chapters.
//!
//! Best effort and labelled experimental in the app. The heuristics: running headers, footers and
//! page numbers (the same line in the same place on many pages) are dropped; lines are joined to
//! paragraphs by the gaps, indents and short last lines; a word hyphenated at the end of a line
//! is joined; a paragraph that goes on over a column or page break is merged; lines in a
//! clearly larger font are headings. Images, tables and formulas are not reconstructed.

use std::collections::HashMap;

use crate::doc::OutlineItem;
use crate::text::{PageText, TextLine};

/// One chapter of the reflowed text, `html` is the body (XHTML, escaped).
#[derive(Debug, Clone, PartialEq)]
pub struct ReChapter {
    pub title: String,
    pub html: String,
    /// First page (zero based) the chapter covers.
    pub first_page: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Para,
    Heading(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: Kind,
    pub text: String,
    pub page: usize,
}

/// Pages per chapter when the PDF has neither an outline nor headings to split at.
pub const PAGES_PER_CHAPTER: usize = 20;

/// True when the text of the document is too thin to reflow (a scan without a text layer).
pub fn is_scanned(pages: &[PageText]) -> bool {
    let chars: usize = pages.iter().map(|p| p.chars.len()).sum();
    pages.is_empty() || chars < 20 * pages.len().clamp(1, 5)
}

fn norm_key(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn line_text(p: &PageText, l: &TextLine) -> String {
    p.text_for(l.start..l.end)
}

/// Indexes of the lines that are running headers, footers or page numbers, per page.
pub fn furniture(pages: &[PageText]) -> Vec<Vec<bool>> {
    let mut count: HashMap<String, usize> = HashMap::new();
    let zone = |p: &PageText, l: &TextLine| -> bool {
        let c = (l.rect[1] + l.rect[3]) / 2.0;
        c < p.height * 0.09 || c > p.height * 0.91
    };
    for p in pages {
        let mut seen = std::collections::HashSet::new();
        for l in &p.lines {
            if zone(p, l) {
                let k = norm_key(&line_text(p, l));
                if !k.is_empty() && seen.insert(k.clone()) {
                    *count.entry(k).or_default() += 1;
                }
            }
        }
    }
    let need = (pages.len() * 3 / 10).max(3);
    pages
        .iter()
        .map(|p| {
            p.lines
                .iter()
                .map(|l| {
                    if !zone(p, l) {
                        return false;
                    }
                    let t = line_text(p, l);
                    let k = norm_key(&t);
                    let bare_number =
                        k.chars().all(|c| c == '#' || c == ' ' || c == '-') && k.contains('#');
                    if bare_number && t.chars().count() <= 8 {
                        return pages.len() > 1;
                    }
                    count.get(&k).is_some_and(|c| *c >= need)
                })
                .collect()
        })
        .collect()
}

fn body_size(pages: &[PageText]) -> f32 {
    let mut by_size: Vec<(f32, usize)> = Vec::new();
    for p in pages {
        for l in &p.lines {
            by_size.push((l.size, l.end - l.start));
        }
    }
    if by_size.is_empty() {
        return 12.0;
    }
    by_size.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: usize = by_size.iter().map(|x| x.1).sum();
    let mut acc = 0;
    for (s, n) in &by_size {
        acc += n;
        if acc * 2 >= total {
            return *s;
        }
    }
    12.0
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

const END_PUNCT: &[char] = &[
    '.', '!', '?', ':', ';', '”', '"', '\'', ')', '…', '’', '»', '。',
];

fn ends_sentence(s: &str) -> bool {
    s.trim_end()
        .chars()
        .last()
        .is_some_and(|c| END_PUNCT.contains(&c))
}

/// Appends `next` to `acc`: joins a hyphenated word, otherwise adds a space.
pub fn join_lines(acc: &mut String, next: &str) {
    let next = next.trim();
    if next.is_empty() {
        return;
    }
    if acc.is_empty() {
        acc.push_str(next);
        return;
    }
    let mut chars = acc.chars().rev();
    let last = chars.next();
    let before = chars.next();
    let hyphen = matches!(last, Some('-' | '\u{ad}'))
        && before.is_some_and(|c| c.is_alphabetic())
        && next.chars().next().is_some_and(|c| c.is_lowercase());
    if hyphen {
        acc.pop();
        acc.push_str(next);
    } else {
        acc.push(' ');
        acc.push_str(next);
    }
}

/// The paragraphs and headings of the document in order.
pub fn blocks(pages: &[PageText]) -> Vec<Block> {
    let skip = furniture(pages);
    let body = body_size(pages);
    let mut out: Vec<Block> = Vec::new();
    for (pi, p) in pages.iter().enumerate() {
        let lines: Vec<(usize, &TextLine)> = p
            .lines
            .iter()
            .enumerate()
            .filter(|(i, _)| !skip[pi][*i])
            .collect();
        if lines.is_empty() {
            continue;
        }
        // leading: the usual distance between consecutive baselines of a block
        let pitches: Vec<f32> = lines
            .windows(2)
            .filter(|w| w[0].1.block == w[1].1.block)
            .map(|w| w[1].1.baseline - w[0].1.baseline)
            .filter(|d| *d > 0.0 && *d < 3.0 * body)
            .collect();
        let pitch = median(pitches).max(body * 1.1);
        // per block: left edge and right edge of the text
        let mut left: HashMap<usize, f32> = HashMap::new();
        let mut right: HashMap<usize, f32> = HashMap::new();
        for (_, l) in &lines {
            let e = left.entry(l.block).or_insert(l.rect[0]);
            *e = e.min(l.rect[0]);
            let e = right.entry(l.block).or_insert(l.rect[2]);
            *e = e.max(l.rect[2]);
        }
        let mut cur: Option<(String, f32, &TextLine, usize)> = None; // text, size, last line, lines
        let flush = |cur: &mut Option<(String, f32, &TextLine, usize)>, out: &mut Vec<Block>| {
            if let Some((text, size, _, n)) = cur.take() {
                let t = text.trim().to_string();
                if t.is_empty() {
                    return;
                }
                let kind = if n <= 2 && size >= body * 1.2 && t.chars().count() <= 140 {
                    Kind::Heading(if size >= body * 1.7 {
                        1
                    } else if size >= body * 1.4 {
                        2
                    } else {
                        3
                    })
                } else {
                    Kind::Para
                };
                out.push(Block {
                    kind,
                    text: t,
                    page: pi,
                });
            }
        };
        for (_, l) in lines {
            let t = line_text(p, l);
            let Some((text, size, last, n)) = cur.as_mut() else {
                cur = Some((t.trim().to_string(), l.size, l, 1));
                continue;
            };
            let same_block = last.block == l.block;
            let gap = l.baseline - last.baseline;
            let lb = left.get(&l.block).copied().unwrap_or(l.rect[0]);
            let rb = right.get(&l.block).copied().unwrap_or(l.rect[2]);
            let indent = l.rect[0] - lb > 0.8 * l.size && last.rect[0] - lb < 0.5 * l.size;
            let short_last = last.rect[2] < rb - 2.5 * last.size && ends_sentence(text);
            let size_change = (l.size - *size).abs() > 0.15 * size.max(l.size);
            let new_para = !same_block
                || gap > pitch * 1.35
                || gap < 0.0
                || indent
                || short_last
                || size_change;
            if new_para {
                flush(&mut cur, &mut out);
                cur = Some((t.trim().to_string(), l.size, l, 1));
            } else {
                join_lines(text, &t);
                *last = l;
                *n += 1;
            }
        }
        flush(&mut cur, &mut out);
    }
    merge_continuations(out)
}

/// Joins a paragraph that goes on after a column or page break.
fn merge_continuations(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for b in blocks {
        if let Some(prev) = out.last_mut() {
            let continues = prev.kind == Kind::Para
                && b.kind == Kind::Para
                && !ends_sentence(&prev.text)
                && b.text.chars().next().is_some_and(|c| c.is_lowercase());
            if continues {
                let text = std::mem::take(&mut prev.text);
                let mut t = text;
                join_lines(&mut t, &b.text);
                prev.text = t;
                continue;
            }
        }
        out.push(b);
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Where each chapter starts: `(first page, title)`.
fn chapter_starts(
    pages: usize,
    outline: &[OutlineItem],
    blocks: &[Block],
    title: &str,
) -> Vec<(usize, String)> {
    let top = outline
        .iter()
        .filter(|o| o.dest.is_some())
        .map(|o| o.level)
        .min();
    if let Some(top) = top {
        let mut starts: Vec<(usize, String)> = Vec::new();
        for o in outline.iter().filter(|o| o.level == top) {
            let Some(d) = o.dest else { continue };
            if d.page >= pages {
                continue;
            }
            if starts.last().is_some_and(|(p, _)| *p == d.page) {
                continue;
            }
            if starts.last().is_some_and(|(p, _)| *p > d.page) {
                continue;
            }
            let t = if o.title.is_empty() {
                format!("Page {}", d.page + 1)
            } else {
                o.title.clone()
            };
            starts.push((d.page, t));
        }
        if starts.len() >= 2 {
            if starts[0].0 > 0 {
                starts.insert(0, (0, title.to_string()));
            }
            return starts;
        }
    }
    let h1: Vec<(usize, String)> = blocks
        .iter()
        .filter(|b| b.kind == Kind::Heading(1))
        .map(|b| (b.page, b.text.clone()))
        .collect();
    if h1.len() >= 2 && h1.len() <= pages.max(2) {
        let mut v = h1;
        v.dedup_by_key(|x| x.0);
        if v[0].0 > 0 {
            v.insert(0, (0, title.to_string()));
        }
        if v.len() >= 2 {
            return v;
        }
    }
    if pages <= PAGES_PER_CHAPTER + 5 {
        return vec![(0, title.to_string())];
    }
    (0..pages)
        .step_by(PAGES_PER_CHAPTER)
        .map(|s| {
            let e = (s + PAGES_PER_CHAPTER).min(pages);
            (s, format!("Pages {}–{}", s + 1, e))
        })
        .collect()
}

/// Reflows the text of `pages` into chapters. `title` names a chapter without a better title.
pub fn reflow(pages: &[PageText], outline: &[OutlineItem], title: &str) -> Vec<ReChapter> {
    let bl = blocks(pages);
    let starts = chapter_starts(pages.len(), outline, &bl, title);
    let mut chapters: Vec<ReChapter> = Vec::new();
    for (i, (first, name)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map(|s| s.0).unwrap_or(usize::MAX);
        let mut html = String::new();
        let mut has_heading = false;
        let mut chars = 0;
        for b in bl.iter().filter(|b| b.page >= *first && b.page < end) {
            match b.kind {
                Kind::Heading(l) => {
                    // the first heading of a chapter is its title
                    if !has_heading && norm_key(&b.text) == norm_key(name) {
                        has_heading = true;
                    }
                    html.push_str(&format!("<h{l}>{}</h{l}>\n", esc(&b.text)));
                }
                Kind::Para => html.push_str(&format!("<p>{}</p>\n", esc(&b.text))),
            }
            chars += b.text.len();
        }
        if chars == 0 {
            continue;
        }
        if !has_heading {
            html = format!("<h2>{}</h2>\n{html}", esc(name));
        }
        chapters.push(ReChapter {
            title: name.clone(),
            html,
            first_page: *first,
        });
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Dest;
    use crate::text::RawGlyph;

    fn line(x: f32, y: f32, size: f32, text: &str) -> Vec<RawGlyph> {
        let w = size * 0.5;
        text.chars()
            .enumerate()
            .map(|(i, c)| RawGlyph {
                text: c.to_string(),
                x: x + w * i as f32,
                y,
                width: w,
                size,
            })
            .collect()
    }

    fn page(lines: Vec<Vec<RawGlyph>>) -> PageText {
        PageText::build(612.0, 792.0, lines.into_iter().flatten().collect())
    }

    /// Body lines 12 pt, 14 pt apart from y=200.
    fn body(lines: &[&str]) -> Vec<Vec<RawGlyph>> {
        lines
            .iter()
            .enumerate()
            .map(|(i, t)| line(72.0, 200.0 + 14.0 * i as f32, 12.0, t))
            .collect()
    }

    #[test]
    fn lines_join_to_a_paragraph_and_hyphens_are_healed() {
        let p = page(body(&[
            "The ocean was ali-",
            "ve with thoughts that",
            "moved slowly.",
        ]));
        let b = blocks(&[p]);
        assert_eq!(b.len(), 1);
        assert_eq!(
            b[0].text,
            "The ocean was alive with thoughts that moved slowly."
        );
    }

    #[test]
    fn a_real_hyphen_before_a_capital_is_kept() {
        let mut acc = "Jean-".to_string();
        join_lines(&mut acc, "Paul came");
        assert_eq!(acc, "Jean- Paul came");
        let mut acc = "well-".to_string();
        join_lines(&mut acc, "known");
        assert_eq!(
            acc, "wellknown",
            "known limit: a lower case follow-up is joined"
        );
    }

    #[test]
    fn a_gap_between_lines_starts_a_paragraph() {
        let mut lines = body(&["First paragraph line one", "line two ends here."]);
        lines.push(line(
            72.0,
            200.0 + 14.0 * 2.0 + 20.0,
            12.0,
            "Second paragraph.",
        ));
        let b = blocks(&[page(lines)]);
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].text, "Second paragraph.");
    }

    #[test]
    fn first_line_indent_starts_a_paragraph() {
        let lines = vec![
            line(
                72.0,
                200.0,
                12.0,
                "A paragraph that fills the whole width of the page here",
            ),
            line(72.0, 214.0, 12.0, "and goes on until it ends here."),
            line(90.0, 228.0, 12.0, "An indented new paragraph begins"),
            line(72.0, 242.0, 12.0, "and continues on the next line."),
        ];
        let b = blocks(&[page(lines)]);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!(b[0].text.ends_with("ends here."));
        assert!(b[1].text.starts_with("An indented"));
    }

    #[test]
    fn a_short_last_line_ending_a_sentence_ends_the_paragraph() {
        let lines = vec![
            line(
                72.0,
                200.0,
                12.0,
                "A long line that goes right across the page to the margin",
            ),
            line(72.0, 214.0, 12.0, "short end."),
            line(
                72.0,
                228.0,
                12.0,
                "Next paragraph starts on the next line of the page text",
            ),
        ];
        let b = blocks(&[page(lines)]);
        assert_eq!(b.len(), 2, "{b:?}");
    }

    #[test]
    fn larger_type_is_a_heading() {
        let mut lines = vec![line(72.0, 100.0, 24.0, "Chapter One")];
        lines.extend(body(&[
            "Some body text that is long enough to count as the body",
            "text of the book for the median size.",
        ]));
        let b = blocks(&[page(lines)]);
        assert_eq!(b[0].kind, Kind::Heading(1));
        assert_eq!(b[1].kind, Kind::Para);
    }

    #[test]
    fn repeated_headers_footers_and_page_numbers_are_removed() {
        let mut pages = Vec::new();
        for i in 0..6 {
            let mut lines = vec![
                line(72.0, 40.0, 9.0, "THE BOOK TITLE"),
                line(300.0, 760.0, 9.0, &format!("{}", i + 1)),
            ];
            lines.extend(body(&[&format!(
                "Unique content of page number {i} goes here."
            )]));
            pages.push(page(lines));
        }
        let b = blocks(&pages);
        let all: String = b
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("|");
        assert!(!all.contains("BOOK TITLE"), "{all}");
        assert!(!all.contains("| 3 |"), "{all}");
        assert_eq!(b.len(), 6, "{all}");
    }

    #[test]
    fn a_paragraph_continues_over_a_page_break() {
        let p1 = page(body(&["A sentence that does not end on this page and"]));
        let p2 = page(body(&["goes on at the top of the next page."]));
        let b = blocks(&[p1, p2]);
        assert_eq!(b.len(), 1);
        assert_eq!(
            b[0].text,
            "A sentence that does not end on this page and goes on at the top of the next page."
        );
    }

    #[test]
    fn two_columns_reflow_in_reading_order() {
        let lines = vec![
            line(72.0, 200.0, 12.0, "left column first paragraph"),
            line(350.0, 200.0, 12.0, "right column paragraph"),
            line(72.0, 214.0, 12.0, "continues on the left."),
            line(350.0, 214.0, 12.0, "continues on the right."),
        ];
        let b = blocks(&[page(lines)]);
        let t: Vec<&str> = b.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(
            t,
            [
                "left column first paragraph continues on the left.",
                "right column paragraph continues on the right."
            ]
        );
    }

    #[test]
    fn scanned_documents_are_detected() {
        assert!(is_scanned(&[PageText::default(), PageText::default()]));
        assert!(!is_scanned(&[page(body(&[
            "Plenty of real text on this page to count"
        ]))]));
        assert!(is_scanned(&[]));
    }

    #[test]
    fn chapters_follow_the_outline() {
        let pages: Vec<PageText> = (0..4)
            .map(|i| page(body(&[&format!("Text of page {i} is here.")])))
            .collect();
        let outline = vec![
            OutlineItem {
                title: "One".into(),
                level: 0,
                dest: Some(Dest { page: 0, y: None }),
            },
            OutlineItem {
                title: "Sub".into(),
                level: 1,
                dest: Some(Dest { page: 1, y: None }),
            },
            OutlineItem {
                title: "Two".into(),
                level: 0,
                dest: Some(Dest { page: 2, y: None }),
            },
        ];
        let ch = reflow(&pages, &outline, "Book");
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].title, "One");
        assert_eq!(ch[1].first_page, 2);
        assert!(ch[0].html.starts_with("<h2>One</h2>"));
        assert!(ch[0].html.contains("Text of page 1 is here."));
        assert!(!ch[0].html.contains("page 2"));
    }

    #[test]
    fn without_an_outline_a_short_book_is_one_chapter_and_a_long_one_is_chunked() {
        let mk = |n: usize| -> Vec<PageText> {
            (0..n)
                .map(|i| page(body(&[&format!("Text {i} here.")])))
                .collect()
        };
        assert_eq!(reflow(&mk(3), &[], "Book").len(), 1);
        let long = reflow(&mk(45), &[], "Book");
        assert_eq!(long.len(), 3);
        assert_eq!(long[1].title, "Pages 21–40");
    }

    #[test]
    fn text_is_escaped() {
        let ch = reflow(&[page(body(&["a < b & c > d"]))], &[], "T<>");
        assert!(ch[0].html.contains("a &lt; b &amp; c &gt; d"));
        assert!(ch[0].html.contains("T&lt;&gt;"));
    }
}
