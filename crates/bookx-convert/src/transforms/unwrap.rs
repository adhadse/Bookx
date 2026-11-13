//! Unwrap hard-wrapped lines in books derived from plain text.

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::map_spine_files;
use crate::markup::plain_text;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UnwrapLines {
    /// A line counts as "wrapped" when it is at least `threshold` times as long as the typical
    /// (90th percentile) line of the document.
    pub threshold: f64,
    pub jobs: usize,
}

impl Default for UnwrapLines {
    fn default() -> Self {
        UnwrapLines {
            threshold: 0.7,
            jobs: 0,
        }
    }
}

/// Characters that end a sentence (also when followed by a closing quote).
fn ends_sentence(text: &str) -> bool {
    let t = text.trim_end();
    let t = t.trim_end_matches(['"', '\'', '”', '’', ')', ']', '»', '*']);
    matches!(
        t.chars().last(),
        Some('.' | '!' | '?' | ':' | ';' | '…' | '—')
    )
}

fn percentile90(lens: &[usize]) -> usize {
    if lens.is_empty() {
        return 0;
    }
    let mut v = lens.to_vec();
    v.sort_unstable();
    v[((v.len() - 1) as f64 * 0.9).round() as usize]
}

struct Para {
    start: usize,
    end: usize,
    open: String,
    inner: String,
}

fn paragraphs(html: &str) -> Vec<Para> {
    let re = Regex::new(r"(?s)(<p(?:\s[^>]*)?>)(.*?)</p>").expect("static regex");
    re.captures_iter(html)
        .map(|c| {
            let m = c.get(0).expect("whole match");
            Para {
                start: m.start(),
                end: m.end(),
                open: c[1].to_string(),
                inner: c[2].to_string(),
            }
        })
        .collect()
}

fn text_len(inner: &str) -> usize {
    plain_text(inner)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .count()
}

impl UnwrapLines {
    pub fn unwrap_html(&self, html: &str) -> String {
        let paras = paragraphs(html);
        if paras.len() < 2 && !html.contains("<br") {
            return html.to_string();
        }
        let br = Regex::new(r"(?i)\s*<br\s*/?>\s*").expect("static regex");
        let lens: Vec<usize> = paras
            .iter()
            .flat_map(|p| br.split(&p.inner).map(text_len).collect::<Vec<_>>())
            .collect();
        let typical = percentile90(&lens);
        if typical == 0 {
            return html.to_string();
        }
        let min_len = ((typical as f64) * self.threshold).ceil() as usize;
        let long = |inner: &str| text_len(inner) >= min_len;
        let wrapped = |inner: &str| long(inner) && !ends_sentence(&plain_text(inner));
        // a short line without punctuation is a heading or a standalone line: never joined
        let standalone = |inner: &str| !long(inner) && !ends_sentence(&plain_text(inner));

        // 1. `<br/>` inside a paragraph between two wrapped lines
        let mut out = String::with_capacity(html.len());
        let mut last = 0;
        let mut i = 0;
        while i < paras.len() {
            let p = &paras[i];
            out.push_str(&html[last..p.start]);
            let mut inner = join_breaks(&br, &p.inner, &long);
            let mut j = i;
            let mut tail = last_line(&br, &inner);
            // 2. consecutive paragraphs
            while j + 1 < paras.len() {
                let between = &html[paras[j].end..paras[j + 1].start];
                if !between.trim().is_empty() {
                    break;
                }
                let next = &paras[j + 1];
                let next_inner = join_breaks(&br, &next.inner, &long);
                if wrapped(&tail) && !next_inner.trim().is_empty() && !standalone(&next_inner) {
                    inner = format!("{} {}", inner.trim_end(), next_inner.trim_start());
                    tail = last_line(&br, &next_inner);
                    j += 1;
                } else {
                    break;
                }
            }
            out.push_str(&p.open);
            out.push_str(&inner);
            out.push_str("</p>");
            last = paras[j].end;
            i = j + 1;
        }
        out.push_str(&html[last..]);
        out
    }
}

fn last_line(br: &Regex, inner: &str) -> String {
    br.split(inner).last().unwrap_or("").to_string()
}

fn join_breaks(br: &Regex, inner: &str, long: &dyn Fn(&str) -> bool) -> String {
    if !br.is_match(inner) {
        return inner.to_string();
    }
    let parts: Vec<&str> = br.split(inner).collect();
    let seps: Vec<&str> = br.find_iter(inner).map(|m| m.as_str()).collect();
    let mut out = String::new();
    for (k, part) in parts.iter().enumerate() {
        out.push_str(part);
        if k < seps.len() {
            let next_ok = parts.get(k + 1).is_some_and(|n| !n.trim().is_empty());
            if long(part) && !ends_sentence(&plain_text(part)) && next_ok {
                out.push(' ');
            } else {
                out.push_str(seps[k]);
            }
        }
    }
    out
}

impl Transform for UnwrapLines {
    fn id(&self) -> &'static str {
        "unwrap_lines"
    }
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let n = map_spine_files(book, self.jobs, |_, t| Some(self.unwrap_html(t)))?;
        Ok(TransformReport {
            files_changed: n,
            notes: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;

    fn wrapped_text() -> String {
        let lines = [
            "It was a bright cold day in April, and the clocks were",
            "striking thirteen. Winston Smith, his chin nuzzled into",
            "his breast in an effort to escape the vile wind, slipped",
            "quickly through the glass doors of Victory Mansions.",
            "",
            "The hallway smelt of boiled cabbage and old rag mats. At",
            "one end of it a coloured poster, too large for indoor",
            "display, had been tacked to the wall.",
        ];
        let mut s = String::new();
        for l in lines {
            if !l.is_empty() {
                s.push_str(&format!("<p>{l}</p>\n"));
            }
        }
        s
    }

    #[test]
    fn joins_wrapped_paragraph_lines() {
        let out = UnwrapLines::default().unwrap_html(&wrapped_text());
        let n = out.matches("<p>").count();
        assert_eq!(n, 2, "{out}");
        assert!(out.contains(
            "clocks were striking thirteen. Winston Smith, his chin nuzzled into his breast"
        ));
        assert!(out.contains("of Victory Mansions.</p>"));
    }

    #[test]
    fn keeps_paragraphs_ending_in_sentence_punctuation() {
        let html = "<p>This line is long enough to be considered a full line okay.</p>\n<p>Another long line that also ends with a period right here.</p>\n<p>And one more line that finishes with an exclamation mark!</p>";
        assert_eq!(UnwrapLines::default().unwrap_html(html), html);
    }

    #[test]
    fn keeps_short_headings() {
        let html = "<p>A long hard wrapped line that does not end with punctuation at</p>\n<p>CHAPTER TWO</p>\n<p>the beginning of something that is long enough to count ok.</p>";
        let out = UnwrapLines::default().unwrap_html(html);
        assert!(out.contains("<p>CHAPTER TWO</p>"), "{out}");
    }

    #[test]
    fn closing_quote_after_period_is_still_a_sentence_end() {
        assert!(ends_sentence("He said \"stop.\""));
        assert!(ends_sentence("What?)"));
        assert!(!ends_sentence("and then the"));
        assert!(!ends_sentence(""));
    }

    #[test]
    fn joins_br_separated_lines() {
        let html = "<p>The first long line of a hard wrapped paragraph here<br/>continues on the second long line of the paragraph.<br/>Short.</p><p>Another paragraph that is reasonably long and complete.</p>";
        let out = UnwrapLines::default().unwrap_html(html);
        assert!(out.contains("paragraph here continues on"), "{out}");
        assert!(out.contains("paragraph.<br/>Short."), "{out}");
    }

    #[test]
    fn preserves_inline_markup_and_attributes() {
        let html = "<p class=\"x\">A long line with <i>italic words</i> that keeps going and going</p>\n<p>and finishes here in a sentence that is also long enough.</p>";
        let out = UnwrapLines::default().unwrap_html(html);
        assert!(out.starts_with("<p class=\"x\">A long line with <i>italic words</i>"));
        assert_eq!(out.matches("<p").count(), 1);
    }

    #[test]
    fn no_paragraphs_is_noop() {
        assert_eq!(
            UnwrapLines::default().unwrap_html("<div>x</div>"),
            "<div>x</div>"
        );
        assert_eq!(UnwrapLines::default().unwrap_html(""), "");
    }

    #[test]
    fn idempotent() {
        let once = UnwrapLines::default().unwrap_html(&wrapped_text());
        assert_eq!(UnwrapLines::default().unwrap_html(&once), once);
        let mixed = "<p>A long line that does not end and keeps on going to the</p>\n<p>end of the line okay</p>\n<p>Next paragraph begins with a very long line that does not end</p>\n<p>CHAPTER</p>\n<p>after the heading we continue with a long line that ends here.</p>";
        let a = UnwrapLines::default().unwrap_html(mixed);
        assert_eq!(UnwrapLines::default().unwrap_html(&a), a);
    }

    #[test]
    fn transform_applies_to_every_file() {
        let mut b = book_with(&[("A", &wrapped_text()), ("B", "<p>ok</p>")]);
        let t = UnwrapLines::default();
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 1);
        let snap = snapshot(&b);
        assert_eq!(t.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(snapshot(&b), snap);
        assert_eq!(t.id(), "unwrap_lines");
    }

    #[test]
    fn threshold_controls_joining() {
        let html = "<p>medium length line without a stop</p>\n<p>another medium length line without a stop</p>\n<p>the very very very very very very long reference line that sets the typical width.</p>";
        let strict = UnwrapLines {
            threshold: 0.95,
            jobs: 1,
        }
        .unwrap_html(html);
        assert_eq!(strict, html);
        let loose = UnwrapLines {
            threshold: 0.2,
            jobs: 1,
        }
        .unwrap_html(html);
        assert!(loose.matches("<p>").count() < 3);
    }
}
