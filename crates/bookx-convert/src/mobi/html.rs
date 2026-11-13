//! XHTML spine -> the HTML subset MOBI 6 readers understand, plus images, anchors and TOC offsets.

use std::collections::HashMap;
use std::io::Cursor;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::toc::TocEntry;
use bookx_core::util::{is_external, parent_dir, resolve_href};
use image::codecs::jpeg::JpegEncoder;
use image::ImageReader;

use crate::markup::{
    body_range, decode_entities, escape_attr, escape_text, get_attr, tag_info, tokenize, Tok,
};

/// Marker used for 10-digit `filepos` values that are patched once all offsets are known.
const FILEPOS_PLACEHOLDER: &str = "0000000000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocItem {
    pub title: String,
    pub offset: usize,
    pub depth: usize,
}

pub struct Converted {
    pub text: Vec<u8>,
    /// Image records in file order; the cover (when present) comes first, its thumbnail last.
    pub images: Vec<Vec<u8>>,
    pub cover_index: Option<usize>,
    pub thumb_index: Option<usize>,
    pub toc: Vec<TocItem>,
    /// Offset of the first content, used as the "start reading" position.
    pub start_offset: usize,
}

fn mobi_tag(name: &str) -> Option<&'static str> {
    Some(match name {
        "a" => "a",
        "b" | "strong" => "b",
        "i" | "em" | "cite" | "dfn" => "i",
        "u" | "ins" => "u",
        "s" | "strike" | "del" => "s",
        "p" => "p",
        "div" => "div",
        "br" => "br",
        "hr" => "hr",
        "h1" => "h1",
        "h2" => "h2",
        "h3" => "h3",
        "h4" => "h4",
        "h5" => "h5",
        "h6" => "h6",
        "ul" => "ul",
        "ol" => "ol",
        "li" => "li",
        "dl" => "dl",
        "dt" => "dt",
        "dd" => "dd",
        "blockquote" => "blockquote",
        "pre" => "pre",
        "table" => "table",
        "thead" => "thead",
        "tbody" => "tbody",
        "tfoot" => "tfoot",
        "tr" => "tr",
        "td" => "td",
        "th" => "th",
        "caption" => "caption",
        "sup" => "sup",
        "sub" => "sub",
        "small" => "small",
        "big" => "big",
        "center" => "center",
        "code" | "tt" | "kbd" | "samp" => "tt",
        _ => return None,
    })
}

/// JPEG and GIF are kept; PNG is flattened onto white and re-encoded as JPEG. Other formats are
/// dropped (MOBI 6 readers cannot show them).
pub fn to_mobi_image(data: &[u8]) -> Option<Vec<u8>> {
    if data.starts_with(&[0xff, 0xd8, 0xff]) || data.starts_with(b"GIF8") {
        return Some(data.to_vec());
    }
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        let img = image::load_from_memory(data).ok()?;
        let rgba = img.to_rgba8();
        let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
        for (x, y, p) in rgba.enumerate_pixels() {
            let a = p[3] as u32;
            let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
            rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
        }
        let mut out = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&rgb)
            .ok()?;
        return Some(out.into_inner());
    }
    None
}

pub(crate) fn thumbnail(data: &[u8]) -> Option<Vec<u8>> {
    let img = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let small = img.thumbnail(180, 240).to_rgb8();
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&small)
        .ok()?;
    Some(out.into_inner())
}

type Key = (String, String);

struct Builder<'a> {
    book: &'a OpenBook,
    out: String,
    pos: HashMap<Key, usize>,
    fixups: Vec<(usize, Key)>,
    images: Vec<Vec<u8>>,
    image_ix: HashMap<String, usize>,
    spine: Vec<String>,
    /// True right after a block-level tag: white space there is not significant.
    at_boundary: bool,
}

fn is_block_out(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "blockquote"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "caption"
            | "center"
            | "br"
            | "hr"
    )
}

impl Builder<'_> {
    /// 1-based `recindex` of the image at `path`, adding it on first use.
    fn image_for(&mut self, path: &str) -> Option<usize> {
        if let Some(i) = self.image_ix.get(path) {
            return Some(*i);
        }
        let data = self.book.archive.get(path)?;
        let conv = to_mobi_image(data)?;
        self.images.push(conv);
        let idx = self.images.len();
        self.image_ix.insert(path.to_string(), idx);
        Some(idx)
    }

    fn push_img(&mut self, doc_dir: &str, src: &str, alt: Option<String>) {
        let src = decode_entities(src);
        let (path, _) = resolve_href(doc_dir, &src);
        if let Some(idx) = self.image_for(&path) {
            let alt = alt
                .map(|a| format!(" alt=\"{}\"", escape_attr(&decode_entities(&a))))
                .unwrap_or_default();
            self.out
                .push_str(&format!("<img recindex=\"{idx:05}\"{alt}/>"));
            self.at_boundary = false;
        }
    }

    fn convert_doc(&mut self, path: &str, html: &str) {
        let (bs, be) = body_range(html).unwrap_or((0, html.len()));
        let body = &html[bs..be];
        let dir = parent_dir(path).to_string();
        let (mut skip, mut svg, mut pre) = (0usize, 0usize, 0usize);
        self.at_boundary = true;
        for t in tokenize(body) {
            match t {
                Tok::Tag(tag) => {
                    let Some(info) = tag_info(tag) else { continue };
                    let name = info.name.as_str();
                    if !info.closing {
                        for attr in ["id", "name"] {
                            if let Some(v) = get_attr(tag, attr) {
                                self.pos
                                    .entry((path.to_string(), decode_entities(&v)))
                                    .or_insert(self.out.len());
                            }
                        }
                    }
                    match name {
                        "script" | "style" | "title" | "head" | "object" | "iframe"
                            if !info.self_closing =>
                        {
                            if info.closing {
                                skip = skip.saturating_sub(1)
                            } else {
                                skip += 1
                            }
                            continue;
                        }
                        "svg" | "math" if !info.self_closing => {
                            if info.closing {
                                svg = svg.saturating_sub(1)
                            } else {
                                svg += 1
                            }
                            continue;
                        }
                        _ => {}
                    }
                    if skip > 0 {
                        continue;
                    }
                    if svg > 0 {
                        if name == "image" && !info.closing {
                            if let Some(h) =
                                get_attr(tag, "xlink:href").or_else(|| get_attr(tag, "href"))
                            {
                                self.push_img(&dir, &h, None);
                            }
                        }
                        continue;
                    }
                    if name == "img" {
                        if !info.closing {
                            if let Some(src) = get_attr(tag, "src") {
                                self.push_img(&dir, &src, get_attr(tag, "alt"));
                            }
                        }
                        continue;
                    }
                    let Some(out_name) = mobi_tag(name) else {
                        continue;
                    };
                    self.at_boundary =
                        is_block_out(out_name) || (self.at_boundary && !info.closing);
                    if name == "pre" {
                        if info.closing {
                            pre = pre.saturating_sub(1)
                        } else {
                            pre += 1
                        }
                    }
                    if info.closing {
                        if !matches!(out_name, "br" | "hr") {
                            self.out.push_str(&format!("</{out_name}>"));
                        }
                        continue;
                    }
                    match out_name {
                        "br" | "hr" => self.out.push_str(&format!("<{out_name}/>")),
                        "a" => self.open_anchor(tag, path, &dir),
                        "td" | "th" => {
                            let mut s = format!("<{out_name}");
                            for attr in ["colspan", "rowspan"] {
                                if let Some(v) = get_attr(tag, attr)
                                    .filter(|v| v.chars().all(|c| c.is_ascii_digit()))
                                {
                                    s.push_str(&format!(" {attr}=\"{v}\""));
                                }
                            }
                            s.push_str(if info.self_closing { "></" } else { ">" });
                            if info.self_closing {
                                s.push_str(out_name);
                                s.push('>');
                            }
                            self.out.push_str(&s);
                        }
                        _ if info.self_closing => {}
                        _ => self.out.push_str(&format!("<{out_name}>")),
                    }
                }
                Tok::Text(s) => {
                    if skip > 0 || svg > 0 {
                        continue;
                    }
                    let mut text = decode_entities(s);
                    text.retain(|c| {
                        c == '\n' || c == '\t' || c == '\r' || (c >= ' ' && c != '\u{feff}')
                    });
                    if pre == 0 {
                        let mut collapsed = String::with_capacity(text.len());
                        let mut last_space = false;
                        for c in text.chars() {
                            if c.is_ascii_whitespace() {
                                if !last_space {
                                    collapsed.push(' ');
                                }
                                last_space = true;
                            } else {
                                collapsed.push(c);
                                last_space = false;
                            }
                        }
                        text = collapsed;
                    }
                    if self.at_boundary && pre == 0 {
                        text = text.trim_start().to_string();
                    }
                    if text.is_empty() {
                        continue;
                    }
                    self.at_boundary = false;
                    self.out.push_str(&escape_text(&text));
                }
            }
        }
    }

    fn open_anchor(&mut self, tag: &str, path: &str, dir: &str) {
        let Some(href) = get_attr(tag, "href") else {
            self.out.push_str("<a>");
            return;
        };
        let href = decode_entities(&href);
        if is_external(&href) {
            self.out
                .push_str(&format!("<a href=\"{}\">", escape_attr(&href)));
            return;
        }
        let (target, frag) = resolve_href(dir, &href);
        let target = if target.is_empty() {
            path.to_string()
        } else {
            target
        };
        if !self.spine.contains(&target) {
            self.out.push_str("<a>");
            return;
        }
        self.out.push_str("<a filepos=");
        let digits = self.out.len();
        self.out.push_str(FILEPOS_PLACEHOLDER);
        self.out.push('>');
        self.fixups
            .push((digits, (target, frag.unwrap_or_default())));
    }
}

fn flatten(entries: &[TocEntry], depth: usize, out: &mut Vec<(String, String, usize)>) {
    for e in entries {
        out.push((e.title.clone(), e.href.clone(), depth));
        flatten(&e.children, depth + 1, out);
    }
}

pub(crate) fn doc_title(html: &str, fallback: usize) -> String {
    regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>")
        .ok()
        .and_then(|re| {
            re.captures(html)
                .map(|c| crate::markup::plain_text(&c[1]).trim().to_string())
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("Section {fallback}"))
}

pub fn convert(book: &OpenBook, inline_toc: bool) -> Result<Converted> {
    let spine = book.spine_paths();
    let mut b = Builder {
        book,
        out: String::new(),
        pos: HashMap::new(),
        fixups: Vec::new(),
        images: Vec::new(),
        image_ix: HashMap::new(),
        spine: spine.clone(),
        at_boundary: true,
    };

    // cover first so that its record offset is 0
    let mut cover_index = None;
    let mut cover_src: Option<Vec<u8>> = None;
    if let Some(p) = book.package.cover_path() {
        if b.image_for(&p).is_some() {
            cover_index = Some(0);
            cover_src = book.archive.get(&p).map(|d| d.to_vec());
        }
    }

    b.out
        .push_str("<html><head><guide><reference type=\"text\" title=\"Start\" filepos=");
    let guide_digits = b.out.len();
    b.out.push_str(FILEPOS_PLACEHOLDER);
    b.out.push_str(" />");
    let mut guide_toc_digits = None;
    if inline_toc {
        b.out
            .push_str("<reference type=\"toc\" title=\"Table of Contents\" filepos=");
        guide_toc_digits = Some(b.out.len());
        b.out.push_str(FILEPOS_PLACEHOLDER);
        b.out.push_str(" />");
    }
    b.out.push_str("</guide></head><body>");

    let mut start_offset = None;
    let mut first = true;
    for path in &spine {
        let Some(html) = book.archive.get_string(path) else {
            continue;
        };
        if !first {
            b.out.push_str("<mbp:pagebreak/>");
        }
        first = false;
        b.pos.insert((path.clone(), String::new()), b.out.len());
        start_offset.get_or_insert(b.out.len());
        b.convert_doc(path, &html);
    }
    let start_offset = start_offset.unwrap_or(b.out.len());
    if let Some(first) = spine.first() {
        b.fixups
            .push((guide_digits, (first.clone(), String::new())));
    }

    // table of contents entries: (title, key, depth)
    let mut flat = Vec::new();
    flatten(&book.toc(), 0, &mut flat);
    let mut toc_keys: Vec<(String, Key, usize)> = flat
        .into_iter()
        .filter_map(|(title, href, depth)| {
            let (path, frag) = match href.split_once('#') {
                Some((p, f)) => (p.to_string(), f.to_string()),
                None => (href.clone(), String::new()),
            };
            let key = if b.pos.contains_key(&(path.clone(), frag.clone())) {
                (path, frag)
            } else if b.pos.contains_key(&(path.clone(), String::new())) {
                (path, String::new())
            } else {
                return None;
            };
            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            (!title.is_empty()).then(|| (title.chars().take(200).collect::<String>(), key, depth))
        })
        .collect();
    if toc_keys.is_empty() {
        for (i, path) in spine.iter().enumerate() {
            let key = (path.clone(), String::new());
            if b.pos.contains_key(&key) {
                let title = doc_title(&book.archive.get_string(path).unwrap_or_default(), i + 1);
                toc_keys.push((title, key, 0));
            }
        }
    }

    // inline table of contents page at the end
    if inline_toc && !toc_keys.is_empty() {
        b.out.push_str("<mbp:pagebreak/>");
        let page = b.out.len();
        b.pos.insert(("\0toc".to_string(), String::new()), page);
        b.fixups.push((
            guide_toc_digits.unwrap_or(guide_digits),
            ("\0toc".to_string(), String::new()),
        ));
        b.out.push_str("<h2>Table of Contents</h2>");
        for (title, key, depth) in &toc_keys {
            for _ in 0..*depth {
                b.out.push_str("<blockquote>");
            }
            b.out.push_str("<p><a filepos=");
            let digits = b.out.len();
            b.out.push_str(FILEPOS_PLACEHOLDER);
            b.out.push('>');
            b.out.push_str(&escape_text(title));
            b.out.push_str("</a></p>");
            for _ in 0..*depth {
                b.out.push_str("</blockquote>");
            }
            b.fixups.push((digits, key.clone()));
        }
    } else if let Some(d) = guide_toc_digits {
        // no page: point the reference at the start so that the guide stays valid
        b.fixups.push((
            d,
            (spine.first().cloned().unwrap_or_default(), String::new()),
        ));
    }
    b.out.push_str("</body></html>");

    // patch filepos placeholders (same width, so offsets do not move)
    let fixups = std::mem::take(&mut b.fixups);
    for (at, key) in fixups {
        let off = b
            .pos
            .get(&key)
            .or_else(|| b.pos.get(&(key.0.clone(), String::new())))
            .copied()
            .unwrap_or(0);
        b.out.replace_range(at..at + 10, &format!("{off:010}"));
    }

    let mut toc: Vec<TocItem> = toc_keys
        .iter()
        .filter_map(|(title, key, depth)| {
            b.pos.get(key).map(|off| TocItem {
                title: title.clone(),
                offset: *off,
                depth: *depth,
            })
        })
        .collect();
    if toc.is_empty() {
        toc.push(TocItem {
            title: book.metadata().title.clone(),
            offset: start_offset,
            depth: 0,
        });
    }

    let mut thumb_index = None;
    if let Some(src) = &cover_src {
        if let Some(t) = thumbnail(src) {
            b.images.push(t);
            thumb_index = Some(b.images.len() - 1);
        }
    }
    let text = b.out.into_bytes();
    Ok(Converted {
        text,
        images: b.images,
        cover_index,
        thumb_index,
        toc,
        start_offset,
    })
}
