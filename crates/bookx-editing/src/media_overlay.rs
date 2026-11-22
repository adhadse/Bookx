//! EPUB 3 Media Overlays, the pure parts (CE-422): wrapping the sentences of a chapter in
//! `<span id>`, SMIL clock values, building and parsing SMIL documents, timing.
//!
//! Related modules: `media_overlay_opf` (package document edits and removal),
//! `media_overlay_check` (validation) and `media_overlay_narrate` (the per-chapter pipeline that
//! turns text into audio with a [`SentenceSynth`](crate::media_overlay_narrate::SentenceSynth)).
//!
//! Everything that reads a book here is **untrusted input**: SMIL files are limited in size, depth
//! and number of `par` elements, and a reference to another scheme, an absolute path or a path
//! leaving the book is dropped (never followed).

use std::ops::Range;

use anyhow::{bail, Result};
use bookx_core::util::{parent_dir, relative_href, xml_escape};

use crate::markup::{scan, tag_attr, Event};

/// Id prefix of the spans this module adds (`bxs1`, `bxs2`, ...).
pub const SPAN_PREFIX: &str = "bxs";
/// Largest SMIL document that is parsed.
pub const MAX_SMIL_BYTES: usize = 16 * 1024 * 1024;
/// Most `par` elements read from one SMIL document.
pub const MAX_PARS: usize = 200_000;
/// Deepest `seq` nesting read.
pub const MAX_DEPTH: usize = 32;
/// Longest clock value accepted (a book longer than this is not a book).
pub const MAX_CLOCK_MS: u64 = 1000 * 3600 * 1000;

/// Class given to the sentence being read (`media:active-class`).
pub const ACTIVE_CLASS: &str = "-epub-media-overlay-active";

/// One sentence of a chapter: the id of its span and the plain text to be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    pub id: String,
    pub text: String,
}

/// A chapter with its sentences wrapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    pub xhtml: String,
    pub sentences: Vec<Sentence>,
}

const VOID: &[&str] = &[
    "br", "hr", "img", "meta", "link", "input", "col", "area", "base", "wbr", "source", "track",
    "embed", "param",
];

/// Elements whose text is never read.
const SKIP: &[&str] = &[
    "head", "script", "style", "title", "rt", "rp", "svg", "math", "template", "noscript",
    "object", "audio", "video", "canvas", "select", "option", "textarea",
];

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "e.g", "i.e", "mt", "fig", "capt",
    "sgt", "inc", "ltd", "approx", "dept", "cf",
];

/// Decodes the XML/HTML character references the editor produces (`&amp; &lt; &gt; &quot; &apos;
/// &nbsp; &#N; &#xN;`). Unknown references are kept as they are.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.find(';').filter(|&e| e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '?' | '!' | '…' | '。' | '！' | '？' | '؟' | '।')
}

fn is_cjk_terminator(c: char) -> bool {
    matches!(c, '。' | '！' | '？')
}

fn is_closer(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ')' | ']' | '}' | '”' | '’' | '»' | '›' | '」' | '』' | '）'
    )
}

/// The word that ends at byte `end` of `s` (letters, digits and inner dots).
fn word_before(s: &str, end: usize) -> &str {
    let head = &s[..end];
    let start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '.')
        .last()
        .map(|(i, _)| i)
        .unwrap_or(end);
    &head[start..]
}

fn is_abbreviation(word: &str) -> bool {
    let w = word.trim_matches('.');
    if w.is_empty() {
        return false;
    }
    if w.chars().count() == 1 && w.chars().all(|c| c.is_uppercase()) {
        return true;
    }
    ABBREVIATIONS.contains(&w.to_lowercase().as_str())
}

/// Splits one run of text (source form: entities still encoded) into sentence ranges. The ranges
/// cover non-whitespace text only; whitespace between sentences belongs to nobody.
pub fn split_sentence_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if start.is_none() {
            if c.is_whitespace() {
                continue;
            }
            start = Some(i);
        }
        if !is_terminator(c) {
            continue;
        }
        // swallow a run of terminators and closing quotes/brackets
        let mut end = i + c.len_utf8();
        while let Some(&(j, d)) = it.peek() {
            if is_terminator(d) || is_closer(d) {
                end = j + d.len_utf8();
                it.next();
            } else {
                break;
            }
        }
        let next = it.peek().map(|&(_, d)| d);
        let boundary = match next {
            None => true,
            Some(d) if d.is_whitespace() => true,
            Some(_) => is_cjk_terminator(c),
        };
        if !boundary {
            continue;
        }
        if c == '.' && is_abbreviation(word_before(text, i)) && next.is_some() {
            continue;
        }
        if let Some(s) = start.take() {
            out.push(s..end);
        }
    }
    if let Some(s) = start {
        let e = text.trim_end().len();
        if e > s {
            out.push(s..e);
        }
    }
    out
}

fn has_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric())
}

/// Wraps every sentence of the text of `xhtml` in `<span id="bxsN">…</span>`. Only the body is
/// touched; script, style, ruby annotations, SVG and MathML are left alone, and whitespace stays
/// where it was, so the page renders the same. A sentence that spans an inline element (`<i>`) is
/// split at the element. Fails when the chapter already has narration spans.
pub fn wrap_sentences(xhtml: &str) -> Result<Wrapped> {
    if xhtml.contains(&format!("id=\"{SPAN_PREFIX}1\""))
        || xhtml.contains(&format!("id='{SPAN_PREFIX}1'"))
    {
        bail!("this chapter already has narration spans");
    }
    let mut out = String::with_capacity(xhtml.len() + xhtml.len() / 8);
    let mut sentences = Vec::new();
    let mut last = 0;
    let mut in_body = false;
    // names of the open elements; a skipped element remembers the depth it started at
    let mut stack: Vec<String> = Vec::new();
    let mut skip_from: Option<usize> = None;
    for ev in scan(xhtml) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                if name == "body" && !in_body {
                    in_body = true;
                }
                if VOID.contains(&name.as_str()) || self_closing {
                    continue;
                }
                stack.push(name.clone());
                if skip_from.is_none() && SKIP.contains(&name.as_str()) {
                    skip_from = Some(stack.len());
                }
                let tag = &xhtml[range];
                if skip_from.is_none() && tag_attr(tag, "aria-hidden").as_deref() == Some("true") {
                    skip_from = Some(stack.len());
                }
            }
            Event::Close { name, .. } => {
                if name == "body" {
                    in_body = false;
                }
                if let Some(pos) = stack.iter().rposition(|n| *n == name) {
                    stack.truncate(pos);
                    if skip_from.is_some_and(|d| stack.len() < d) {
                        skip_from = None;
                    }
                }
            }
            Event::Text(r) => {
                if !in_body || skip_from.is_some() {
                    continue;
                }
                let text = &xhtml[r.clone()];
                let ranges: Vec<Range<usize>> = split_sentence_ranges(text)
                    .into_iter()
                    .filter(|s| has_letter(&text[s.clone()]))
                    .collect();
                if ranges.is_empty() {
                    continue;
                }
                out.push_str(&xhtml[last..r.start]);
                let mut at = 0;
                for s in ranges {
                    out.push_str(&text[at..s.start]);
                    let id = format!("{SPAN_PREFIX}{}", sentences.len() + 1);
                    out.push_str(&format!("<span id=\"{id}\">{}</span>", &text[s.clone()]));
                    sentences.push(Sentence {
                        id,
                        text: normalize_space(&decode_entities(&text[s.clone()])),
                    });
                    at = s.end;
                }
                out.push_str(&text[at..]);
                last = r.end;
            }
            Event::Other(_) => {}
        }
    }
    out.push_str(&xhtml[last..]);
    Ok(Wrapped {
        xhtml: out,
        sentences,
    })
}

fn normalize_space(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Undoes [`wrap_sentences`]: removes the `<span id="bxsN">` wrappers (and nothing else).
pub fn unwrap_sentences(xhtml: &str) -> String {
    let mut out = String::with_capacity(xhtml.len());
    let mut last = 0;
    // for each open span: ours (true) or somebody else's (false)
    let mut spans: Vec<bool> = Vec::new();
    for ev in scan(xhtml) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } if name == "span" && !self_closing => {
                let tag = &xhtml[range.clone()];
                let ours = is_our_span(tag);
                spans.push(ours);
                if ours {
                    out.push_str(&xhtml[last..range.start]);
                    last = range.end;
                }
            }
            Event::Close { name, range } if name == "span" && spans.pop() == Some(true) => {
                out.push_str(&xhtml[last..range.start]);
                last = range.end;
            }
            _ => {}
        }
    }
    out.push_str(&xhtml[last..]);
    out
}

fn is_our_span(tag: &str) -> bool {
    let Some(id) = tag_attr(tag, "id") else {
        return false;
    };
    let Some(n) = id.strip_prefix(SPAN_PREFIX) else {
        return false;
    };
    !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
        && crate::markup::tag_attrs(tag).len() == 1
}

// ---------------------------------------------------------------------------------------------
// clock values

/// `h:mm:ss.mmm` (hours are not padded, as in the EPUB examples).
pub fn format_clock(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = ms / 60_000 % 60;
    let s = ms / 1000 % 60;
    format!("{h}:{m:02}:{s:02}.{:03}", ms % 1000)
}

/// Parses a SMIL clock value: full (`1:02:03.5`), partial (`02:03.5`) or timecount (`3.5s`,
/// `250ms`, `2min`, `1h`, `12`). Negative, non-finite and absurdly large values are `None`.
pub fn parse_clock(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 32 {
        return None;
    }
    let ms: f64 = if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() > 3 || parts.iter().any(|p| p.is_empty()) {
            return None;
        }
        let secs = parts.last()?.parse::<f64>().ok()?;
        let mut total = secs;
        let mut mult = 60.0;
        for p in parts[..parts.len() - 1].iter().rev() {
            if !p.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            total += p.parse::<f64>().ok()? * mult;
            mult *= 60.0;
        }
        if parts.len() >= 2 && secs >= 60.0 {
            return None;
        }
        if parts.len() == 3 && parts[1].parse::<u32>().ok()? >= 60 {
            return None;
        }
        total * 1000.0
    } else if let Some(v) = s.strip_suffix("ms") {
        v.trim().parse::<f64>().ok()?
    } else if let Some(v) = s.strip_suffix("min") {
        v.trim().parse::<f64>().ok()? * 60_000.0
    } else if let Some(v) = s.strip_suffix('h') {
        v.trim().parse::<f64>().ok()? * 3_600_000.0
    } else if let Some(v) = s.strip_suffix('s') {
        v.trim().parse::<f64>().ok()? * 1000.0
    } else {
        s.parse::<f64>().ok()? * 1000.0
    };
    if !ms.is_finite() || ms < 0.0 || ms > MAX_CLOCK_MS as f64 {
        return None;
    }
    Some(ms.round() as u64)
}

// ---------------------------------------------------------------------------------------------
// SMIL

/// One `par`: a text fragment and the audio clip that reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Par {
    /// Fragment id in the chapter (`bxs3`).
    pub text_id: String,
    pub begin_ms: u64,
    pub end_ms: u64,
}

/// What [`build_smil`] writes: references are relative to the SMIL file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmilDoc {
    pub text_href: String,
    pub audio_href: String,
    pub pars: Vec<Par>,
}

/// Relative reference from the SMIL file to `target`.
pub fn href_from(smil_path: &str, target: &str) -> String {
    relative_href(smil_path, target)
}

/// The SMIL document of one chapter: a `seq` with one `par` per sentence.
pub fn build_smil(doc: &SmilDoc) -> String {
    let mut s = String::with_capacity(160 + doc.pars.len() * 200);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<smil xmlns=\"http://www.w3.org/ns/SMIL\" xmlns:epub=\"http://www.idpf.org/2007/ops\" version=\"3.0\">\n");
    s.push_str("  <body>\n");
    s.push_str(&format!(
        "    <seq id=\"seq1\" epub:textref=\"{}\" epub:type=\"bodymatter chapter\">\n",
        xml_escape(&doc.text_href)
    ));
    for (i, p) in doc.pars.iter().enumerate() {
        s.push_str(&format!(
            "      <par id=\"par{}\"><text src=\"{}#{}\"/><audio src=\"{}\" clipBegin=\"{}\" clipEnd=\"{}\"/></par>\n",
            i + 1,
            xml_escape(&doc.text_href),
            xml_escape(&p.text_id),
            xml_escape(&doc.audio_href),
            format_clock(p.begin_ms),
            format_clock(p.end_ms)
        ));
    }
    s.push_str("    </seq>\n  </body>\n</smil>\n");
    s
}

/// A `par` read from a book, references resolved to container paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPar {
    pub text_path: String,
    pub text_id: Option<String>,
    pub audio_path: String,
    pub begin_ms: u64,
    pub end_ms: u64,
}

/// The result of reading a SMIL file; `skipped` counts `par` elements that were unusable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadSmil {
    pub pars: Vec<ReadPar>,
    pub skipped: usize,
    /// More than [`MAX_PARS`] pars were present; the rest was ignored.
    pub truncated: bool,
}

/// Resolves a reference found in the SMIL file at `smil_path` to a container path and fragment.
/// `None` for anything that is not a plain relative reference into the book.
pub fn resolve_local(smil_path: &str, href: &str) -> Option<(String, Option<String>)> {
    let href = href.trim();
    if href.is_empty() || href.len() > 2048 || href.contains(['\\', '\0']) || href.starts_with('/')
    {
        return None;
    }
    let path_part = href.split('#').next().unwrap_or("");
    // a scheme (`http:`, `file:`, `data:`) before the first slash
    if let Some(colon) = path_part.find(':') {
        if !path_part[..colon].contains('/') {
            return None;
        }
    }
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f.to_string()).filter(|f| !f.is_empty())),
        None => (href, None),
    };
    if path.is_empty() {
        return None;
    }
    let joined = if parent_dir(smil_path).is_empty() {
        path.to_string()
    } else {
        format!("{}/{}", parent_dir(smil_path), path)
    };
    // resolve by hand: a `..` that would leave the book is an error, not silently dropped
    let mut parts: Vec<&str> = Vec::new();
    let decoded = bookx_core::util::percent_decode(&joined);
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return None;
    }
    let norm = parts.join("/");
    Some((norm, frag))
}

/// Reads the `par` elements of a SMIL document. Never follows references; drops the unsafe ones.
pub fn parse_smil(smil_path: &str, xml: &str) -> Result<ReadSmil> {
    if xml.len() > MAX_SMIL_BYTES {
        bail!("the SMIL file is larger than {MAX_SMIL_BYTES} bytes");
    }
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 4_000_000,
            ..Default::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("the SMIL file is not well-formed: {e}"))?;
    let mut out = ReadSmil::default();
    let mut stack = vec![(doc.root_element(), 0usize)];
    // document order
    let mut order: Vec<roxmltree::Node> = Vec::new();
    while let Some((n, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            bail!("the SMIL file nests deeper than {MAX_DEPTH} levels");
        }
        order.push(n);
        for c in n.children().filter(|c| c.is_element()).rev() {
            stack.push((c, depth + 1));
        }
    }
    for n in order {
        if n.tag_name().name() != "par" {
            continue;
        }
        if out.pars.len() + out.skipped >= MAX_PARS {
            out.truncated = true;
            break;
        }
        let text = n.descendants().find(|d| d.has_tag_name("text"));
        let audio = n.descendants().find(|d| d.has_tag_name("audio"));
        let (Some(text), Some(audio)) = (text, audio) else {
            out.skipped += 1;
            continue;
        };
        let t = text
            .attribute("src")
            .and_then(|s| resolve_local(smil_path, s));
        let a = audio
            .attribute("src")
            .and_then(|s| resolve_local(smil_path, s));
        let begin = audio
            .attribute("clipBegin")
            .map(parse_clock)
            .unwrap_or(Some(0));
        let end = audio.attribute("clipEnd").and_then(parse_clock);
        match (t, a, begin, end) {
            (Some((tp, tf)), Some((ap, _)), Some(b), Some(e)) if e > b => out.pars.push(ReadPar {
                text_path: tp,
                text_id: tf,
                audio_path: ap,
                begin_ms: b,
                end_ms: e,
            }),
            _ => out.skipped += 1,
        }
    }
    Ok(out)
}

/// The text of the elements with the given ids (descendant text, entities decoded, spaces
/// collapsed). Used to read what an overlay's `par` says. Elements nested in each other are fine.
pub fn fragment_texts(
    xhtml: &str,
    ids: &std::collections::HashSet<&str>,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    // (id, depth of the element when it opened, text so far)
    let mut open: Vec<(String, usize, String)> = Vec::new();
    let mut depth = 0usize;
    for ev in scan(xhtml) {
        match ev {
            Event::Open {
                name,
                range,
                self_closing,
            } => {
                let is_void = VOID.contains(&name.as_str()) || self_closing;
                if !ids.is_empty() {
                    if let Some(id) =
                        tag_attr(&xhtml[range], "id").filter(|i| ids.contains(i.as_str()))
                    {
                        if is_void {
                            out.insert(id, String::new());
                        } else {
                            open.push((id, depth, String::new()));
                        }
                    }
                }
                if !is_void {
                    depth += 1;
                }
            }
            Event::Close { .. } => {
                depth = depth.saturating_sub(1);
                while open.last().is_some_and(|(_, d, _)| *d >= depth) {
                    if let Some((id, _, t)) = open.pop() {
                        out.insert(id, normalize_space(&decode_entities(&t)));
                    }
                }
            }
            Event::Text(r) => {
                for (_, _, t) in open.iter_mut() {
                    t.push_str(&xhtml[r.clone()]);
                }
            }
            Event::Other(_) => {}
        }
    }
    out
}

/// Total duration of a list of pars: the end of the last one.
pub fn duration_ms(pars: &[Par]) -> u64 {
    pars.iter().map(|p| p.end_ms).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CH: &str = "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>It is. A title.</title><style>p { color: red }</style></head>\n<body>\n<h1>Chapter One</h1>\n<p>Hello world. Mr. Smith went to Washington! Did he?  Yes &amp; no.</p>\n<p>He said <i>hello</i> to <b>her</b>.</p>\n<script>var a = 'x. y.';</script>\n<p>Last one</p>\n</body></html>";

    fn texts(w: &Wrapped) -> Vec<&str> {
        w.sentences.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn sentences_are_wrapped_with_ids_and_decoded_text() {
        let w = wrap_sentences(CH).unwrap();
        assert_eq!(
            texts(&w),
            vec![
                "Chapter One",
                "Hello world.",
                "Mr. Smith went to Washington!",
                "Did he?",
                "Yes & no.",
                "He said",
                "hello",
                "to",
                "her",
                "Last one",
            ]
        );
        assert_eq!(w.sentences[0].id, "bxs1");
        assert!(w.xhtml.contains("<span id=\"bxs2\">Hello world.</span>"));
        assert!(w.xhtml.contains("<span id=\"bxs5\">Yes &amp; no.</span>"));
        // head, style and script are untouched
        assert!(w.xhtml.contains("<title>It is. A title.</title>"));
        assert!(w.xhtml.contains("var a = 'x. y.';"));
        assert!(!w.xhtml.contains("bxs12"));
    }

    #[test]
    fn wrapping_does_not_change_the_rendered_text() {
        let w = wrap_sentences(CH).unwrap();
        let strip = |s: &str| {
            let mut out = String::new();
            let mut in_tag = false;
            for c in s.chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    c if !in_tag => out.push(c),
                    _ => {}
                }
            }
            out
        };
        assert_eq!(strip(&w.xhtml), strip(CH));
        // and unwrapping gives the original back byte for byte
        assert_eq!(unwrap_sentences(&w.xhtml), CH);
    }

    #[test]
    fn already_narrated_chapters_are_refused() {
        let w = wrap_sentences(CH).unwrap();
        assert!(wrap_sentences(&w.xhtml).is_err());
    }

    #[test]
    fn unwrap_leaves_foreign_spans_alone() {
        let src = "<body><p><span class=\"x\">a. </span><span id=\"bxs1\">b.</span><span id=\"other\">c</span></p></body>";
        assert_eq!(
            unwrap_sentences(src),
            "<body><p><span class=\"x\">a. </span>b.<span id=\"other\">c</span></p></body>"
        );
        // a span with more attributes than the id is not ours
        let src = "<p><span id=\"bxs1\" class=\"x\">b.</span></p>";
        assert_eq!(unwrap_sentences(src), src);
    }

    #[test]
    fn sentence_splitting_rules() {
        let r = |t: &str| -> Vec<String> {
            split_sentence_ranges(t)
                .into_iter()
                .map(|r| t[r].to_string())
                .collect()
        };
        assert_eq!(r("One. Two."), vec!["One.", "Two."]);
        assert_eq!(r("  Wait... what?! Ok."), vec!["Wait...", "what?!", "Ok."]);
        assert_eq!(r("It costs 3.50 today."), vec!["It costs 3.50 today."]);
        assert_eq!(
            r("Dr. Who and J. K. Rowling."),
            vec!["Dr. Who and J. K. Rowling."]
        );
        assert_eq!(r("\"Go.\" He left."), vec!["\"Go.\"", "He left."]);
        assert_eq!(r("今日は。明日は！"), vec!["今日は。", "明日は！"]);
        assert_eq!(r("no end"), vec!["no end"]);
        assert_eq!(r("   "), Vec::<String>::new());
        assert_eq!(r("I said no. Then left."), vec!["I said no.", "Then left."]);
    }

    #[test]
    fn nothing_to_read_means_no_spans() {
        let w =
            wrap_sentences("<html><body><p>* * *</p><p>—</p><img src=\"a.png\"/></body></html>")
                .unwrap();
        assert!(w.sentences.is_empty());
        assert!(!w.xhtml.contains("<span"));
    }

    #[test]
    fn aria_hidden_ruby_and_svg_are_skipped() {
        let src = "<html><body><p aria-hidden=\"true\">Skip me.</p><p><ruby>漢<rt>kan</rt></ruby>字 read.</p><svg><text>No.</text></svg><p>Yes.</p></body></html>";
        let w = wrap_sentences(src).unwrap();
        assert!(!w
            .sentences
            .iter()
            .any(|s| s.text == "Skip me." || s.text == "kan"));
        assert!(w.sentences.iter().any(|s| s.text == "Yes."));
    }

    #[test]
    fn entities_decode() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#65;&#x42; &bogus; &"),
            "a & b <c> AB &bogus; &"
        );
        assert_eq!(decode_entities("no entities"), "no entities");
        assert_eq!(decode_entities("&#99999999999;"), "&#99999999999;");
    }

    #[test]
    fn fragment_texts_read_what_a_par_points_at() {
        let w = wrap_sentences(CH).unwrap();
        let ids: std::collections::HashSet<&str> =
            w.sentences.iter().map(|s| s.id.as_str()).collect();
        let m = fragment_texts(&w.xhtml, &ids);
        for s in &w.sentences {
            assert_eq!(m.get(&s.id), Some(&s.text), "{}", s.id);
        }
        // any element, with nested markup and entities
        let src = "<body><div id=\"a\">One <b>two</b> &amp; <i id=\"b\">three</i>\n four<br id=\"c\"/></div><p id=\"d\">x</p></body>";
        let ids: std::collections::HashSet<&str> = ["a", "b", "c", "d", "zz"].into_iter().collect();
        let m = fragment_texts(src, &ids);
        assert_eq!(m["a"], "One two & three four");
        assert_eq!(m["b"], "three");
        assert_eq!(m["c"], "");
        assert_eq!(m["d"], "x");
        assert!(!m.contains_key("zz"));
        // unclosed markup does not panic
        let _ = fragment_texts("<p id=\"a\">open", &["a"].into_iter().collect());
    }

    #[test]
    fn clocks() {
        assert_eq!(format_clock(0), "0:00:00.000");
        assert_eq!(format_clock(3_723_456), "1:02:03.456");
        for ms in [0, 1, 999, 1000, 61_001, 3_600_000, 36_000_000] {
            assert_eq!(parse_clock(&format_clock(ms)), Some(ms), "{ms}");
        }
        assert_eq!(parse_clock("1.5s"), Some(1500));
        assert_eq!(parse_clock("250ms"), Some(250));
        assert_eq!(parse_clock("2min"), Some(120_000));
        assert_eq!(parse_clock("1h"), Some(3_600_000));
        assert_eq!(parse_clock("12"), Some(12_000));
        assert_eq!(parse_clock("02:03.5"), Some(123_500));
        for bad in [
            "",
            "-1s",
            "abc",
            "1:2:3:4",
            "1::2",
            "0:61:00",
            "0:00:75",
            "1e999",
            "NaN",
            "inf",
            "5000h",
            "1:xx:00",
            &"9".repeat(40),
        ] {
            assert_eq!(parse_clock(bad), None, "{bad}");
        }
    }

    fn doc() -> SmilDoc {
        SmilDoc {
            text_href: "../text/ch1.xhtml".into(),
            audio_href: "../audio/ch1.wav".into(),
            pars: vec![
                Par {
                    text_id: "bxs1".into(),
                    begin_ms: 0,
                    end_ms: 1500,
                },
                Par {
                    text_id: "bxs2".into(),
                    begin_ms: 1500,
                    end_ms: 4000,
                },
            ],
        }
    }

    #[test]
    fn smil_round_trips() {
        let xml = build_smil(&doc());
        assert!(roxmltree::Document::parse(&xml).is_ok());
        assert!(xml.contains("clipBegin=\"0:00:01.500\" clipEnd=\"0:00:04.000\""));
        let r = parse_smil("OEBPS/smil/ch1.smil", &xml).unwrap();
        assert_eq!(r.skipped, 0);
        assert_eq!(r.pars.len(), 2);
        assert_eq!(r.pars[1].text_path, "OEBPS/text/ch1.xhtml");
        assert_eq!(r.pars[1].text_id.as_deref(), Some("bxs2"));
        assert_eq!(r.pars[1].audio_path, "OEBPS/audio/ch1.wav");
        assert_eq!((r.pars[1].begin_ms, r.pars[1].end_ms), (1500, 4000));
        assert_eq!(duration_ms(&doc().pars), 4000);
        assert_eq!(duration_ms(&[]), 0);
    }

    #[test]
    fn hostile_references_are_dropped() {
        let par = |t: &str, a: &str, b: &str, e: &str| {
            format!("<par><text src=\"{t}\"/><audio src=\"{a}\" clipBegin=\"{b}\" clipEnd=\"{e}\"/></par>")
        };
        let xml = format!(
            "<smil xmlns=\"http://www.w3.org/ns/SMIL\"><body><seq>{}{}{}{}{}{}{}{}</seq></body></smil>",
            par("http://evil.example/x.xhtml#a", "a.wav", "0s", "1s"),
            par("t.xhtml#a", "file:///etc/passwd", "0s", "1s"),
            par("t.xhtml#a", "/etc/passwd", "0s", "1s"),
            par("../../etc/passwd", "a.wav", "0s", "1s"),
            par("t.xhtml#a", "a.wav", "2s", "1s"),
            par("t.xhtml#a", "a.wav", "x", "1s"),
            par("data:text/html,x", "a.wav", "0s", "1s"),
            par("t.xhtml#ok", "a.wav", "0s", "1s"),
        );
        let r = parse_smil("OEBPS/s.smil", &xml).unwrap();
        assert_eq!(r.pars.len(), 1);
        assert_eq!(r.skipped, 7);
        assert_eq!(r.pars[0].text_id.as_deref(), Some("ok"));
    }

    #[test]
    fn malformed_smil_is_an_error_not_a_panic() {
        for bad in [
            "",
            "<smil",
            "<smil><par></smil>",
            "<!DOCTYPE smil [<!ENTITY a \"b\">]><smil/>",
            "\u{0}\u{1}",
        ] {
            assert!(parse_smil("a.smil", bad).is_err(), "{bad:?}");
        }
        // structurally odd but well-formed: nothing usable, nothing fatal
        let r = parse_smil(
            "a.smil",
            "<smil><par/><par><text/></par><par><audio/></par></smil>",
        )
        .unwrap();
        assert_eq!((r.pars.len(), r.skipped), (0, 3));
    }

    #[test]
    fn size_depth_and_count_limits() {
        let big = format!("<smil>{}</smil>", " ".repeat(MAX_SMIL_BYTES));
        assert!(parse_smil("a.smil", &big).is_err());
        let deep = format!("{}{}", "<seq>".repeat(60), "</seq>".repeat(60));
        assert!(parse_smil("a.smil", &format!("<smil>{deep}</smil>")).is_err());
        let one = "<par><text src=\"t.xhtml#a\"/><audio src=\"a.wav\" clipBegin=\"0s\" clipEnd=\"1s\"/></par>";
        let many = format!("<smil><seq>{}</seq></smil>", one.repeat(MAX_PARS + 10));
        let r = parse_smil("a.smil", &many).unwrap();
        assert!(r.truncated);
        assert_eq!(r.pars.len(), MAX_PARS);
    }

    #[test]
    fn local_references() {
        assert_eq!(
            resolve_local("OEBPS/smil/a.smil", "../text/c%201.xhtml#x"),
            Some(("OEBPS/text/c 1.xhtml".into(), Some("x".into())))
        );
        assert_eq!(
            resolve_local("a.smil", "c.xhtml"),
            Some(("c.xhtml".into(), None))
        );
        for bad in [
            "",
            "#x",
            "//host/x",
            "C:/x",
            "a\\b",
            "../x",
            "https://x",
            "javascript:1",
        ] {
            assert_eq!(resolve_local("a.smil", bad), None, "{bad}");
        }
    }
}
