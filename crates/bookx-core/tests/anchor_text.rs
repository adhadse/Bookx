//! Highlights are anchored by text in the reader's numbering of a chapter (`Doc::reader_text`).
//! Generated chapters with every kind of inline markup: the text of every paragraph is found
//! exactly where the reader numbers it, and offsets counted the other way (`Doc::to_text`, the
//! owner's bug) are caught and repaired.

use bookx_core::anchor::{self, Chapters, Match, Repair};
use bookx_core::html::parse_html;
use bookx_core::library::Highlight;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const WORDS: &[&str] = &[
    "independent",
    "deployability",
    "service",
    "naïve",
    "café",
    "“quoted”",
    "it’s",
    "well-known",
    "状態",
    "microservices,",
    "idea",
    "release.",
    "Straße",
    "x²",
];

/// `(html, visible text)` of one inline run.
fn inline(r: &mut Rng) -> (String, String) {
    let w = WORDS[r.below(WORDS.len())];
    match r.below(12) {
        0 => (format!("<em>{w}</em>"), w.to_string()),
        1 => (format!("<strong>{w}</strong>"), w.to_string()),
        2 => (
            format!("<a href=\"#n{}\">{w}</a>", r.below(9)),
            w.to_string(),
        ),
        3 => (format!("<span class=\"c\">{w}</span>"), w.to_string()),
        4 => (
            format!("<em>{w} <strong>{w}</strong></em>"),
            format!("{w} {w}"),
        ),
        5 => {
            let d = r.below(9);
            (format!("{w}<sup>{d}</sup>"), format!("{w}{d}"))
        }
        6 => (format!("<code>{w}</code>"), w.to_string()),
        7 => (format!("{w}\u{ad}{w}"), format!("{w}{w}")),
        8 => (
            "Fish &amp; chips &lt;3".to_string(),
            "Fish & chips <3".to_string(),
        ),
        _ => (w.to_string(), w.to_string()),
    }
}

/// One generated paragraph: its HTML and the text a reader sees.
fn paragraph(r: &mut Rng) -> (String, String) {
    let n = 3 + r.below(10);
    let (mut html, mut text) = (String::new(), String::new());
    for i in 0..n {
        let (h, t) = inline(r);
        if i > 0 {
            html.push(' ');
            text.push(' ');
        }
        html.push_str(&h);
        text.push_str(&t);
    }
    (html, text)
}

/// A chapter with `n` blocks of every kind; returns the HTML and the expected text of each block.
fn chapter(seed: u64, n: usize) -> (String, Vec<String>) {
    let mut r = Rng(seed | 1);
    let mut html = String::from("<html><body>");
    let mut texts = Vec::new();
    let push = |h: String, t: String, texts: &mut Vec<String>, html: &mut String| {
        html.push_str(&h);
        texts.push(t);
    };
    for _ in 0..n {
        let (h, t) = paragraph(&mut r);
        match r.below(8) {
            0 => push(format!("<h2>{h}</h2>"), t, &mut texts, &mut html),
            1 => push(
                format!("<blockquote><p>{h}</p></blockquote>"),
                t,
                &mut texts,
                &mut html,
            ),
            2 => {
                // a list item: its marker belongs to the item
                let (h2, t2) = paragraph(&mut r);
                html.push_str(&format!("<ul><li>{h}</li><li>{h2}</li></ul>"));
                texts.push(format!("•\t{t}"));
                texts.push(format!("•\t{t2}"));
            }
            3 => push(format!("<div>{h}</div>"), t, &mut texts, &mut html),
            4 => push(
                format!("<table><tr><td>{h}</td></tr></table>"),
                t,
                &mut texts,
                &mut html,
            ),
            5 => {
                // a nested list
                let (h2, t2) = paragraph(&mut r);
                html.push_str(&format!("<ul><li>{h}<ul><li>{h2}</li></ul></li></ul>"));
                texts.push(format!("•\t{t}"));
                texts.push(format!("•\t{t2}"));
            }
            _ => push(format!("<p>{h}</p>"), t, &mut texts, &mut html),
        }
    }
    html.push_str("</body></html>");
    (html, texts)
}

struct One(Vec<char>);
impl Chapters for One {
    fn count(&self) -> usize {
        1
    }
    fn texts(&self, _: usize) -> Vec<std::sync::Arc<Vec<char>>> {
        vec![std::sync::Arc::new(self.0.clone())]
    }
}

#[test]
fn every_block_is_found_where_the_reader_numbers_it() {
    let mut checked = 0;
    for seed in 1..=40u64 {
        let (html, expected) = chapter(seed.wrapping_mul(0x9E37_79B9), 12 + (seed as usize % 20));
        let doc = parse_html(&html, "c.html");
        let rt: Vec<char> = doc.reader_text().chars().collect();
        let lines: Vec<String> = doc.reader_text().split('\n').map(str::to_string).collect();
        // the generator's blocks and the reader's lines agree in number and in text
        let want: Vec<&String> = expected.iter().collect();
        assert_eq!(
            lines.len(),
            want.len(),
            "seed {seed}: {} lines vs {} generated blocks\n{:#?}\n{:#?}",
            lines.len(),
            want.len(),
            lines,
            want
        );
        let mut start = 0usize;
        for (line, exp) in lines.iter().zip(&expected) {
            assert!(
                anchor::same_text(line, exp),
                "seed {seed}: reader line {line:?} is not generated text {exp:?}"
            );
            // a quote of the block is found exactly at the block
            let quote: String = exp.trim_start_matches("•\t").to_string();
            if quote.chars().count() >= 12 {
                let hint = start + 3;
                let f = anchor::locate(&rt, &quote, Some(hint))
                    .unwrap_or_else(|| panic!("seed {seed}: {quote:?} not found in the chapter"));
                assert!(
                    matches!(f.kind, Match::Exact | Match::Normalised | Match::CaseFolded),
                    "seed {seed}: {:?}",
                    f.kind
                );
                assert!(
                    f.start >= start && f.end <= start + line.chars().count(),
                    "seed {seed}: {quote:?} found at {}..{} outside its block {start}..{}",
                    f.start,
                    f.end,
                    start + line.chars().count()
                );
                assert!(anchor::verify(&rt, f.start as i32, f.end as i32, &quote));
                checked += 1;
            }
            start += line.chars().count() + 1;
        }
    }
    assert!(checked > 400, "{checked} quotes checked");
}

#[test]
fn offsets_counted_in_the_export_text_are_off_by_the_blocks_before_them() {
    for seed in 1..=15u64 {
        let (html, expected) = chapter(seed * 77, 25);
        let doc = parse_html(&html, "c.html");
        let ch = One(doc.reader_text().chars().collect());
        let export = doc.to_text().replace('\u{ad}', "");
        // the last block that is plain text: its place in the export text is far ahead of the reader's
        let target = expected
            .iter()
            .rposition(|t| !t.starts_with('•') && t.chars().count() > 20)
            .unwrap();
        let quote = &expected[target];
        let old_start = export
            .find(quote.as_str())
            .map(|b| export[..b].chars().count())
            .or_else(|| {
                // headings are upper-cased in the export
                export
                    .find(&quote.to_uppercase())
                    .map(|b| export[..b].chars().count())
            })
            .expect("quote is in the export text") as i32;
        let mut h = Highlight {
            id: "x".into(),
            spine: 0,
            start: old_start,
            end: old_start + quote.chars().count() as i32,
            color: "yellow".into(),
            style: String::new(),
            note: "n".into(),
            text: quote.clone(),
            created: chrono::Utc::now(),
        };
        let real = anchor::resolve(&ch, quote, Some(0), None)
            .unwrap()
            .found
            .start as i32;
        assert!(old_start > real, "seed {seed}: {old_start} {real}");
        assert_eq!(
            anchor::repair_one(&ch, &mut h),
            Repair::Moved,
            "seed {seed}"
        );
        assert_eq!(h.start, real, "seed {seed}");
        assert_eq!(h.note, "n");
    }
}

#[test]
fn the_readers_text_ignores_what_the_reader_does_not_show() {
    let doc = parse_html(
        "<body><h1>Title</h1><p>One\u{ad}two  <br/>three</p><hr/><p>   </p><ul><li>a</li></ul><p>End</p></body>",
        "c.html",
    );
    // headings keep their case, the soft hyphen does not count, a rule and empty blocks have no
    // text, a list marker and its tab belong to the item
    assert_eq!(doc.reader_text(), "Title\nOnetwo \nthree\n•\ta\nEnd");
}
