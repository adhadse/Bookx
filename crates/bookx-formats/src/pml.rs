//! PML / PMLZ (Palm Markup Language) input and the PML renderer shared with the eReader PDB
//! reader.
//!
//! Supported codes: `\p` `\x` `\Xn` `\Cn="t"` (chapters become `<h1>`..`<h5>`), `\c` `\r` `\t`
//! (alignment / indent blocks), `\i` `\u` `\o` `\b` `\B` `\l` `\k` `\Sp` `\Sb` (inline styles),
//! `\v` (invisible text; `TITLE="" AUTHOR="" PUBLISHER="" COPYRIGHT="" EISBN=""` inside it is
//! read as metadata), `\\`, `\aNNN`, `\UXXXX`, `\-`, `\m="img"`, `\q="#a"text\q`, `\Q="a"`,
//! `\Fn="id"text\Fn`, `\Sd="id"text\Sd`, `\w="50%"`, `\T="50%"`, `\n` `\s` `\I` (accepted, no
//! visible effect). Footnotes and sidebars come from `<footnote id="">` / `<sidebar id="">`
//! blocks at the end of the text (or the eReader PDB records).

use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::{bail, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::convert::Chapter;
use bookx_core::opf::Metadata;
use bookx_core::util::xml_escape;
use regex::Regex;

use crate::common::*;

type ImageLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Stylesheet appended to books produced from PML.
pub const CSS: &str = ".center { text-align: center; }\n.right { text-align: right; }\n.indent { margin-left: 2em; }\n.pml-page { page-break-after: always; }\n.pml-marker { display: none; }\n.smallcaps { font-variant: small-caps; }\n";

/// Reader for plain `.pml` files.
pub struct PmlReader;
/// Reader for `.pmlz` archives (a zip with the `.pml`, its images and optional extras).
pub struct PmlzReader;

impl FormatReader for PmlReader {
    fn id(&self) -> &'static str {
        "pml"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["pml"]
    }
    fn sniff(&self, _head: &[u8]) -> bool {
        false
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_pml(bytes, "")
    }
}

impl FormatReader for PmlzReader {
    fn id(&self) -> &'static str {
        "pmlz"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["pmlz"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        zip_names(head).is_some_and(|n| n.iter().any(|f| lower_ext(f) == "pml"))
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_pmlz(bytes)
    }
}

fn lower_ext(name: &str) -> String {
    name.rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Images available to a PML text, by lower case file name.
#[derive(Default, Clone)]
pub struct ImageStore {
    files: HashMap<String, Vec<u8>>,
}

impl ImageStore {
    pub fn insert(&mut self, name: &str, data: Vec<u8>) {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        self.files.entry(base.to_ascii_lowercase()).or_insert(data);
    }

    /// Looks `name` up, also trying with a `.png` extension.
    pub fn get(&self, name: &str) -> Option<(&str, &[u8])> {
        let base = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(name)
            .to_ascii_lowercase();
        for key in [base.clone(), format!("{base}.png")] {
            if let Some((k, v)) = self.files.get_key_value(&key) {
                return Some((k.as_str(), v.as_slice()));
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// The rendered PML.
#[derive(Debug, Default)]
pub struct Rendered {
    /// One XHTML block per paragraph / heading.
    pub blocks: Vec<String>,
    /// `KEY="value"` pairs found in invisible (`\v`) text, keys upper case.
    pub meta: Vec<(String, String)>,
    /// Footnote and sidebar asides referenced from the text, `(id, xhtml)`.
    pub notes: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Default, Debug)]
struct Style {
    italic: bool,
    bold: bool,
    underline: bool,
    strike: bool,
    large: bool,
    small_caps: bool,
    sup: bool,
    sub: bool,
    link: Option<String>,
}

impl Style {
    fn wrap(&self, text: String) -> String {
        let mut s = text;
        let tags: [(bool, &str); 8] = [
            (self.italic, "i"),
            (self.bold, "b"),
            (self.underline, "u"),
            (self.strike, "s"),
            (self.large, "big"),
            (self.sup, "sup"),
            (self.sub, "sub"),
            (self.small_caps, "span class=\"smallcaps\""),
        ];
        for (on, tag) in tags {
            if on {
                let name = tag.split(' ').next().unwrap_or(tag);
                s = format!("<{tag}>{s}</{name}>");
            }
        }
        if let Some(href) = &self.link {
            s = format!("<a href=\"{}\">{s}</a>", xml_escape(href));
        }
        s
    }
}

enum Run {
    Text(Style, String),
    Raw(String),
}

#[derive(Default)]
struct Renderer<'a> {
    kinds: Vec<(String, String)>,
    notes_src: Option<&'a HashMap<String, String>>,
    sidebars_src: Option<&'a HashMap<String, String>>,
    allow_notes: bool,
    images: Option<ImageLookup<'a>>,
    style: Style,
    center: bool,
    right: bool,
    indent: bool,
    hidden: bool,
    hidden_buf: String,
    head_open: bool,
    line_head: Option<u8>,
    note: Option<(String, String, String)>,
    runs: Vec<Run>,
    out: Rendered,
    page_break_pending: bool,
    para_cls: (bool, bool, bool),
}

/// Anchor / footnote ids restricted to characters safe in `id` and `href`.
fn safe_id(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("pml-{t}")
}

fn heading_level(n: u8) -> u8 {
    if n == 0 {
        1
    } else {
        (n + 1).min(5)
    }
}

impl<'a> Renderer<'a> {
    fn push_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        if self.hidden {
            self.hidden_buf.push_str(s);
            return;
        }
        if let Some((_, _, label)) = self.note.as_mut() {
            label.push_str(s);
            return;
        }
        self.snapshot_alignment();
        let esc = xml_escape(s);
        match self.runs.last_mut() {
            Some(Run::Text(st, t)) if *st == self.style => t.push_str(&esc),
            _ => self.runs.push(Run::Text(self.style.clone(), esc)),
        }
    }

    fn push_raw(&mut self, html: String) {
        if !self.hidden {
            self.snapshot_alignment();
            self.runs.push(Run::Raw(html));
        }
    }

    fn snapshot_alignment(&mut self) {
        if self.runs.is_empty() {
            self.para_cls = (self.center, self.right, self.indent);
        }
    }

    fn flush_para(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        let head = self.line_head.take();
        let has_raw = runs.iter().any(|r| matches!(r, Run::Raw(_)));
        let mut inner = String::new();
        for r in runs {
            match r {
                Run::Text(st, t) => inner.push_str(&st.wrap(t)),
                Run::Raw(h) => inner.push_str(&h),
            }
        }
        if self.page_break_pending {
            self.page_break_pending = false;
            self.out
                .blocks
                .push("<div class=\"pml-page\"></div>".into());
        }
        if !has_raw && inner.trim().is_empty() {
            return;
        }
        let inner = inner.trim().to_string();
        if let Some(n) = head {
            let l = heading_level(n);
            self.out.blocks.push(format!("<h{l}>{inner}</h{l}>"));
            return;
        }
        let (center, right, indent) = self.para_cls;
        let mut classes = Vec::new();
        if center {
            classes.push("center");
        }
        if right {
            classes.push("right");
        }
        if indent {
            classes.push("indent");
        }
        if classes.is_empty() {
            self.out.blocks.push(format!("<p>{inner}</p>"));
        } else {
            self.out
                .blocks
                .push(format!("<p class=\"{}\">{inner}</p>", classes.join(" ")));
        }
    }

    fn end_hidden(&mut self) {
        let buf = std::mem::take(&mut self.hidden_buf);
        let re = Regex::new(r#"([A-Za-z]+)\s*=\s*"([^"]*)""#).expect("static regex");
        for c in re.captures_iter(&buf) {
            self.out
                .meta
                .push((c[1].to_ascii_uppercase(), c[2].to_string()));
        }
    }

    fn open_note(&mut self, kind: &str, id: String) {
        if self.allow_notes && !self.hidden {
            self.note = Some((kind.to_string(), id, String::new()));
        } else {
            // Notes inside notes (or hidden text) are shown as plain text.
            self.note = None;
        }
    }

    fn close_note(&mut self) {
        let Some((kind, id, label)) = self.note.take() else {
            return;
        };
        let src = if kind == "fn" {
            self.notes_src
        } else {
            self.sidebars_src
        };
        let Some(body) = src.and_then(|m| m.get(&id)) else {
            self.push_text(&label);
            return;
        };
        let aside_id = format!(
            "{}-{}",
            if kind == "fn" { "fn" } else { "sb" },
            safe_id(&id)
        );
        if !self.kinds.iter().any(|(k, _)| *k == aside_id) {
            let mut sub = Renderer {
                images: self.images,
                ..Default::default()
            };
            sub.run(body);
            let inner: String = sub.out.blocks.iter().map(|b| format!("{b}\n")).collect();
            self.kinds.push((aside_id.clone(), inner));
        }
        self.push_raw(noteref(&aside_id, &label));
    }

    fn run(&mut self, text: &str) {
        let c: Vec<char> = text.chars().collect();
        let mut i = 0usize;
        let mut buf = String::new();
        macro_rules! flush_buf {
            () => {
                if !buf.is_empty() {
                    let s = std::mem::take(&mut buf);
                    self.push_text(&s);
                }
            };
        }
        while i < c.len() {
            let ch = c[i];
            if ch == '\n' || ch == '\r' {
                flush_buf!();
                if ch == '\r' && c.get(i + 1) == Some(&'\n') {
                    i += 1;
                }
                self.flush_para();
                self.head_open = false;
                i += 1;
                continue;
            }
            if ch != '\\' {
                buf.push(ch);
                i += 1;
                continue;
            }
            let Some(&code) = c.get(i + 1) else {
                break;
            };
            i += 2;
            // Codes that can start a chapter or change the structure flush text first.
            match code {
                '\\' => buf.push('\\'),
                '-' => buf.push('\u{ad}'),
                'a' => {
                    let digits: String = c[i..]
                        .iter()
                        .take(3)
                        .take_while(|d| d.is_ascii_digit())
                        .collect();
                    i += digits.len();
                    if let Ok(n) = digits.parse::<u32>() {
                        if (1..=255).contains(&n) {
                            let b = [n as u8];
                            buf.push_str(
                                &encoding_rs::WINDOWS_1252.decode_without_bom_handling(&b).0,
                            );
                        }
                    }
                }
                'U' => {
                    let hex: String = c[i..]
                        .iter()
                        .take(4)
                        .take_while(|d| d.is_ascii_hexdigit())
                        .collect();
                    i += hex.len();
                    if let Some(u) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        buf.push(u);
                    }
                }
                'p' => {
                    flush_buf!();
                    self.flush_para();
                    self.page_break_pending = true;
                }
                'x' | 'X' => {
                    flush_buf!();
                    let level = if code == 'x' {
                        0
                    } else if let Some(d) = c.get(i).and_then(|d| d.to_digit(10)) {
                        i += 1;
                        d.min(4) as u8
                    } else {
                        0
                    };
                    if self.head_open {
                        self.head_open = false;
                    } else {
                        self.head_open = true;
                        self.line_head = Some(level);
                    }
                }
                'C' => {
                    flush_buf!();
                    let level = if let Some(d) = c.get(i).and_then(|d| d.to_digit(10)) {
                        i += 1;
                        d.min(4) as u8
                    } else {
                        0
                    };
                    if let Some(title) = param(&c, &mut i) {
                        self.flush_para();
                        let l = heading_level(level);
                        self.out.blocks.push(format!(
                            "<h{l} class=\"pml-marker\">{}</h{l}>",
                            xml_escape(&title)
                        ));
                    }
                }
                'c' => {
                    flush_buf!();
                    self.center = !self.center;
                }
                'r' => {
                    flush_buf!();
                    self.right = !self.right;
                }
                't' => {
                    flush_buf!();
                    self.indent = !self.indent;
                }
                'T' => {
                    param(&c, &mut i);
                }
                'w' => {
                    param(&c, &mut i);
                    flush_buf!();
                    self.flush_para();
                    self.out.blocks.push("<hr/>".into());
                }
                'i' => {
                    flush_buf!();
                    self.style.italic = !self.style.italic;
                }
                'u' => {
                    flush_buf!();
                    self.style.underline = !self.style.underline;
                }
                'o' => {
                    flush_buf!();
                    self.style.strike = !self.style.strike;
                }
                'b' | 'B' => {
                    flush_buf!();
                    self.style.bold = !self.style.bold;
                }
                'l' => {
                    flush_buf!();
                    self.style.large = !self.style.large;
                }
                'k' => {
                    flush_buf!();
                    self.style.small_caps = !self.style.small_caps;
                }
                'n' => {
                    flush_buf!();
                    self.style.bold = false;
                    self.style.large = false;
                }
                's' | 'I' => {}
                'v' => {
                    flush_buf!();
                    if self.hidden {
                        self.hidden = false;
                        self.end_hidden();
                    } else {
                        self.hidden = true;
                    }
                }
                'S' => {
                    flush_buf!();
                    match c.get(i) {
                        Some('p') => {
                            i += 1;
                            self.style.sup = !self.style.sup;
                        }
                        Some('b') => {
                            i += 1;
                            self.style.sub = !self.style.sub;
                        }
                        Some('d') => {
                            i += 1;
                            if let Some(id) = param(&c, &mut i) {
                                self.open_note("sb", id);
                            } else {
                                self.close_note();
                            }
                        }
                        _ => {}
                    }
                }
                'F' => {
                    flush_buf!();
                    if c.get(i) == Some(&'n') {
                        i += 1;
                        if let Some(id) = param(&c, &mut i) {
                            self.open_note("fn", id);
                        } else {
                            self.close_note();
                        }
                    }
                }
                'm' => {
                    flush_buf!();
                    if let Some(name) = param(&c, &mut i) {
                        if let Some(path) = self.images.and_then(|f| f(&name)) {
                            self.push_raw(format!("<img src=\"{}\" alt=\"\"/>", xml_escape(&path)));
                        }
                    }
                }
                'q' => {
                    flush_buf!();
                    if let Some(target) = param(&c, &mut i) {
                        let t = target.trim_start_matches('#');
                        self.style.link = Some(format!("#{}", safe_id(t)));
                    } else {
                        self.style.link = None;
                    }
                }
                'Q' => {
                    flush_buf!();
                    if let Some(name) = param(&c, &mut i) {
                        self.push_raw(format!("<a id=\"{}\"></a>", safe_id(&name)));
                    }
                }
                _ => {}
            }
        }
        flush_buf!();
        if self.hidden {
            self.hidden = false;
            self.end_hidden();
        }
        if self.note.is_some() {
            self.close_note();
        }
        self.flush_para();
    }
}

/// Reads `="value"` at `*i`, advancing past it.
fn param(c: &[char], i: &mut usize) -> Option<String> {
    if c.get(*i) != Some(&'=') || c.get(*i + 1) != Some(&'"') {
        return None;
    }
    let start = *i + 2;
    let end = c[start..]
        .iter()
        .position(|ch| *ch == '"')
        .map(|p| p + start)?;
    *i = end + 1;
    Some(c[start..end].iter().collect())
}

/// Splits `<footnote id="x">..</footnote>` / `<sidebar ..>` blocks from the end of a PML text.
pub fn split_notes(text: &str) -> (String, HashMap<String, String>, HashMap<String, String>) {
    let re =
        Regex::new(r#"(?s)<(footnote|sidebar)\s+id="([^"]*)"\s*>(.*?)</(?:footnote|sidebar)>"#)
            .expect("static regex");
    let cut = [text.find("<footnote"), text.find("<sidebar")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(text.len());
    let mut notes = HashMap::new();
    let mut sidebars = HashMap::new();
    for c in re.captures_iter(&text[cut..]) {
        let target = if &c[1] == "footnote" {
            &mut notes
        } else {
            &mut sidebars
        };
        target.insert(c[2].to_string(), c[3].trim().to_string());
    }
    (text[..cut].to_string(), notes, sidebars)
}

/// Renders PML text. `images` maps an image name to the path to use in `src`.
pub fn render(
    text: &str,
    images: &dyn Fn(&str) -> Option<String>,
    footnotes: &HashMap<String, String>,
    sidebars: &HashMap<String, String>,
) -> Rendered {
    let mut r = Renderer {
        images: Some(images),
        notes_src: Some(footnotes),
        sidebars_src: Some(sidebars),
        allow_notes: true,
        ..Default::default()
    };
    r.run(text);
    r.out.notes = std::mem::take(&mut r.kinds);
    r.out
}

/// Metadata read from invisible-text pairs.
pub fn metadata_from_pairs(pairs: &[(String, String)]) -> Metadata {
    let mut md = Metadata::default();
    for (k, v) in pairs {
        let v = v.trim().to_string();
        if v.is_empty() {
            continue;
        }
        match k.as_str() {
            "TITLE" if md.title.is_empty() => md.title = v,
            "AUTHOR" if md.authors.is_empty() => md.authors = vec![v],
            "PUBLISHER" if md.publisher.is_empty() => md.publisher = v,
            "COPYRIGHT" if md.rights.is_empty() => md.rights = v,
            "EISBN" | "ISBN" if md.identifier("isbn").is_none() => {
                md.identifiers.push(bookx_core::opf::Identifier {
                    scheme: "isbn".into(),
                    value: v,
                    xml_id: None,
                })
            }
            _ => {}
        }
    }
    md
}

/// Everything needed to build the book from PML text.
pub struct PmlBook {
    pub md: Metadata,
    pub chapters: Vec<Chapter>,
    pub media: Vec<Media>,
    pub cover: Option<(Vec<u8>, String)>,
}

/// Converts PML text with its images, extra footnotes / sidebars (e.g. from PDB records) and
/// metadata defaults into chapters.
pub fn pml_to_book(
    text: &str,
    images: &ImageStore,
    mut footnotes: HashMap<String, String>,
    mut sidebars: HashMap<String, String>,
    mut md: Metadata,
) -> PmlBook {
    let (body, f2, s2) = split_notes(text);
    footnotes.extend(f2);
    sidebars.extend(s2);
    let media = RefCell::new(MediaSet::default());
    let lookup = |name: &str| -> Option<String> {
        let (key, data) = images.get(name)?;
        media.borrow_mut().add(key, data.to_vec())
    };
    let rendered = render(&body, &lookup, &footnotes, &sidebars);
    let from_text = metadata_from_pairs(&rendered.meta);
    if !from_text.title.is_empty() {
        md.title = from_text.title;
    }
    if !from_text.authors.is_empty() {
        md.authors = from_text.authors;
    }
    for (dst, src) in [
        (&mut md.publisher, from_text.publisher),
        (&mut md.rights, from_text.rights),
    ] {
        if !src.is_empty() {
            *dst = src;
        }
    }
    if md.identifier("isbn").is_none() {
        md.identifiers.extend(from_text.identifiers);
    }
    let default_title = if md.title.is_empty() {
        "Beginning".to_string()
    } else {
        md.title.clone()
    };
    let mut chapters = chapters_from_blocks(rendered.blocks, &default_title);
    if !rendered.notes.is_empty() {
        let asides: String = rendered
            .notes
            .iter()
            .map(|(id, inner)| footnote_aside(id, inner))
            .collect();
        match chapters.last_mut() {
            Some(last) => last.body.push_str(&asides),
            None => chapters.push(Chapter {
                title: "Notes".into(),
                body: asides,
            }),
        }
    }
    let cover = images
        .get("cover.png")
        .map(|(_, d)| (d.to_vec(), image_ext(d).unwrap_or("png").to_string()));
    PmlBook {
        md,
        chapters,
        media: media.into_inner().items,
        cover,
    }
}

fn decode_pml(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

fn finish(book: PmlBook, fallback_title: &str) -> Result<Archive> {
    let mut md = book.md;
    if md.title.is_empty() && !fallback_title.is_empty() {
        md.title = fallback_title.to_string();
    }
    build(Parts {
        md,
        chapters: book.chapters,
        cover: book.cover,
        media: book.media,
        css: CSS.into(),
        ..Default::default()
    })
}

/// Converts a bare `.pml` file (no images). `file_name` (may be empty) is the title fallback.
pub fn convert_pml(bytes: &[u8], file_name: &str) -> Result<Archive> {
    let text = decode_pml(bytes);
    if text.trim().is_empty() {
        bail!("the PML file is empty");
    }
    let book = pml_to_book(
        &text,
        &ImageStore::default(),
        HashMap::new(),
        HashMap::new(),
        Metadata::default(),
    );
    finish(book, &stem_of_name(file_name))
}

/// Converts a PMLZ archive: the first `.pml` (shortest path first) is the text, every image
/// anywhere in the archive can be referenced by file name. A `metadata.opf` supplies metadata
/// the text does not.
pub fn convert_pmlz(bytes: &[u8]) -> Result<Archive> {
    let zip = read_zip(bytes)?;
    let mut pmls: Vec<&(String, Vec<u8>)> = zip
        .files
        .iter()
        .filter(|(n, _)| lower_ext(n) == "pml")
        .collect();
    pmls.sort_by_key(|(n, _)| (n.matches('/').count(), n.clone()));
    let Some((name, data)) = pmls.first() else {
        bail!("the PMLZ archive contains no .pml file");
    };
    let text = decode_pml(data);
    if text.trim().is_empty() {
        bail!("the PML file in the archive is empty");
    }
    let mut images = ImageStore::default();
    for (n, d) in &zip.files {
        if image_ext(d).is_some() {
            images.insert(n, d.clone());
        }
    }
    let mut md = Metadata::default();
    if let Some(opf) = zip.get_ci("metadata.opf") {
        if let Some(info) = parse_opf_info(&decode_xml(opf)) {
            md = info.md;
        }
    }
    let book = pml_to_book(&text, &images, HashMap::new(), HashMap::new(), md);
    // generic member names say nothing about the book; the archive's own file name does
    let stem = stem_of_name(name);
    let generic = ["index", "book", "content", "main", "text"];
    finish(
        book,
        if generic.contains(&stem.to_ascii_lowercase().as_str()) {
            ""
        } else {
            &stem
        },
    )
}

/// File name without directories and extension.
pub fn stem_of_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(base)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn no_img(_: &str) -> Option<String> {
        None
    }

    fn blocks(src: &str) -> Vec<String> {
        render(src, &no_img, &HashMap::new(), &HashMap::new()).blocks
    }

    #[test]
    fn paragraphs_per_line_and_blank_lines_ignored() {
        assert_eq!(
            blocks("One\n\nTwo\r\nThree\r"),
            ["<p>One</p>", "<p>Two</p>", "<p>Three</p>"]
        );
        assert!(blocks("").is_empty());
        assert!(blocks("\n \n").is_empty());
    }

    #[test]
    fn inline_styles_toggle_and_merge() {
        assert_eq!(blocks("a \\ib\\i c"), ["<p>a <i>b</i> c</p>"]);
        assert_eq!(blocks("\\B\\iboth\\i\\B"), ["<p><b><i>both</i></b></p>"]);
        assert_eq!(
            blocks("\\uU\\u\\oO\\o\\ll\\l"),
            ["<p><u>U</u><s>O</s><big>l</big></p>"]
        );
        assert_eq!(
            blocks("x\\Spsup\\Sp\\Sbsub\\Sb"),
            ["<p>x<sup>sup</sup><sub>sub</sub></p>"]
        );
        assert_eq!(
            blocks("\\kab\\k"),
            ["<p><span class=\"smallcaps\">ab</span></p>"]
        );
        // an unclosed style ends with the text without breaking markup
        assert_eq!(blocks("\\ibroken"), ["<p><i>broken</i></p>"]);
        // \n resets bold and large
        assert_eq!(blocks("\\b\\lx\\ny"), ["<p><big><b>x</b></big>y</p>"]);
    }

    #[test]
    fn alignment_and_indent_blocks() {
        assert_eq!(
            blocks("\\cTitle\\c\nbody"),
            ["<p class=\"center\">Title</p>", "<p>body</p>"]
        );
        assert_eq!(blocks("\\rRight\\r"), ["<p class=\"right\">Right</p>"]);
        assert_eq!(blocks("\\tquoted\\t"), ["<p class=\"indent\">quoted</p>"]);
    }

    #[test]
    fn chapters_use_heading_levels() {
        assert_eq!(blocks("\\xOne\\x"), ["<h1>One</h1>"]);
        assert_eq!(blocks("\\X0Zero\\X0"), ["<h1>Zero</h1>"]);
        assert_eq!(blocks("\\X1Sub\\X1"), ["<h2>Sub</h2>"]);
        assert_eq!(blocks("\\X4Deep\\X4"), ["<h5>Deep</h5>"]);
        // unclosed chapter marker ends at the line end
        assert_eq!(blocks("\\xOpen\nnext"), ["<h1>Open</h1>", "<p>next</p>"]);
        assert_eq!(
            blocks("\\C1=\"Hidden\"\ntext"),
            ["<h2 class=\"pml-marker\">Hidden</h2>", "<p>text</p>"]
        );
    }

    #[test]
    fn special_characters() {
        assert_eq!(blocks("back\\\\slash"), ["<p>back\\slash</p>"]);
        assert_eq!(blocks("caf\\a233"), ["<p>café</p>"]);
        assert_eq!(blocks("\\a128 \\a000x"), ["<p>€ x</p>"]);
        assert_eq!(blocks("\\U20ac and \\U00e9"), ["<p>€ and é</p>"]);
        assert_eq!(blocks("soft\\-hyphen"), ["<p>soft\u{ad}hyphen</p>"]);
        assert_eq!(blocks("a < b & c"), ["<p>a &lt; b &amp; c</p>"]);
        assert_eq!(blocks("trailing\\"), ["<p>trailing</p>"]);
        assert_eq!(blocks("unknown \\zcode"), ["<p>unknown code</p>"]);
    }

    #[test]
    fn page_breaks_rules_and_params() {
        assert_eq!(
            blocks("a\n\\p\nb"),
            ["<p>a</p>", "<div class=\"pml-page\"></div>", "<p>b</p>"]
        );
        assert_eq!(blocks("a\\w=\"50%\"b"), ["<p>a</p>", "<hr/>", "<p>b</p>"]);
        assert_eq!(blocks("\\T=\"50%\"indented"), ["<p>indented</p>"]);
    }

    #[test]
    fn links_and_anchors() {
        assert_eq!(
            blocks("\\Q=\"top\"start \\q=\"#top\"back\\q."),
            ["<p><a id=\"pml-top\"></a>start <a href=\"#pml-top\">back</a>.</p>"]
        );
        // ids with odd characters are made safe
        assert!(blocks("\\Q=\"a b/c\"x")[0].contains("id=\"pml-a_b_c\""));
    }

    #[test]
    fn invisible_text_is_hidden_and_parsed_as_metadata() {
        let r = render(
            "\\vTITLE=\"My Book\" AUTHOR=\"Ann\"\nPUBLISHER=\"Pub\" EISBN=\"123\" COPYRIGHT=\"(c) 2020\"\\v\nVisible",
            &no_img,
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(r.blocks, ["<p>Visible</p>"]);
        let md = metadata_from_pairs(&r.meta);
        assert_eq!(md.title, "My Book");
        assert_eq!(md.authors, ["Ann"]);
        assert_eq!(md.publisher, "Pub");
        assert_eq!(md.rights, "(c) 2020");
        assert_eq!(md.identifier("isbn"), Some("123"));
        // an unclosed \v at the end still yields metadata and hides the text
        let r = render("\\vTITLE=\"T\"", &no_img, &HashMap::new(), &HashMap::new());
        assert!(r.blocks.is_empty());
        assert_eq!(r.meta, [("TITLE".to_string(), "T".to_string())]);
    }

    #[test]
    fn metadata_pairs_first_value_wins_and_empty_ignored() {
        let pairs: Vec<(String, String)> =
            [("TITLE", "A"), ("TITLE", "B"), ("AUTHOR", " "), ("X", "y")]
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect();
        let md = metadata_from_pairs(&pairs);
        assert_eq!(md.title, "A");
        assert!(md.authors.is_empty());
    }

    #[test]
    fn footnotes_and_sidebars() {
        let mut f = HashMap::new();
        f.insert("n1".to_string(), "Note \\itext\\i here".to_string());
        let mut s = HashMap::new();
        s.insert("s1".to_string(), "Side bar".to_string());
        let r = render(
            "See\\Fn=\"n1\"1\\Fn and \\Sd=\"s1\"box\\Sd.",
            &no_img,
            &f,
            &s,
        );
        assert_eq!(r.blocks.len(), 1);
        assert!(r.blocks[0].contains("epub:type=\"noteref\""));
        assert!(r.blocks[0].contains("href=\"#fn-pml-n1\""));
        assert!(r.blocks[0].contains("href=\"#sb-pml-s1\""));
        assert_eq!(r.notes.len(), 2);
        assert_eq!(r.notes[0].0, "fn-pml-n1");
        assert!(r.notes[0].1.contains("<p>Note <i>text</i> here</p>"));
        assert!(r.notes[1].1.contains("Side bar"));
        // unknown note ids degrade to plain text
        let r = render("x\\Fn=\"missing\"7\\Fn", &no_img, &f, &s);
        assert_eq!(r.blocks, ["<p>x7</p>"]);
        assert!(r.notes.is_empty());
        // the same note referenced twice is stored once
        let r = render("\\Fn=\"n1\"a\\Fn \\Fn=\"n1\"b\\Fn", &no_img, &f, &s);
        assert_eq!(r.notes.len(), 1);
    }

    #[test]
    fn split_notes_cuts_text_after_the_first_note() {
        let (body, f, s) = split_notes(
            "Body text\n<footnote id=\"a\">First</footnote>\nIgnored\n<sidebar id=\"b\">\nSide\n</sidebar>",
        );
        assert_eq!(body, "Body text\n");
        assert_eq!(f["a"], "First");
        assert_eq!(s["b"], "Side");
        let (body, f, s) = split_notes("plain");
        assert_eq!(body, "plain");
        assert!(f.is_empty() && s.is_empty());
    }

    #[test]
    fn images_resolve_through_the_store() {
        let mut store = ImageStore::default();
        store.insert("book_img/Pic.PNG", PNG.to_vec());
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
        assert!(store.get("pic.png").is_some());
        assert!(store.get("PIC").is_some(), "extension optional");
        assert!(store.get("other.png").is_none());
        let book = pml_to_book(
            "\\m=\"pic.png\"\n\\m=\"none.png\"\ntext",
            &store,
            HashMap::new(),
            HashMap::new(),
            Metadata::default(),
        );
        assert_eq!(book.media.len(), 1);
        assert!(book.media[0].path.starts_with("images/"));
        assert!(book.chapters[0]
            .body
            .contains(&format!("src=\"{}\"", book.media[0].path)));
    }

    #[test]
    fn book_has_chapters_from_headings_and_notes_at_the_end() {
        let text = "\\xFirst\\x\nHello\\Fn=\"a\"1\\Fn\n\\xSecond\\x\nWorld\n<footnote id=\"a\">Foot</footnote>";
        let b = pml_to_book(
            text,
            &ImageStore::default(),
            HashMap::new(),
            HashMap::new(),
            Metadata::default(),
        );
        assert_eq!(b.chapters.len(), 2);
        assert_eq!(b.chapters[0].title, "First");
        assert_eq!(b.chapters[1].title, "Second");
        assert!(b.chapters[1].body.contains("epub:type=\"footnote\""));
        assert!(b.chapters[0].body.contains("noteref"));
    }

    #[test]
    fn pml_reader_end_to_end() {
        let src = b"\\vTITLE=\"Test Book\" AUTHOR=\"Al\"\\v\n\\x\\cOne\\c\\x\nHello \\iworld\\i.\n\\xTwo\\x\nBye.\n";
        let arc = PmlReader.read(src).unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Test Book");
        assert_eq!(book.metadata().authors, ["Al"]);
        let texts = chapter_texts(&book);
        assert_eq!(texts.len(), 2);
        assert!(texts[0].contains("Hello"));
        assert!(chapter_xml(&book, 0).contains("<i>world</i>"));
        assert_eq!(toc_titles(&book), ["One", "Two"]);
    }

    #[test]
    fn pml_cp1252_bytes_and_title_fallback() {
        let arc = convert_pml(b"Caf\xe9 au lait\n", "dir/My Doc.pml").unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "My Doc");
        assert!(chapter_texts(&book)[0].contains("Café au lait"));
        let arc = convert_pml("\u{feff}Ünï\n".as_bytes(), "").unwrap();
        assert!(chapter_texts(&validate(&arc))[0].contains("Ünï"));
    }

    #[test]
    fn empty_pml_is_an_error() {
        assert!(format!("{}", PmlReader.read(b"  \n").unwrap_err()).contains("empty"));
        assert!(
            PmlReader.read(b"\\v hidden \\v").is_err(),
            "no visible text"
        );
    }

    #[test]
    fn pmlz_with_images_cover_and_metadata() {
        let opf = br#"<package><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Zipped</dc:title><dc:creator>Zed</dc:creator></metadata></package>"#;
        let z = make_zip(&[
            ("book.pml", b"\\xCh\\x\n\\m=\"pic.png\"\nText"),
            ("book_img/pic.png", PNG),
            ("book_img/cover.png", PNG),
            ("metadata.opf", opf),
        ]);
        assert!(PmlzReader.sniff(&z));
        let arc = PmlzReader.read(&z).unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Zipped");
        assert_eq!(book.metadata().authors, ["Zed"]);
        assert!(chapter_xml(&book, 0).contains("<img src=\"images/"));
        assert!(
            arc.to_bytes().unwrap().windows(5).any(|w| w == b"cover"),
            "cover stored"
        );
    }

    #[test]
    fn pmlz_without_pml_and_title_from_file_name() {
        let z = make_zip(&[("readme.txt", b"hi")]);
        assert!(!PmlzReader.sniff(&z));
        assert!(format!("{}", PmlzReader.read(&z).unwrap_err()).contains("no .pml"));
        let z = make_zip(&[("sub/deep.pml", b"x\n"), ("Top Title.pml", b"Hello\n")]);
        let book = validate(&PmlzReader.read(&z).unwrap());
        assert_eq!(book.metadata().title, "Top Title", "shallowest .pml wins");
        let z = make_zip(&[("a.pml", b"  \n")]);
        assert!(PmlzReader.read(&z).is_err());
    }

    #[test]
    fn sniff_and_ids() {
        assert!(!PmlReader.sniff(b"\\x"));
        assert_eq!(PmlReader.extensions(), &["pml"]);
        assert_eq!(PmlzReader.extensions(), &["pmlz"]);
        assert_eq!(PmlReader.id(), "pml");
        assert_eq!(PmlzReader.id(), "pmlz");
        assert!(!PmlzReader.sniff(b"not a zip"));
        assert_eq!(stem_of_name("a/b/c.d.pml"), "c.d");
        assert_eq!(stem_of_name("noext"), "noext");
    }

    #[test]
    fn fuzz() {
        let z = make_zip(&[
            ("book.pml", b"\\xCh\\x\n\\m=\"pic.png\" \\iit\\i \\Fn=\"a\"1\\Fn\n<footnote id=\"a\">F</footnote>"),
            ("pic.png", PNG),
        ]);
        fuzz_no_panic(&PmlzReader, &z);
        fuzz_no_panic(
            &PmlReader,
            b"\\vTITLE=\"x\"\\v\\xA\\x\n\\a233 \\U20ac \\Sd=\"s\"b\\Sd text\n",
        );
        assert_rejects_garbage(&PmlzReader);
        assert_rejects_garbage(&PmlReader);
    }
}
