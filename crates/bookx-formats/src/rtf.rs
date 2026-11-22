//! Rich Text Format reader.
//!
//! Supported: paragraphs, bold / italic / underline / strike / super / subscript, centred and
//! right aligned paragraphs, line breaks, `\uN` unicode escapes with fallback skipping, `\'hh`
//! hex escapes decoded with the document code page, hyperlink fields, `\info` metadata (title,
//! author, subject, keywords), headings from `\outlinelevelN` or a `heading N` / `Title` style.
//! Ignored: fonts, colours, headers/footers, pictures, lists (list markers appear as text),
//! tables (their text becomes paragraphs) and footnotes.

use std::collections::HashMap;

use anyhow::{Result, bail};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::opf::Metadata;
use bookx_core::util::xml_escape;

use crate::common::*;

const MAX_DEPTH: usize = 2000;

/// Reader for `.rtf` files.
pub struct RtfReader;

impl FormatReader for RtfReader {
    fn id(&self) -> &'static str {
        "rtf"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["rtf"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(b"{\\rtf")
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_rtf(bytes)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dest {
    Body,
    Info,
    Title,
    Author,
    Subject,
    Keywords,
    StyleSheet,
    FieldInst,
}

#[derive(Clone)]
struct State {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    sup: bool,
    sub: bool,
    skip: bool,
    uc: usize,
    dest: Dest,
    link: Option<String>,
}

#[derive(Clone, PartialEq)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    sup: bool,
    sub: bool,
    link: Option<String>,
}

const SKIP_DESTS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "pict",
    "header",
    "footer",
    "headerl",
    "headerr",
    "headerf",
    "footerl",
    "footerr",
    "footerf",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "themedata",
    "colorschememapping",
    "datastore",
    "latentstyles",
    "xmlnstbl",
    "mmath",
    "mmathPr",
    "object",
    "objdata",
    "footnote",
    "bkmkstart",
    "bkmkend",
    "revtbl",
    "filetbl",
    "pgptbl",
    "private",
    "userprops",
    "pnseclvl",
    "template",
    "stylesheet_skip",
];

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Start,
    Center,
    End,
}

struct Parser {
    cur: State,
    stack: Vec<State>,
    codepage: &'static encoding_rs::Encoding,
    hex: Vec<u8>,
    skip_count: usize,
    high: Option<u16>,
    star: bool,
    runs: Vec<Run>,
    blocks: Vec<String>,
    in_table: bool,
    cells: Vec<String>,
    rows: Vec<Vec<String>>,
    style_names: HashMap<i32, String>,
    para_style: Option<i32>,
    outline: Option<i32>,
    align: Align,
    style_id: Option<i32>,
    style_buf: String,
    inst: String,
    pending_link: Option<String>,
    title: String,
    author: String,
    subject: String,
    keywords: String,
}

fn codepage_encoding(cp: i32) -> &'static encoding_rs::Encoding {
    let label = match cp {
        932 => "shift_jis".to_string(),
        936 => "gbk".to_string(),
        949 => "euc-kr".to_string(),
        950 => "big5".to_string(),
        10000 => "macintosh".to_string(),
        65001 => "utf-8".to_string(),
        874 => "windows-874".to_string(),
        1250..=1258 => format!("windows-{cp}"),
        _ => "windows-1252".to_string(),
    };
    encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252)
}

impl Parser {
    fn new() -> Self {
        Parser {
            cur: State {
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                sup: false,
                sub: false,
                skip: false,
                uc: 1,
                dest: Dest::Body,
                link: None,
            },
            stack: Vec::new(),
            codepage: encoding_rs::WINDOWS_1252,
            hex: Vec::new(),
            skip_count: 0,
            high: None,
            star: false,
            runs: Vec::new(),
            blocks: Vec::new(),
            in_table: false,
            cells: Vec::new(),
            rows: Vec::new(),
            style_names: HashMap::new(),
            para_style: None,
            outline: None,
            align: Align::Start,
            style_id: None,
            style_buf: String::new(),
            inst: String::new(),
            pending_link: None,
            title: String::new(),
            author: String::new(),
            subject: String::new(),
            keywords: String::new(),
        }
    }

    fn flush_hex(&mut self) {
        if self.hex.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.hex);
        let (s, _, _) = self.codepage.decode(&bytes);
        let s = s.into_owned();
        for c in s.chars() {
            self.put_char(c);
        }
    }

    fn put_char(&mut self, c: char) {
        if self.skip_count > 0 {
            self.skip_count -= 1;
            return;
        }
        if self.cur.skip {
            return;
        }
        match self.cur.dest {
            Dest::Body => self.push_text(c),
            Dest::Title => self.title.push(c),
            Dest::Author => self.author.push(c),
            Dest::Subject => self.subject.push(c),
            Dest::Keywords => self.keywords.push(c),
            Dest::FieldInst => self.inst.push(c),
            Dest::StyleSheet => {
                if c == ';' {
                    if let Some(id) = self.style_id.take() {
                        self.style_names
                            .insert(id, self.style_buf.trim().to_string());
                    }
                    self.style_buf.clear();
                } else {
                    self.style_buf.push(c);
                }
            }
            Dest::Info => {}
        }
    }

    fn push_text(&mut self, c: char) {
        let s = &self.cur;
        let same = self.runs.last().is_some_and(|r| {
            r.bold == s.bold
                && r.italic == s.italic
                && r.underline == s.underline
                && r.strike == s.strike
                && r.sup == s.sup
                && r.sub == s.sub
                && r.link == s.link
        });
        if same {
            if let Some(r) = self.runs.last_mut() {
                r.text.push(c);
            }
        } else {
            self.runs.push(Run {
                text: c.to_string(),
                bold: s.bold,
                italic: s.italic,
                underline: s.underline,
                strike: s.strike,
                sup: s.sup,
                sub: s.sub,
                link: s.link.clone(),
            });
        }
    }

    fn put_str(&mut self, s: &str) {
        for c in s.chars() {
            self.put_char(c);
        }
    }

    fn render_runs(runs: &[Run]) -> String {
        let mut inner = String::new();
        for r in runs {
            let mut t = xml_escape(&r.text).replace('\n', "<br/>");
            for (on, tag) in [
                (r.sub, "sub"),
                (r.sup, "sup"),
                (r.strike, "del"),
                (r.underline, "u"),
                (r.italic, "em"),
                (r.bold, "strong"),
            ] {
                if on {
                    t = format!("<{tag}>{t}</{tag}>");
                }
            }
            if let Some(l) = &r.link {
                t = format!("<a href=\"{}\">{t}</a>", xml_escape(l));
            }
            inner.push_str(&t);
        }
        inner
    }

    /// `\cell`: the text so far is one table cell.
    fn end_cell(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        self.cells.push(Self::render_runs(&runs).trim().to_string());
    }

    /// `\row`: the cells so far are one table row.
    fn end_row(&mut self) {
        if !self.runs.is_empty() {
            self.end_cell();
        }
        if !self.cells.is_empty() {
            self.rows.push(std::mem::take(&mut self.cells));
        }
    }

    fn flush_table(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let mut t = String::from("<table>\n");
        for row in std::mem::take(&mut self.rows) {
            t.push_str("<tr>");
            for c in row {
                t.push_str(&format!("<td>{c}</td>"));
            }
            t.push_str("</tr>\n");
        }
        t.push_str("</table>\n");
        self.blocks.push(t);
    }

    fn end_paragraph(&mut self) {
        self.flush_table();
        let runs = std::mem::take(&mut self.runs);
        let level = self.heading_level();
        self.para_style = None;
        self.outline = None;
        let align = std::mem::replace(&mut self.align, Align::Start);
        let all_blank = runs.iter().all(|r| r.text.trim().is_empty());
        if all_blank {
            return;
        }
        let inner = Self::render_runs(&runs);
        let style = match align {
            Align::Center => " style=\"text-align:center\"",
            Align::End => " style=\"text-align:right\"",
            Align::Start => "",
        };
        match level {
            Some(l) => self
                .blocks
                .push(format!("<h{l}{style}>{}</h{l}>\n", inner.trim())),
            None => self.blocks.push(format!("<p{style}>{inner}</p>\n")),
        }
    }

    fn heading_level(&self) -> Option<usize> {
        if let Some(o) = self.outline {
            if (0..9).contains(&o) {
                return Some((o as usize + 1).min(6));
            }
        }
        let name = self
            .style_names
            .get(&self.para_style?)?
            .to_ascii_lowercase();
        if name == "title" {
            return Some(1);
        }
        let n = name
            .strip_prefix("heading ")?
            .trim()
            .parse::<usize>()
            .ok()?;
        (1..=9).contains(&n).then_some(n.min(6))
    }

    fn control(&mut self, word: &str, param: Option<i32>) {
        let star = std::mem::take(&mut self.star);
        if star {
            if word == "fldinst" {
                self.cur.dest = Dest::FieldInst;
                self.inst.clear();
            } else {
                self.cur.skip = true;
            }
            return;
        }
        let on = param != Some(0);
        match word {
            "ansicpg" => {
                if let Some(p) = param {
                    self.codepage = codepage_encoding(p);
                }
            }
            "info" => self.cur.dest = Dest::Info,
            "stylesheet" => self.cur.dest = Dest::StyleSheet,
            "title" if self.cur.dest == Dest::Info => self.cur.dest = Dest::Title,
            "author" if self.cur.dest == Dest::Info => self.cur.dest = Dest::Author,
            "subject" if self.cur.dest == Dest::Info => self.cur.dest = Dest::Subject,
            "keywords" if self.cur.dest == Dest::Info => self.cur.dest = Dest::Keywords,
            "s" if self.cur.dest == Dest::StyleSheet => self.style_id = param,
            "fldrslt" => self.cur.link = self.pending_link.take(),
            w if SKIP_DESTS.contains(&w) => self.cur.skip = true,
            _ if self.cur.skip || self.cur.dest != Dest::Body => {
                if word == "u" {
                    self.unicode(param);
                }
            }
            "par" | "sect" => {
                if !self.in_table {
                    self.end_paragraph()
                } else {
                    // a paragraph mark inside a table cell separates its lines
                    self.put_char('\n');
                }
            }
            "trowd" | "intbl" => self.in_table = true,
            "cell" | "nestcell" => self.end_cell(),
            "row" | "nestrow" => {
                self.end_row();
                self.in_table = false;
            }
            "pard" => {
                self.para_style = None;
                self.outline = None;
                self.align = Align::Start;
            }
            "line" => self.put_char('\n'),
            "tab" => self.put_char(' '),
            "page" | "column" => {}
            "b" => self.cur.bold = on,
            "i" => self.cur.italic = on,
            "ul" | "uld" | "uldb" | "ulw" => self.cur.underline = on,
            "ulnone" => self.cur.underline = false,
            "strike" | "striked" => self.cur.strike = on,
            "super" => self.cur.sup = on,
            "sub" => self.cur.sub = on,
            "nosupersub" => {
                self.cur.sup = false;
                self.cur.sub = false;
            }
            "plain" => {
                self.cur.bold = false;
                self.cur.italic = false;
                self.cur.underline = false;
                self.cur.strike = false;
                self.cur.sup = false;
                self.cur.sub = false;
            }
            "uc" => self.cur.uc = param.unwrap_or(1).clamp(0, 16) as usize,
            "u" => self.unicode(param),
            "s" => self.para_style = param,
            "outlinelevel" => self.outline = param,
            "qc" => self.align = Align::Center,
            "qr" => self.align = Align::End,
            "ql" | "qj" => self.align = Align::Start,
            "emdash" => self.put_char('\u{2014}'),
            "endash" => self.put_char('\u{2013}'),
            "emspace" | "enspace" | "qmspace" => self.put_char('\u{2003}'),
            "lquote" => self.put_char('\u{2018}'),
            "rquote" => self.put_char('\u{2019}'),
            "ldblquote" => self.put_char('\u{201C}'),
            "rdblquote" => self.put_char('\u{201D}'),
            "bullet" => self.put_char('\u{2022}'),
            _ => {}
        }
    }

    fn unicode(&mut self, param: Option<i32>) {
        let Some(mut v) = param else { return };
        if v < 0 {
            v += 65536;
        }
        let v = v.clamp(0, 65535) as u16;
        let uc = self.cur.uc;
        match v {
            0xD800..=0xDBFF => {
                self.high = Some(v);
            }
            0xDC00..=0xDFFF => {
                if let Some(h) = self.high.take() {
                    let c = 0x10000 + (((h as u32) - 0xD800) << 10) + ((v as u32) - 0xDC00);
                    if let Some(ch) = char::from_u32(c) {
                        self.put_char(ch);
                    }
                }
            }
            _ => {
                if let Some(c) = char::from_u32(v as u32) {
                    self.put_char(c);
                }
            }
        }
        self.skip_count = uc;
    }

    fn end_group(&mut self) {
        let closed = self.cur.clone();
        if let Some(prev) = self.stack.pop() {
            if closed.dest == Dest::FieldInst && prev.dest != Dest::FieldInst {
                self.pending_link = parse_hyperlink(&self.inst);
                self.inst.clear();
            }
            if closed.dest == Dest::StyleSheet && prev.dest != Dest::StyleSheet {
                self.style_id = None;
            }
            self.cur = prev;
        }
    }
}

fn parse_hyperlink(inst: &str) -> Option<String> {
    let t = inst.trim();
    let rest = t
        .strip_prefix("HYPERLINK")
        .or_else(|| t.strip_prefix("hyperlink"))?;
    let rest = rest.trim();
    if rest.starts_with("\\l") {
        return None;
    }
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    let url = rest[start..end].trim();
    (!url.is_empty()).then(|| url.to_string())
}

/// Converts RTF bytes to an EPUB archive.
pub fn convert_rtf(bytes: &[u8]) -> Result<Archive> {
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if !b.starts_with(b"{\\rtf") {
        bail!("not an RTF document");
    }
    let mut p = Parser::new();
    let mut i = 0;
    let n = b.len();
    while i < n {
        let c = b[i];
        match c {
            b'{' => {
                p.flush_hex();
                if p.stack.len() >= MAX_DEPTH {
                    bail!("RTF groups are nested too deeply");
                }
                p.stack.push(p.cur.clone());
                i += 1;
            }
            b'}' => {
                p.flush_hex();
                p.star = false;
                p.end_group();
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= n {
                    break;
                }
                let d = b[i];
                if d.is_ascii_alphabetic() {
                    p.flush_hex();
                    let start = i;
                    while i < n && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = String::from_utf8_lossy(&b[start..i]).into_owned();
                    let mut param = None;
                    let mut j = i;
                    let neg = j < n && b[j] == b'-';
                    if neg {
                        j += 1;
                    }
                    let ds = j;
                    while j < n && b[j].is_ascii_digit() && j - ds < 10 {
                        j += 1;
                    }
                    if j > ds {
                        let v: i64 = std::str::from_utf8(&b[ds..j])
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        param = Some(
                            (if neg { -v } else { v }).clamp(-1_000_000_000, 1_000_000_000) as i32,
                        );
                        i = j;
                    }
                    if i < n && b[i] == b' ' {
                        i += 1;
                    }
                    p.control(&word, param);
                } else {
                    i += 1;
                    match d {
                        b'\'' => {
                            let hex = b.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok());
                            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                                i += 2;
                                if p.skip_count > 0 {
                                    p.skip_count -= 1;
                                } else {
                                    p.hex.push(v);
                                }
                            }
                        }
                        b'*' => {
                            p.flush_hex();
                            p.star = true;
                        }
                        b'\\' | b'{' | b'}' => {
                            p.flush_hex();
                            p.put_char(d as char);
                        }
                        b'~' => {
                            p.flush_hex();
                            p.put_char('\u{a0}');
                        }
                        b'_' => {
                            p.flush_hex();
                            p.put_char('\u{2011}');
                        }
                        b'-' => {}
                        b'\r' | b'\n' => {
                            p.flush_hex();
                            p.end_paragraph();
                        }
                        _ => {}
                    }
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                p.flush_hex();
                // Plain bytes are in the document code page.
                let start = i;
                while i < n && !matches!(b[i], b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                    i += 1;
                }
                let (s, _, _) = p.codepage.decode(&b[start..i]);
                let s = s.into_owned();
                p.put_str(&s);
            }
        }
    }
    p.flush_hex();
    p.end_paragraph();
    if p.blocks.is_empty() {
        bail!("the RTF document has no text");
    }
    let clean = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let md = Metadata {
        title: clean(&p.title),
        authors: clean(&p.author)
            .split(';')
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect(),
        description: clean(&p.subject),
        tags: clean(&p.keywords)
            .split([',', ';'])
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect(),
        ..Default::default()
    };
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let chapters = chapters_from_blocks(p.blocks, &default_title);
    build(Parts {
        md,
        chapters,
        ..Default::default()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testutil::*;

    pub fn sample() -> Vec<u8> {
        br#"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0 Times;}}{\colortbl;\red0\green0\blue0;}
{\stylesheet{\s0 Normal;}{\s1 heading 1;}{\s2 Title;}}
{\info{\title Sample \'e9 Book}{\author Ann Author; Bob B.}{\subject A subject}{\keywords k1, k2}}
\pard\s1 Chapter One\par
\pard Plain \b bold\b0  \i italic\i0  \ul under\ulnone  \strike gone\strike0  H\sub 2\nosupersub O x\super 2\nosupersub\par
\pard\qc Centered\par
\pard\qr Right\line break\par
\pard Caf\'e9 \u8364?  \u-10179?\u-8704? \u233\'e9x \'93q\'94 \~nb \emdash\endash\lquote\rquote\ldblquote\rdblquote\bullet\par
\pard \{braces\} \\ back\par
{\field{\*\fldinst HYPERLINK "http://example.com/a?b=1&c=2"}{\fldrslt {\ul link text}}} after\par
{\*\unknown ignored text}{\pict\wmetafile8 deadbeef}
\pard\s1 Chapter Two\par
\pard\outlinelevel1 Sub heading\par
\pard Body of two.\par
\pard \par
}"#
        .to_vec()
    }

    #[test]
    fn metadata_comes_from_info() {
        let book = validate(&convert_rtf(&sample()).unwrap());
        let md = book.metadata();
        assert_eq!(md.title, "Sample é Book");
        assert_eq!(md.authors, vec!["Ann Author", "Bob B."]);
        assert_eq!(md.description, "A subject");
        assert_eq!(md.tags, vec!["k1", "k2"]);
    }

    #[test]
    fn headings_split_chapters() {
        let book = validate(&convert_rtf(&sample()).unwrap());
        assert_eq!(
            toc_titles(&book),
            vec!["Chapter One", "Chapter Two", "Sub heading"]
        );
        assert!(chapter_xml(&book, 0).contains("<h1>Chapter One</h1>"));
        assert!(chapter_xml(&book, 1).contains(">Sub heading</h2>"));
        assert!(chapter_texts(&book)[1].contains("Body of two."));
    }

    #[test]
    fn character_formatting_is_kept() {
        let book = validate(&convert_rtf(&sample()).unwrap());
        let x = chapter_xml(&book, 0);
        assert!(x.contains("<strong>bold</strong>"), "{x}");
        assert!(x.contains("<em>italic</em>"));
        assert!(x.contains("<u>under</u>"));
        assert!(x.contains("<del>gone</del>"));
        assert!(x.contains("H<sub>2</sub>O"));
        assert!(x.contains("x<sup>2</sup>"));
        assert!(x.contains("<p style=\"text-align:center\">Centered</p>"));
        assert!(x.contains("<p style=\"text-align:right\">Right<br/>break</p>"));
    }

    #[test]
    fn escapes_and_codepages() {
        let book = validate(&convert_rtf(&sample()).unwrap());
        let t = chapter_texts(&book)[0].clone();
        assert!(t.contains("Café €"), "{t}");
        assert!(t.contains("\u{1F600}"), "surrogate pair: {t}");
        assert!(t.contains("éx"), "\\u233 with fallback skipped: {t}");
        assert!(t.contains("“q”"), "{t}");
        assert!(t.contains("{braces} \\ back"));
        assert!(t.contains("\u{a0}nb"));
        assert!(t.contains("—–‘’“”•"));
    }

    #[test]
    fn surrogate_pairs_become_one_char() {
        // U+1F600 = D83D DE00
        let rtf = br"{\rtf1\ansi\pard Smile \u-10179?\u-8704? done\par}";
        let book = validate(&convert_rtf(rtf).unwrap());
        assert!(
            chapter_texts(&book)[0].contains("Smile \u{1F600} done"),
            "{:?}",
            chapter_texts(&book)
        );
    }

    #[test]
    fn hyperlinks_and_ignored_destinations() {
        let book = validate(&convert_rtf(&sample()).unwrap());
        let x = chapter_xml(&book, 0);
        assert!(
            x.contains("<a href=\"http://example.com/a?b=1&amp;c=2\"><u>link text</u></a> after"),
            "{x}"
        );
        let all = chapter_texts(&book).join(" ");
        assert!(
            !all.contains("ignored text") && !all.contains("deadbeef") && !all.contains("Times")
        );
    }

    #[test]
    fn other_codepages() {
        let rtf = br"{\rtf1\ansi\ansicpg1251 \pard \'cf\'f0\'e8\'e2\'e5\'f2\par}";
        let book = validate(&convert_rtf(rtf).unwrap());
        assert!(chapter_texts(&book)[0].contains("Привет"));
        let rtf = br"{\rtf1\ansi\ansicpg932 \pard \'93\'fa\'96\'7b\par}";
        let book = validate(&convert_rtf(rtf).unwrap());
        assert!(chapter_texts(&book)[0].contains("日本"));
        assert_eq!(codepage_encoding(1253), encoding_rs::WINDOWS_1253);
        assert_eq!(codepage_encoding(936), encoding_rs::GBK);
        assert_eq!(codepage_encoding(949), encoding_rs::EUC_KR);
        assert_eq!(codepage_encoding(950), encoding_rs::BIG5);
        assert_eq!(codepage_encoding(65001), encoding_rs::UTF_8);
        assert_eq!(codepage_encoding(10000), encoding_rs::MACINTOSH);
        assert_eq!(codepage_encoding(1), encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn plain_bytes_use_the_codepage_and_no_title_defaults() {
        let rtf = b"{\\rtf1\\ansi\\ansicpg1252 caf\xe9 one\\par}";
        let book = validate(&convert_rtf(rtf).unwrap());
        assert!(chapter_texts(&book)[0].contains("café one"));
        assert_eq!(book.metadata().title, "Untitled");
        assert_eq!(toc_titles(&book), vec!["Beginning"]);
    }

    #[test]
    fn plain_resets_formatting_and_title_style() {
        let rtf =
            br"{\rtf1{\stylesheet{\s5 Title;}}\pard\s5 Big Title\par \b\i bi\plain  plain\par}";
        let book = validate(&convert_rtf(rtf).unwrap());
        let x = chapter_xml(&book, 0);
        assert!(x.contains("<h1>Big Title</h1>"));
        assert!(x.contains("<strong><em>bi</em></strong> plain"), "{x}");
    }

    #[test]
    fn hyperlink_instruction_parsing() {
        assert_eq!(
            parse_hyperlink(" HYPERLINK \"http://a.b/c\" "),
            Some("http://a.b/c".into())
        );
        assert_eq!(parse_hyperlink("HYPERLINK \\l \"bookmark\""), None);
        assert_eq!(parse_hyperlink("PAGE"), None);
        assert_eq!(parse_hyperlink("HYPERLINK \"\""), None);
        assert_eq!(parse_hyperlink("HYPERLINK noquote"), None);
    }

    #[test]
    fn errors() {
        assert!(
            convert_rtf(b"")
                .unwrap_err()
                .to_string()
                .contains("not an RTF")
        );
        assert!(convert_rtf(b"plain text").is_err());
        assert!(
            convert_rtf(br"{\rtf1\ansi {\fonttbl{\f0 Times;}}}")
                .unwrap_err()
                .to_string()
                .contains("no text")
        );
        let deep = format!("{{\\rtf1{}x", "{".repeat(5000));
        assert!(
            convert_rtf(deep.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("nested")
        );
        // truncated in the middle of escapes must not panic
        for cut in 1..40 {
            let s = sample();
            let _ = convert_rtf(&s[..cut.min(s.len())]);
        }
        assert!(convert_rtf(b"{\\rtf1 text \\'4").is_ok());
    }

    #[test]
    fn bom_is_tolerated_and_sniffing_works() {
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend(sample());
        assert!(convert_rtf(&v).is_ok());
        assert!(RtfReader.sniff(&sample()));
        assert!(!RtfReader.sniff(b"hello"));
        assert!(!RtfReader.sniff(b""));
        assert_eq!(RtfReader.id(), "rtf");
        assert_eq!(RtfReader.extensions(), &["rtf"]);
    }

    #[test]
    fn robust_against_garbage_and_mutations() {
        assert_rejects_garbage(&RtfReader);
        let errors = fuzz_no_panic(&RtfReader, &sample());
        let _ = errors;
    }
}
