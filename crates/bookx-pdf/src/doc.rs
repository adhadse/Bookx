//! A PDF document: opening (with a password), metadata, outline, page labels, links and the
//! text of a page. Everything here is synchronous and single threaded; [`crate::worker`] runs it
//! on a thread of its own.

use std::collections::{HashMap, HashSet};

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object};
use hayro::hayro_interpret::hayro_syntax::xref::XRef;
use hayro::hayro_interpret::hayro_syntax::{LoadPdfError, Pdf};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask,
};
use hayro::kurbo::{Affine, BezPath, Point, Rect as KRect};

use crate::text::{PageText, RawGlyph, Rect};

/// Why a document could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// Encrypted and no password was given.
    NeedsPassword,
    /// Encrypted and the password does not open it.
    WrongPassword,
    /// An encryption method that is not supported.
    Unsupported(String),
    /// Not a PDF, or too damaged to read.
    Invalid,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NeedsPassword => f.write_str("the PDF is password protected"),
            OpenError::WrongPassword => f.write_str("that password does not open the PDF"),
            OpenError::Unsupported(s) => write!(f, "unsupported PDF encryption: {s}"),
            OpenError::Invalid => f.write_str("not a readable PDF file"),
        }
    }
}

impl std::error::Error for OpenError {}

/// The document information dictionary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
}

/// Where an outline entry or a link goes inside the document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dest {
    pub page: usize,
    /// Distance from the top of the page in points, when the destination names one.
    pub y: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    /// 0 for the top level.
    pub level: usize,
    pub dest: Option<Dest>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkTarget {
    Page(Dest),
    Uri(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub rect: Rect,
    pub target: LinkTarget,
}

/// An open PDF.
pub struct Document {
    pub(crate) pdf: Pdf,
    ids: HashMap<(u32, i32), usize>,
}

const MAX_OUTLINE: usize = 20_000;
const MAX_DEPTH: usize = 24;

/// Decodes a PDF text string (UTF-16 with BOM, UTF-8 with BOM, or PDFDocEncoding).
pub fn decode_text_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let u: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u)
            .trim_end_matches('\0')
            .to_string();
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        let u: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&u)
            .trim_end_matches('\0')
            .to_string();
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).to_string();
    }
    b.iter()
        .map(|&c| match c {
            0x80 => '•',
            0x81 => '†',
            0x82 => '‡',
            0x83 => '…',
            0x84 => '—',
            0x85 => '–',
            0x86 => 'ƒ',
            0x87 => '⁄',
            0x88 => '‹',
            0x89 => '›',
            0x8A => '−',
            0x8B => '‰',
            0x8C => '„',
            0x8D => '“',
            0x8E => '”',
            0x8F => '‘',
            0x90 => '’',
            0x91 => '‚',
            0x92 => '™',
            c => c as char,
        })
        .collect()
}

fn text_of(d: &Dict, key: &str) -> Option<String> {
    let s = d.get::<hayro::hayro_interpret::hayro_syntax::object::String>(key)?;
    Some(decode_text_string(s.as_bytes()))
}

impl Document {
    /// Opens a PDF held in memory. `password` is tried for encrypted files (an empty password
    /// is always tried first).
    pub fn open(data: Vec<u8>, password: Option<&str>) -> Result<Document, OpenError> {
        if !data.windows(5).take(2048).any(|w| w == b"%PDF-") {
            return Err(OpenError::Invalid);
        }
        let pdf = match Pdf::new_with_password(data, password.unwrap_or("")) {
            Ok(p) => p,
            Err(LoadPdfError::Decryption(e)) => {
                use hayro::hayro_interpret::hayro_syntax::DecryptionError as D;
                return Err(match e {
                    D::PasswordProtected if password.is_some_and(|p| !p.is_empty()) => {
                        OpenError::WrongPassword
                    }
                    D::PasswordProtected => OpenError::NeedsPassword,
                    D::UnsupportedAlgorithm => OpenError::Unsupported("algorithm".into()),
                    D::MissingIDEntry | D::InvalidEncryption => OpenError::Invalid,
                });
            }
            Err(LoadPdfError::Invalid) => return Err(OpenError::Invalid),
        };
        if pdf.pages().is_empty() {
            return Err(OpenError::Invalid);
        }
        let mut ids = HashMap::new();
        for (i, p) in pdf.pages().iter().enumerate() {
            if let Some(id) = p.raw().obj_id() {
                ids.entry((id.obj_number, id.gen_number)).or_insert(i);
            }
        }
        Ok(Document { pdf, ids })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Width and height of page `i` as displayed (rotation applied), in points.
    pub fn page_size(&self, i: usize) -> (f32, f32) {
        self.pdf
            .pages()
            .get(i)
            .map(|p| p.render_dimensions())
            .map(|(w, h)| (w.max(1.0), h.max(1.0)))
            .unwrap_or((612.0, 792.0))
    }

    pub fn page_sizes(&self) -> Vec<(f32, f32)> {
        (0..self.page_count()).map(|i| self.page_size(i)).collect()
    }

    pub fn info(&self) -> Info {
        let m = self.pdf.metadata();
        let s = |b: &Option<Box<[u8]>>| {
            b.as_deref()
                .map(decode_text_string)
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        Info {
            title: s(&m.title),
            author: s(&m.author),
            subject: s(&m.subject),
            keywords: s(&m.keywords),
            creator: s(&m.creator),
            producer: s(&m.producer),
        }
    }

    fn xref(&self) -> &XRef {
        self.pdf.xref()
    }

    fn catalog(&self) -> Option<Dict<'_>> {
        self.xref().get::<Dict>(self.xref().root_id())
    }

    /// The outline (bookmarks) flattened in document order with its nesting level.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let Some(root) = self.catalog().and_then(|c| c.get::<Dict>("Outlines")) else {
            return out;
        };
        let Some(first) = root.get::<Dict>("First") else {
            return out;
        };
        let mut seen = HashSet::new();
        self.walk_outline(first, 0, &mut out, &mut seen);
        out
    }

    fn walk_outline<'a>(
        &'a self,
        first: Dict<'a>,
        level: usize,
        out: &mut Vec<OutlineItem>,
        seen: &mut HashSet<(u32, i32)>,
    ) {
        let mut cur = Some(first);
        while let Some(d) = cur {
            if out.len() >= MAX_OUTLINE || level > MAX_DEPTH {
                return;
            }
            if let Some(id) = d.obj_id() {
                if !seen.insert((id.obj_number, id.gen_number)) {
                    return;
                }
            }
            let title = text_of(&d, "Title").unwrap_or_default();
            let dest = self.entry_dest(&d);
            out.push(OutlineItem {
                title: title.trim().to_string(),
                level,
                dest,
            });
            if let Some(kid) = d.get::<Dict>("First") {
                self.walk_outline(kid, level + 1, out, seen);
            }
            cur = d.get::<Dict>("Next");
        }
    }

    /// The destination of an outline entry or a link annotation (`Dest`, or a GoTo action).
    fn entry_dest(&self, d: &Dict<'_>) -> Option<Dest> {
        if let Some(o) = d.get::<Object>("Dest") {
            return self.resolve_dest(o, 0);
        }
        let a = d.get::<Dict>("A")?;
        if a.get::<Name>("S").is_some_and(|n| n.as_str() == "GoTo") {
            return self.resolve_dest(a.get::<Object>("D")?, 0);
        }
        None
    }

    fn resolve_dest(&self, o: Object<'_>, depth: usize) -> Option<Dest> {
        if depth > 4 {
            return None;
        }
        match o {
            Object::Array(a) => self.array_dest(&a),
            Object::Dict(d) => self.resolve_dest(d.get::<Object>("D")?, depth + 1),
            Object::Name(n) => self.named_dest(n.as_ref(), depth),
            Object::String(s) => self.named_dest(s.as_bytes(), depth),
            _ => None,
        }
    }

    fn array_dest(&self, a: &Array<'_>) -> Option<Dest> {
        let mut it = a.raw_iter();
        let page = match it.next()? {
            MaybeRef::Ref(r) => *self.ids.get(&(r.obj_number, r.gen_number))?,
            MaybeRef::NotRef(Object::Number(n)) => {
                let i = n.as_i64();
                if i < 0 || i as usize >= self.page_count() {
                    return None;
                }
                i as usize
            }
            _ => return None,
        };
        let kind = match it.next() {
            Some(MaybeRef::NotRef(Object::Name(n))) => n.as_str().to_string(),
            _ => String::new(),
        };
        let nums: Vec<Option<f64>> = it
            .map(|x| match x {
                MaybeRef::NotRef(Object::Number(n)) => Some(n.as_f64()),
                _ => None,
            })
            .collect();
        let top = match kind.as_str() {
            "XYZ" => nums.get(1).copied().flatten(),
            "FitH" | "FitBH" => nums.first().copied().flatten(),
            _ => None,
        };
        let y = top.map(|t| {
            let m = self.page_affine(page);
            (m * Point::new(0.0, t)).y as f32
        });
        Some(Dest { page, y })
    }

    fn named_dest(&self, name: &[u8], depth: usize) -> Option<Dest> {
        let cat = self.catalog()?;
        if let Some(d) = cat.get::<Dict>("Dests") {
            if let Some(o) = d.get::<Object>(name) {
                return self.resolve_dest(o, depth + 1);
            }
        }
        let tree = cat.get::<Dict>("Names")?.get::<Dict>("Dests")?;
        let o = name_tree_lookup(&tree, name, 0)?;
        self.resolve_dest(o, depth + 1)
    }

    fn page_affine(&self, page: usize) -> Affine {
        self.pdf
            .pages()
            .get(page)
            .map(|p| p.initial_transform(true).to_kurbo())
            .unwrap_or(Affine::IDENTITY)
    }

    /// The label of every page (`iv`, `A-3`, `12`), from the `/PageLabels` number tree; plain
    /// numbers when the document has none.
    pub fn page_labels(&self) -> Vec<String> {
        let n = self.page_count();
        let mut ranges: Vec<(usize, String, String, i64)> = Vec::new();
        if let Some(tree) = self.catalog().and_then(|c| c.get::<Dict>("PageLabels")) {
            let mut raw: Vec<(i64, Dict)> = Vec::new();
            collect_number_tree(&tree, &mut raw, 0);
            raw.sort_by_key(|(k, _)| *k);
            for (k, d) in raw {
                if k < 0 {
                    continue;
                }
                let style = d
                    .get::<Name>("S")
                    .map(|n| n.as_str().to_string())
                    .unwrap_or_default();
                let prefix = text_of(&d, "P").unwrap_or_default();
                let start = d.get::<i32>("St").map(|v| v as i64).unwrap_or(1);
                ranges.push((k as usize, style, prefix, start));
            }
        }
        if ranges.is_empty() || ranges[0].0 != 0 {
            ranges.insert(0, (0, "D".into(), String::new(), 1));
        }
        make_labels(n, &ranges)
    }

    /// The link annotations of a page with their boxes in page space.
    pub fn links(&self, page: usize) -> Vec<Link> {
        let mut out = Vec::new();
        let Some(p) = self.pdf.pages().get(page) else {
            return out;
        };
        let Some(annots) = p.raw().get::<Array>("Annots") else {
            return out;
        };
        let m = self.page_affine(page);
        for a in annots.iter::<Dict>().take(5000) {
            if a.get::<Name>("Subtype")
                .is_none_or(|n| n.as_str() != "Link")
            {
                continue;
            }
            let Some(r) = a.get::<[f64; 4]>("Rect") else {
                continue;
            };
            let p0 = m * Point::new(r[0].min(r[2]), r[1].min(r[3]));
            let p1 = m * Point::new(r[0].max(r[2]), r[1].max(r[3]));
            let rect = [
                p0.x.min(p1.x) as f32,
                p0.y.min(p1.y) as f32,
                p0.x.max(p1.x) as f32,
                p0.y.max(p1.y) as f32,
            ];
            let target = if let Some(d) = self.entry_dest(&a) {
                LinkTarget::Page(d)
            } else { match a.get::<Dict>("A") { Some(act) => {
                match (
                    act.get::<Name>("S").map(|n| n.as_str().to_string()),
                    act.get::<hayro::hayro_interpret::hayro_syntax::object::String>("URI"),
                ) {
                    (Some(s), Some(u)) if s == "URI" => {
                        LinkTarget::Uri(String::from_utf8_lossy(u.as_bytes()).trim().to_string())
                    }
                    _ => continue,
                }
            } _ => {
                continue;
            }}};
            out.push(Link { rect, target });
        }
        out
    }

    /// The text of a page with the position of every character.
    pub fn text(&self, page: usize) -> PageText {
        let Some(p) = self.pdf.pages().get(page) else {
            return PageText::default();
        };
        let (w, h) = p.render_dimensions();
        let cache = InterpreterCache::new();
        let m = p.initial_transform(true).to_kurbo();
        let mut ctx = Context::new(
            m,
            KRect::new(-1.0e6, -1.0e6, 1.0e6, 1.0e6),
            &cache,
            p.xref(),
            InterpreterSettings::default(),
        );
        let mut ex = Extract { glyphs: Vec::new() };
        hayro::hayro_interpret::interpret_page(p, &mut ctx, &mut ex);
        PageText::build(w, h, ex.glyphs)
    }

    /// Text of all pages (for indexing); one entry per page.
    pub fn all_text(&self) -> Vec<PageText> {
        (0..self.page_count()).map(|i| self.text(i)).collect()
    }
}

fn name_tree_lookup<'a>(node: &Dict<'a>, key: &[u8], depth: usize) -> Option<Object<'a>> {
    if depth > 12 {
        return None;
    }
    if let Some(names) = node.get::<Array>("Names") {
        let mut it = names.flex_iter();
        loop {
            let k = it.next::<hayro::hayro_interpret::hayro_syntax::object::String>()?;
            let v = it.next::<Object>()?;
            if k.as_bytes() == key {
                return Some(v);
            }
        }
    }
    for kid in node.get::<Array>("Kids")?.iter::<Dict>() {
        if let Some(v) = name_tree_lookup(&kid, key, depth + 1) {
            return Some(v);
        }
    }
    None
}

fn collect_number_tree<'a>(node: &Dict<'a>, out: &mut Vec<(i64, Dict<'a>)>, depth: usize) {
    if depth > 12 || out.len() > 100_000 {
        return;
    }
    if let Some(nums) = node.get::<Array>("Nums") {
        let mut it = nums.flex_iter();
        while let Some(k) = it.next::<Object>() {
            let Object::Number(k) = k else { break };
            let Some(v) = it.next::<Dict>() else { break };
            out.push((k.as_i64(), v));
        }
    }
    if let Some(kids) = node.get::<Array>("Kids") {
        for kid in kids.iter::<Dict>() {
            collect_number_tree(&kid, out, depth + 1);
        }
    }
}

/// Labels for `n` pages from the label ranges `(first page, style, prefix, start)`.
pub fn make_labels(n: usize, ranges: &[(usize, String, String, i64)]) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    for page in 0..n {
        let r = ranges
            .iter()
            .rev()
            .find(|r| r.0 <= page)
            .unwrap_or(&ranges[0]);
        let num = r.3 + (page - r.0) as i64;
        let body = match r.1.as_str() {
            "D" => num.to_string(),
            "r" => roman(num).to_lowercase(),
            "R" => roman(num),
            "a" => letters(num).to_lowercase(),
            "A" => letters(num),
            _ => String::new(),
        };
        out.push(format!("{}{}", r.2, body));
    }
    out
}

fn roman(mut n: i64) -> String {
    if n <= 0 || n >= 4000 {
        return n.to_string();
    }
    let t = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in t {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// A, B, ... Z, AA, BB, ... (the PDF rule: the letter repeats).
fn letters(n: i64) -> String {
    if n <= 0 {
        return n.to_string();
    }
    let c = (b'A' + ((n - 1) % 26) as u8) as char;
    std::iter::repeat_n(c, ((n - 1) / 26 + 1) as usize).collect()
}

struct Extract {
    glyphs: Vec<RawGlyph>,
}

impl<'a> Device<'a> for Extract {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let glyphs = run.glyphs();
        let mut placed: Vec<(Point, f32, Option<String>)> = Vec::with_capacity(glyphs.len());
        for g in glyphs {
            let t = props.transform * g.transform();
            let [a, b, c, d, _, _] = t.as_coeffs();
            // glyph space is 1000 units per em
            let size = ((c * c + d * d).sqrt() * 1000.0).max((a * a + b * b).sqrt() * 1000.0);
            let origin = t * Point::ZERO;
            let text = g.as_unicode().map(|u| match u {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            });
            placed.push((origin, size as f32, text));
        }
        for i in 0..placed.len() {
            let (o, size, ref text) = placed[i];
            let Some(text) = text.clone() else { continue };
            if !size.is_finite() || size <= 0.0 {
                continue;
            }
            let (x, y) = (o.x as f32, o.y as f32);
            let mut width = size * 0.5;
            if let Some((n, _, _)) = placed.get(i + 1) {
                let dx = n.x as f32 - x;
                let dy = n.y as f32 - y;
                if dx > 0.0 && dy.abs() < size * 0.3 && dx < size * 3.0 {
                    width = dx;
                }
            } else if i > 0 {
                let (p, _, _) = &placed[i - 1];
                let dx = x - p.x as f32;
                if dx > 0.0 && (y - p.y as f32).abs() < size * 0.3 && dx < size * 3.0 {
                    width = dx;
                }
            }
            self.glyphs.push(RawGlyph {
                text,
                x,
                y,
                width,
                size,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpdf::{OutlineEntry, Page, Target, TestPdf};

    fn open(d: &TestPdf) -> Document {
        Document::open(d.build(), None).unwrap()
    }

    #[test]
    fn text_strings_decode_all_three_encodings() {
        assert_eq!(decode_text_string(b"Plain"), "Plain");
        assert_eq!(
            decode_text_string(&[0xFE, 0xFF, 0x00, 0x48, 0x00, 0xE9]),
            "Hé"
        );
        assert_eq!(decode_text_string(&[0xFF, 0xFE, 0x48, 0x00]), "H");
        assert_eq!(decode_text_string(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(decode_text_string(&[0x84, b'x']), "—x");
    }

    #[test]
    fn labels_roman_alpha_and_prefix() {
        let r = vec![
            (0, "r".to_string(), String::new(), 1),
            (3, "D".to_string(), String::new(), 1),
            (6, "D".to_string(), "A-".to_string(), 7),
            (8, "A".to_string(), String::new(), 1),
        ];
        let l = make_labels(10, &r);
        assert_eq!(l[..3], ["i", "ii", "iii"]);
        assert_eq!(l[3], "1");
        assert_eq!(l[5], "3");
        assert_eq!(l[6], "A-7");
        assert_eq!(l[7], "A-8");
        assert_eq!(l[8], "A");
        assert_eq!(l[9], "B");
        assert_eq!(letters(27), "AA");
        assert_eq!(roman(1994), "MCMXCIV");
    }

    #[test]
    fn metadata_pages_and_sizes() {
        let d = open(
            &TestPdf::simple(&[&["one"], &["two"]])
                .title("A Title")
                .author("Some Author"),
        );
        assert_eq!(d.page_count(), 2);
        assert_eq!(d.page_size(0), (612.0, 792.0));
        let i = d.info();
        assert_eq!(i.title, "A Title");
        assert_eq!(i.author, "Some Author");
    }

    #[test]
    fn unicode_titles_survive() {
        let d = open(&TestPdf::simple(&[&["x"]]).title("Éloge de l’ombre"));
        assert_eq!(d.info().title, "Éloge de l’ombre");
    }

    #[test]
    fn the_text_of_a_page_has_positions_in_reading_order() {
        let d = open(&TestPdf::simple(&[&["Hello world", "second line"]]));
        let t = d.text(0);
        assert_eq!(t.text(), "Page 1\nHello world\nsecond line\n");
        // 24 point heading at x=72: its box starts at 72 and sits above the first body line
        assert!((t.chars[0].rect[0] - 72.0).abs() < 0.5, "{:?}", t.chars[0]);
        assert!(t.lines[0].rect[3] < t.lines[1].rect[3]);
        assert!((t.lines[0].size - 24.0).abs() < 0.5, "{}", t.lines[0].size);
        assert!((t.lines[1].size - 12.0).abs() < 0.5, "{}", t.lines[1].size);
        // the heading is 792-740 = 52 points from the top (baseline)
        assert!(
            (t.lines[0].baseline - 52.0).abs() < 1.0,
            "{}",
            t.lines[0].baseline
        );
        // widths: Helvetica "H" is 0.722 em, so the next glyph starts about 8.7 points later
        let w = t.chars[7].rect[2] - t.chars[7].rect[0];
        assert!(w > 4.0 && w < 12.0, "{w}");
    }

    #[test]
    fn outline_nesting_and_destinations() {
        let d = open(
            &TestPdf::simple(&[&["a"], &["b"], &["c"]])
                .outline(
                    OutlineEntry::new("Part one", 0)
                        .with(OutlineEntry::new("Chapter 1", 1))
                        .with(OutlineEntry::new("Chapter 2", 2)),
                )
                .outline(OutlineEntry::new("Part two", 2)),
        );
        let o = d.outline();
        let flat: Vec<(String, usize, usize)> = o
            .iter()
            .map(|i| (i.title.clone(), i.level, i.dest.unwrap().page))
            .collect();
        assert_eq!(
            flat,
            vec![
                ("Part one".into(), 0, 0),
                ("Chapter 1".into(), 1, 1),
                ("Chapter 2".into(), 1, 2),
                ("Part two".into(), 0, 2),
            ]
        );
    }

    #[test]
    fn named_destinations_resolve() {
        let d = open(
            &TestPdf::simple(&[&["a"], &["b"]])
                .named("chapter2", 1)
                .outline(OutlineEntry {
                    title: "Second".into(),
                    to: Target::Named("chapter2".into()),
                    children: vec![],
                }),
        );
        assert_eq!(d.outline()[0].dest.map(|x| x.page), Some(1));
    }

    #[test]
    fn no_outline_is_an_empty_list() {
        assert!(open(&TestPdf::simple(&[&["a"]])).outline().is_empty());
    }

    #[test]
    fn page_labels_follow_the_number_tree() {
        let d = open(
            &TestPdf::simple(&[&["a"], &["b"], &["c"], &["d"]])
                .label(0, "r", "", 1)
                .label(2, "D", "", 1),
        );
        assert_eq!(d.page_labels(), ["i", "ii", "1", "2"]);
        let plain = open(&TestPdf::simple(&[&["a"], &["b"]]));
        assert_eq!(plain.page_labels(), ["1", "2"]);
    }

    #[test]
    fn links_to_pages_and_addresses() {
        let page0 = Page::new(612.0, 792.0)
            .text(72.0, 700.0, 12.0, "go to page two")
            .link([72.0, 690.0, 200.0, 710.0], Target::Page(1))
            .link(
                [72.0, 650.0, 200.0, 670.0],
                Target::Uri("https://example.org/x".into()),
            );
        let d = open(&TestPdf::new().page(page0).page(Page::new(612.0, 792.0)));
        let l = d.links(0);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].target, LinkTarget::Page(Dest { page: 1, y: None }));
        assert_eq!(l[1].target, LinkTarget::Uri("https://example.org/x".into()));
        // PDF y 690..710 becomes 82..102 from the top
        assert!((l[0].rect[1] - 82.0).abs() < 0.5 && (l[0].rect[3] - 102.0).abs() < 0.5);
        assert!(d.links(1).is_empty());
    }

    #[test]
    fn encrypted_documents_ask_for_a_password() {
        let bytes = TestPdf::simple(&[&["secret text"]]).password("pw1").build();
        assert_eq!(
            Document::open(bytes.clone(), None).err(),
            Some(OpenError::NeedsPassword)
        );
        assert_eq!(
            Document::open(bytes.clone(), Some("nope")).err(),
            Some(OpenError::WrongPassword)
        );
        let d = Document::open(bytes, Some("pw1")).unwrap();
        assert_eq!(d.text(0).text(), "Page 1\nsecret text\n");
    }

    #[test]
    fn broken_files_are_errors_not_panics() {
        for bytes in [
            Vec::new(),
            b"%PDF-1.4\n".to_vec(),
            b"%PDF-1.4\n1 0 obj\n<<".to_vec(),
            b"not a pdf at all".to_vec(),
            vec![0u8; 100],
        ] {
            assert!(Document::open(bytes, None).is_err());
        }
        let good = TestPdf::simple(&[&["hello"], &["world"]]).build();
        // truncation anywhere must never panic (repair may still open it)
        for cut in (0..good.len()).step_by(37) {
            if let Ok(d) = Document::open(good[..cut].to_vec(), None) {
                let _ = d.text(0);
                let _ = d.outline();
                let _ = d.links(0);
                let _ = d.page_labels();
            }
        }
    }

    #[test]
    fn a_pdf_without_xref_is_repaired() {
        let good = TestPdf::simple(&[&["repaired text"]]).build();
        let cut = good
            .windows(4)
            .rposition(|w| w == b"xref")
            .unwrap_or(good.len());
        // drop the xref table, trailer and startxref
        let d = Document::open(good[..cut].to_vec(), None);
        if let Ok(d) = d {
            assert!(d.text(0).text().contains("repaired text"));
        }
    }
}
