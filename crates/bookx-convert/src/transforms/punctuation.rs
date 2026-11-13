//! Smarten / unsmarten punctuation (curly quotes, dashes, ellipsis) on the text nodes of XHTML.

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};

use super::map_spine_files;
use crate::markup::{is_block, is_verbatim, tag_info, tokenize, Tok};

const OPEN_CONTEXT: &str = "([{<-/=\u{2014}\u{2013}\u{201c}\u{2018}\u{ab}\u{a0}";

const ELISIONS: &[&str] = &[
    "tis", "twas", "twere", "twill", "twould", "til", "em", "cause", "bout", "round", "n", "nuff",
    "sup", "cept", "fore", "gainst", "neath", "cos", "kay", "scuse",
];

fn opens(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => c.is_whitespace() || OPEN_CONTEXT.contains(c),
    }
}

fn simple_quote_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x22;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// Which kinds of punctuation [`smarten_html_with`] converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartenOptions {
    pub quotes: bool,
    pub dashes: bool,
    pub ellipses: bool,
}

impl Default for SmartenOptions {
    fn default() -> Self {
        SmartenOptions {
            quotes: true,
            dashes: true,
            ellipses: true,
        }
    }
}

fn smarten_text_with(text: &str, prev: &mut Option<char>, opts: SmartenOptions) -> String {
    let src: Vec<char> = simple_quote_entities(text).chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        let next = src.get(i + 1).copied();
        let emitted: Option<char> = match c {
            '-' | '.' => {
                let enabled = if c == '-' { opts.dashes } else { opts.ellipses };
                let mut j = i;
                while j < src.len() && src[j] == c {
                    j += 1;
                }
                let run = j - i;
                let rep = match (c, run) {
                    _ if !enabled => None,
                    ('-', 2) => Some('\u{2013}'),
                    ('-', 3) => Some('\u{2014}'),
                    ('.', 3) => Some('\u{2026}'),
                    _ => None,
                };
                match rep {
                    Some(r) => {
                        out.push(r);
                        i = j;
                        *prev = Some(r);
                    }
                    None => {
                        for _ in 0..run {
                            out.push(c);
                        }
                        i = j;
                        *prev = Some(c);
                    }
                }
                continue;
            }
            '"' | '\'' if !opts.quotes => {
                out.push(c);
                *prev = Some(c);
                i += 1;
                continue;
            }
            '"' => {
                let open = opens(*prev) && !next.is_some_and(|n| n.is_whitespace());
                Some(if open { '\u{201c}' } else { '\u{201d}' })
            }
            '\'' => Some(single_quote(&src, i, *prev)),
            other => {
                out.push(other);
                *prev = Some(other);
                i += 1;
                continue;
            }
        };
        if let Some(e) = emitted {
            out.push(e);
            *prev = Some(e);
        }
        i += 1;
    }
    out
}

fn single_quote(src: &[char], i: usize, prev: Option<char>) -> char {
    const RIGHT: char = '\u{2019}';
    const LEFT: char = '\u{2018}';
    if prev.is_some_and(|p| p.is_alphanumeric()) {
        return RIGHT;
    }
    if !opens(prev) {
        return RIGHT;
    }
    let next = src.get(i + 1).copied();
    match next {
        None => LEFT,
        Some(n) if n.is_whitespace() => RIGHT,
        Some(n) if n.is_ascii_digit() => {
            let digits = src[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            if digits == 2 || digits == 4 && src.get(i + 5) == Some(&'s') {
                RIGHT
            } else {
                LEFT
            }
        }
        Some(n) if n.is_alphabetic() => {
            let word: String = src[i + 1..]
                .iter()
                .take_while(|c| c.is_alphabetic())
                .collect::<String>()
                .to_lowercase();
            if ELISIONS.contains(&word.as_str()) {
                RIGHT
            } else {
                LEFT
            }
        }
        Some(_) => LEFT,
    }
}

fn unsmarten_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{201c}' | '\u{201d}' | '\u{201e}' => out.push('"'),
            '\u{2018}' | '\u{2019}' | '\u{201a}' => out.push('\''),
            '\u{2013}' => out.push_str("--"),
            '\u{2014}' => out.push_str("---"),
            '\u{2026}' => out.push_str("..."),
            o => out.push(o),
        }
    }
    out
}

/// Applies `f` to every text node outside verbatim elements. `reset` tells `f` that a block
/// boundary was crossed (so the previous character is forgotten).
fn rewrite_text(html: &str, mut f: impl FnMut(&str, bool, bool) -> String) -> String {
    let mut out = String::with_capacity(html.len());
    let mut verbatim = 0usize;
    let mut reset = true;
    for t in tokenize(html) {
        match t {
            Tok::Tag(tag) => {
                out.push_str(tag);
                if let Some(info) = tag_info(tag) {
                    if is_verbatim(&info.name) && !info.self_closing {
                        if info.closing {
                            verbatim = verbatim.saturating_sub(1);
                        } else {
                            verbatim += 1;
                        }
                    }
                    if is_block(&info.name) {
                        reset = true;
                    }
                }
            }
            Tok::Text(s) => {
                out.push_str(&f(s, reset, verbatim > 0));
                reset = false;
            }
        }
    }
    out
}

/// Curly quotes, en/em dashes and ellipses in an XHTML document; code, `pre`, scripts, styles and
/// attributes are never touched.
pub fn smarten_html(html: &str) -> String {
    smarten_html_with(html, SmartenOptions::default())
}

/// [`smarten_html`] for the chosen kinds of punctuation only.
pub fn smarten_html_with(html: &str, opts: SmartenOptions) -> String {
    let mut prev: Option<char> = None;
    rewrite_text(html, |text, reset, verbatim| {
        if reset {
            prev = None;
        }
        if verbatim {
            prev = text.chars().last().or(prev);
            return text.to_string();
        }
        smarten_text_with(text, &mut prev, opts)
    })
}

/// The inverse: straight quotes, `--`/`---` dashes and `...`.
pub fn unsmarten_html(html: &str) -> String {
    rewrite_text(html, |text, _, verbatim| {
        if verbatim {
            text.to_string()
        } else {
            unsmarten_text(text)
        }
    })
}

#[derive(Debug, Clone, Default)]
pub struct SmartenPunctuation {
    pub jobs: usize,
}

#[derive(Debug, Clone, Default)]
pub struct UnsmartenPunctuation {
    pub jobs: usize,
}

impl Transform for SmartenPunctuation {
    fn id(&self) -> &'static str {
        "smarten_punctuation"
    }
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let n = map_spine_files(book, self.jobs, |_, t| Some(smarten_html(t)))?;
        Ok(TransformReport {
            files_changed: n,
            notes: vec![],
        })
    }
}

impl Transform for UnsmartenPunctuation {
    fn id(&self) -> &'static str {
        "unsmarten_punctuation"
    }
    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let n = map_spine_files(book, self.jobs, |_, t| Some(unsmarten_html(t)))?;
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

    fn sm(s: &str) -> String {
        smarten_html(s)
    }

    #[test]
    fn options_limit_what_is_converted() {
        let src = "<p>\"Hi\" -- wait... ok</p>";
        let only = |quotes, dashes, ellipses| {
            smarten_html_with(
                src,
                SmartenOptions {
                    quotes,
                    dashes,
                    ellipses,
                },
            )
        };
        assert_eq!(only(true, false, false), "<p>“Hi” -- wait... ok</p>");
        assert_eq!(only(false, true, false), "<p>\"Hi\" – wait... ok</p>");
        assert_eq!(only(false, false, true), "<p>\"Hi\" -- wait… ok</p>");
        assert_eq!(only(true, true, true), smarten_html(src));
    }

    #[test]
    fn double_quotes_open_and_close() {
        assert_eq!(
            sm("<p>\"Hello,\" she said.</p>"),
            "<p>“Hello,” she said.</p>"
        );
        assert_eq!(
            sm("<p>He said \"go\" twice</p>"),
            "<p>He said “go” twice</p>"
        );
    }

    #[test]
    fn quote_after_inline_tag_uses_context() {
        assert_eq!(sm("<p><i>\"Hi\"</i> and</p>"), "<p><i>“Hi”</i> and</p>");
        assert_eq!(
            sm("<p>word<i>x</i>\" next</p>"),
            "<p>word<i>x</i>” next</p>"
        );
        assert_eq!(sm("<p>a <b>\"b\"</b></p>"), "<p>a <b>“b”</b></p>");
    }

    #[test]
    fn block_boundary_resets_context() {
        assert_eq!(sm("<p>end</p><p>\"Start\"</p>"), "<p>end</p><p>“Start”</p>");
        assert_eq!(sm("<p>x<br/>\"y\"</p>"), "<p>x<br/>“y”</p>");
    }

    #[test]
    fn apostrophes() {
        assert_eq!(
            sm("<p>don't it's rock 'n' roll</p>"),
            "<p>don’t it’s rock ’n’ roll</p>"
        );
        assert_eq!(
            sm("<p>'Tis the season, 'twas</p>"),
            "<p>’Tis the season, ’twas</p>"
        );
        assert_eq!(
            sm("<p>in the '90s and '08</p>"),
            "<p>in the ’90s and ’08</p>"
        );
        assert_eq!(sm("<p>the dogs' bones</p>"), "<p>the dogs’ bones</p>");
    }

    #[test]
    fn nested_quotes() {
        assert_eq!(
            sm("<p>\"She said 'no' to me,\" he said.</p>"),
            "<p>“She said ‘no’ to me,” he said.</p>"
        );
        assert_eq!(sm("<p>\"'Hi,' she said\"</p>"), "<p>“‘Hi,’ she said”</p>");
        assert_eq!(sm("<p>'Hello there'</p>"), "<p>‘Hello there’</p>");
    }

    #[test]
    fn dashes_and_ellipsis() {
        assert_eq!(sm("<p>a--b---c...d</p>"), "<p>a–b—c…d</p>");
        // longer runs are left alone (horizontal rules made of text)
        assert_eq!(sm("<p>-----</p>"), "<p>-----</p>");
        assert_eq!(sm("<p>wait....</p>"), "<p>wait....</p>");
        assert_eq!(sm("<p>a - b</p>"), "<p>a - b</p>");
    }

    #[test]
    fn entities_for_quotes_are_decoded_first() {
        assert_eq!(
            sm("<p>&quot;Hi&quot; &amp; &#39;bye&#39;</p>"),
            "<p>“Hi” &amp; ‘bye’</p>"
        );
    }

    #[test]
    fn code_pre_and_attributes_are_untouched() {
        let s = "<p title=\"a 'b' \\\"c\\\"\">x<code>\"q\" -- ...</code></p><pre>\"p\" 'x'</pre><script>var s=\"x\";</script><style>a[href=\"x\"]{}</style>";
        assert_eq!(sm(s), s);
        let mixed = "<p class=\"a\">\"x\"<code>'y'</code> \"z\"</p>";
        assert_eq!(sm(mixed), "<p class=\"a\">“x”<code>'y'</code> “z”</p>");
    }

    #[test]
    fn already_curly_text_is_stable() {
        let s = "<p>“Hi,” she said — ‘ok’… fine–ish</p>";
        assert_eq!(sm(s), s);
    }

    #[test]
    fn smarten_is_idempotent() {
        let s = "<p>\"It's 'twas,\" she said -- \"and 'nested' ...\"</p><p>'90s --- ok</p>";
        let once = sm(s);
        assert_eq!(sm(&once), once);
    }

    #[test]
    fn unsmarten_reverts() {
        assert_eq!(
            unsmarten_html("<p>“Hi,” it’s a—b–c… ‘x’</p><code>“keep”</code>"),
            "<p>\"Hi,\" it's a---b--c... 'x'</p><code>“keep”</code>"
        );
        let once = unsmarten_html("<p>“a” — b</p>");
        assert_eq!(unsmarten_html(&once), once);
    }

    #[test]
    fn smarten_then_unsmarten_roundtrips_simple_text() {
        let s = "<p>\"Hi,\" he said --- and left...</p>";
        assert_eq!(unsmarten_html(&sm(s)), s);
    }

    #[test]
    fn unicode_text_survives() {
        assert_eq!(sm("<p>日本語 \"引用\" ünï</p>"), "<p>日本語 “引用” ünï</p>");
    }

    #[test]
    fn transform_reports_changed_files_and_is_idempotent() {
        let mut b = book_with(&[("A", "<p>\"x\"</p>"), ("B", "<p>plain</p>")]);
        let t = SmartenPunctuation::default();
        let r = t.apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 1);
        assert!(chapter(&b, 0).contains("“x”"));
        let snap = snapshot(&b);
        let r2 = t.apply(&mut b).unwrap();
        assert_eq!(r2.files_changed, 0);
        assert_eq!(snapshot(&b), snap);
        assert_eq!(t.id(), "smarten_punctuation");

        let u = UnsmartenPunctuation::default();
        assert_eq!(u.apply(&mut b).unwrap().files_changed, 1);
        assert!(chapter(&b, 0).contains("\"x\""));
        assert_eq!(u.apply(&mut b).unwrap().files_changed, 0);
        assert_eq!(u.id(), "unsmarten_punctuation");
    }

    #[test]
    fn parallel_equals_sequential() {
        let chapters: Vec<(String, String)> = (0..8)
            .map(|i| (format!("C{i}"), format!("<p>\"q{i}\" -- it's</p>")))
            .collect();
        let refs: Vec<(&str, &str)> = chapters
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let mut a = book_with(&refs);
        let mut b = book_with(&refs);
        SmartenPunctuation { jobs: 1 }.apply(&mut a).unwrap();
        SmartenPunctuation { jobs: 4 }.apply(&mut b).unwrap();
        for i in 0..8 {
            assert_eq!(chapter(&a, i), chapter(&b, i));
        }
    }
}
