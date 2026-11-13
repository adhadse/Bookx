//! Embedded fonts (E1): inventory, which characters each face is used for, subsetting and embedding.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use crate::fontlayout::LayoutReport;
use anyhow::{anyhow, bail, Context, Result};
use bookx_convert::transforms::css::{parse_css, parse_decls, CssItem};
use bookx_core::book::OpenBook;
use bookx_core::util::{extension, join_path, parent_dir, relative_href, resolve_href};
use bookx_text::css::{FontStyle, StyleSheetSet};
use regex::Regex;
use scraper::{ElementRef, Html};
use serde::{Deserialize, Serialize};

use crate::markup::*;

pub const GENERIC_FAMILIES: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "emoji",
    "math",
    "fangsong",
    "inherit",
    "initial",
    "unset",
];

pub fn is_font_path(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "ttf" | "otf" | "woff" | "woff2" | "ttc"
    )
}

// ---------------------------------------------------------------------------
// @font-face
// ---------------------------------------------------------------------------

/// One `@font-face` rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontFaceDecl {
    /// As written (quotes removed).
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// Archive paths of the `url()` sources, in order (including those that do not exist).
    pub sources: Vec<String>,
    pub css_file: String,
}

pub fn parse_weight(v: &str) -> u16 {
    let first = v
        .split_whitespace()
        .next()
        .unwrap_or("normal")
        .to_ascii_lowercase();
    match first.as_str() {
        "normal" => 400,
        "bold" | "bolder" => 700,
        "lighter" => 300,
        n => n.parse::<u16>().unwrap_or(400),
    }
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim()
        .to_string()
}

fn url_in_src() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?i)url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s"']*))\s*\)"#).unwrap())
}

/// All `@font-face` rules of the stylesheets in the book.
pub fn font_face_declarations(book: &OpenBook) -> Vec<FontFaceDecl> {
    let mut out = Vec::new();
    for css_file in css_files(&book.archive) {
        let Some(css) = book.archive.get_string(&css_file) else {
            continue;
        };
        let dir = parent_dir(&css_file).to_string();
        for item in parse_css(&css) {
            let CssItem::At(s) = item else { continue };
            if !s.to_ascii_lowercase().starts_with("@font-face") {
                continue;
            }
            let Some(open) = s.find('{') else { continue };
            let decls = parse_decls(&s[open..]);
            let get = |n: &str| decls.iter().find(|d| d.0 == n).map(|d| d.1.clone());
            let Some(family) = get("font-family").map(|f| unquote(&f)) else {
                continue;
            };
            let sources = get("src")
                .map(|v| {
                    url_in_src()
                        .captures_iter(&v)
                        .filter_map(|c| {
                            c.get(1)
                                .or(c.get(2))
                                .or(c.get(3))
                                .map(|m| m.as_str().to_string())
                        })
                        .filter(|u| is_local_ref(u) && !u.starts_with("data:"))
                        .map(|u| resolve_href(&dir, &u).0)
                        .collect()
                })
                .unwrap_or_default();
            out.push(FontFaceDecl {
                family,
                weight: get("font-weight").map(|w| parse_weight(&w)).unwrap_or(400),
                italic: get("font-style")
                    .is_some_and(|s| s.starts_with("italic") || s.starts_with("oblique")),
                sources,
                css_file: css_file.clone(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// font files
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontInfo {
    pub family: Option<String>,
    pub subfamily: Option<String>,
    pub full_name: Option<String>,
    pub glyph_count: u16,
    /// `true` for OpenType with CFF outlines.
    pub cff: bool,
    pub units_per_em: u16,
}

/// Reads the names and basic facts of a TrueType/OpenType font. `None` for WOFF/WOFF2 and anything unparsable.
pub fn font_info(data: &[u8]) -> Option<FontInfo> {
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let name = |id: u16| {
        face.names()
            .into_iter()
            .filter(|n| n.name_id == id)
            .find_map(|n| n.to_string())
    };
    Some(FontInfo {
        family: name(16).or_else(|| name(1)),
        subfamily: name(17).or_else(|| name(2)),
        full_name: name(4),
        glyph_count: face.number_of_glyphs(),
        cff: face.tables().cff.is_some(),
        units_per_em: face.units_per_em(),
    })
}

/// Characters of `chars` the font has no glyph for.
pub fn uncovered_chars(data: &[u8], chars: &BTreeSet<char>) -> Vec<char> {
    match ttf_parser::Face::parse(data, 0) {
        Ok(face) => chars
            .iter()
            .copied()
            .filter(|c| face.glyph_index(*c).is_none())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// usage analysis
// ---------------------------------------------------------------------------

/// A requested face: the first family of the computed `font-family` list that the book declares (or the first
/// family when none is declared), plus weight and style.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FaceKey {
    /// Lower-cased family; empty = the reader's default font.
    pub family: String,
    pub weight: u16,
    pub italic: bool,
}

fn linked_stylesheets(book: &OpenBook, file: &str, src: &str) -> Vec<String> {
    let dir = parent_dir(file);
    let mut out = Vec::new();
    for ev in scan(src) {
        if let Event::Open { name, range, .. } = ev {
            if name == "link" {
                let tag = &src[range];
                let rel = tag_attr(tag, "rel")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if rel.contains("stylesheet") {
                    if let Some(h) = tag_attr(tag, "href") {
                        let (p, _) = resolve_href(dir, &h);
                        if book.archive.contains(&p) {
                            out.push(p);
                        }
                    }
                }
            }
        }
    }
    out
}

fn style_blocks(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_style = false;
    for ev in scan(src) {
        match ev {
            Event::Open {
                name,
                self_closing: false,
                ..
            } if name == "style" => in_style = true,
            Event::Close { name, .. } if name == "style" => in_style = false,
            Event::Text(r) if in_style => out.push(src[r].to_string()),
            _ => {}
        }
    }
    out
}

fn css_content_strings(css: &str) -> BTreeSet<char> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r#"content\s*:\s*("([^"]*)"|'([^']*)')"#).unwrap());
    re.captures_iter(css)
        .filter_map(|c| c.get(2).or(c.get(3)))
        .flat_map(|m| m.as_str().chars().collect::<Vec<_>>())
        .filter(|c| !c.is_control() && *c != '\\')
        .collect()
}

fn walk_text(
    el: ElementRef<'_>,
    sheets: &StyleSheetSet,
    parent: Option<&bookx_text::css::ComputedStyle>,
    out: &mut BTreeMap<FaceKey, BTreeSet<char>>,
    declared: &dyn Fn(&str) -> bool,
    extra_case: bool,
) {
    let style = sheets.compute_with_parent(el, parent);
    let name = el.value().name();
    if matches!(name, "script" | "style" | "head" | "title") {
        return;
    }
    let family = style
        .font_family
        .iter()
        .find(|f| declared(&f.to_ascii_lowercase()))
        .or_else(|| {
            style
                .font_family
                .iter()
                .find(|f| !GENERIC_FAMILIES.contains(&f.to_ascii_lowercase().as_str()))
        })
        .map(|f| f.to_ascii_lowercase())
        .unwrap_or_default();
    let key = FaceKey {
        family,
        weight: if style.font_weight >= 600 { 700 } else { 400 },
        italic: style.font_style != FontStyle::Normal,
    };
    for child in el.children() {
        if let Some(t) = child.value().as_text() {
            let set = out.entry(key.clone()).or_default();
            for c in t.chars() {
                if c.is_control() && c != '\u{a0}' {
                    continue;
                }
                set.insert(c);
                if extra_case || style.small_caps {
                    set.extend(c.to_uppercase());
                    set.extend(c.to_lowercase());
                }
            }
        } else if let Some(e) = ElementRef::wrap(child) {
            walk_text(e, sheets, Some(&style), out, declared, extra_case);
        }
    }
}

/// Characters rendered in each requested face, from the CSS cascade of every spine/markup document. Text with no
/// explicit family is filed under the empty family.
pub fn used_characters(book: &OpenBook) -> BTreeMap<FaceKey, BTreeSet<char>> {
    let declared: BTreeSet<String> = font_face_declarations(book)
        .iter()
        .map(|d| d.family.to_ascii_lowercase())
        .collect();
    let is_declared = |f: &str| declared.contains(f);
    let mut out: BTreeMap<FaceKey, BTreeSet<char>> = BTreeMap::new();
    let mut sheet_cache: HashMap<Vec<String>, StyleSheetSet> = HashMap::new();
    let all_css: String = css_files(&book.archive)
        .iter()
        .filter_map(|p| book.archive.get_string(p))
        .collect();
    let extra_case = all_css.contains("text-transform") || all_css.contains("small-caps");
    let content_chars = css_content_strings(&all_css);
    for file in markup_files(&book.archive) {
        let Some(src) = book.archive.get_string(&file) else {
            continue;
        };
        let links = linked_stylesheets(book, &file, &src);
        let blocks = style_blocks(&src);
        let mut key = links.clone();
        key.extend(blocks.iter().map(|b| format!("\u{0}{b}")));
        let sheets = sheet_cache.entry(key).or_insert_with(|| {
            let mut s = StyleSheetSet::new();
            for l in &links {
                if let Some(css) = book.archive.get_string(l) {
                    s = s.add_author(&css);
                }
            }
            for b in &blocks {
                s = s.add_author(b);
            }
            s
        });
        let doc = Html::parse_document(&src);
        walk_text(
            doc.root_element(),
            sheets,
            None,
            &mut out,
            &is_declared,
            extra_case,
        );
    }
    if !content_chars.is_empty() {
        for set in out.values_mut() {
            set.extend(content_chars.iter().copied());
        }
    }
    out
}

/// Picks the declared face that best matches the request (same family, closest weight, matching style).
pub fn match_declaration<'a>(
    decls: &'a [FontFaceDecl],
    family: &str,
    weight: u16,
    italic: bool,
) -> Option<&'a FontFaceDecl> {
    decls
        .iter()
        .filter(|d| d.family.eq_ignore_ascii_case(family))
        .min_by_key(|d| {
            (
                u8::from(d.italic != italic),
                (i32::from(d.weight) - i32::from(weight)).unsigned_abs(),
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedFont {
    pub path: String,
    pub size: usize,
    pub info: Option<FontInfo>,
    /// `(family, weight, italic)` of the `@font-face` rules that use the file.
    pub declared_as: Vec<(String, u16, bool)>,
    /// Characters of the book rendered with this file.
    pub used_chars: usize,
    /// Used characters the font has no glyph for.
    pub missing_glyphs: Vec<char>,
    /// No text uses this file.
    pub unused: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontsReport {
    pub embedded: Vec<EmbeddedFont>,
    /// `(css file, family, url)` of `@font-face` sources whose file is not in the archive.
    pub missing_files: Vec<(String, String, String)>,
    /// Non-generic families used in text but neither declared nor embedded, with the number of characters used.
    pub undeclared_families: BTreeMap<String, usize>,
    /// Font files in the archive that no `@font-face` refers to.
    pub orphan_files: Vec<String>,
}

pub fn analyze_fonts(book: &OpenBook) -> FontsReport {
    let decls = font_face_declarations(book);
    let usage = used_characters(book);
    let mut rep = FontsReport::default();
    let mut file_usage: BTreeMap<String, BTreeSet<char>> = BTreeMap::new();
    let mut file_decl: BTreeMap<String, Vec<(String, u16, bool)>> = BTreeMap::new();
    for d in &decls {
        let existing: Vec<&String> = d
            .sources
            .iter()
            .filter(|s| book.archive.contains(s))
            .collect();
        for s in &d.sources {
            if !book.archive.contains(s) {
                rep.missing_files
                    .push((d.css_file.clone(), d.family.clone(), s.clone()));
            }
        }
        for s in &existing {
            file_decl
                .entry((*s).clone())
                .or_default()
                .push((d.family.clone(), d.weight, d.italic));
        }
    }
    for (key, chars) in &usage {
        if key.family.is_empty() {
            continue;
        }
        match match_declaration(&decls, &key.family, key.weight, key.italic) {
            Some(d) => {
                if let Some(src) = d.sources.iter().find(|s| book.archive.contains(s)) {
                    file_usage.entry(src.clone()).or_default().extend(chars);
                }
            }
            None => {
                *rep.undeclared_families
                    .entry(key.family.clone())
                    .or_default() += chars.len();
            }
        }
    }
    for name in book.archive.names() {
        if !is_font_path(name) {
            continue;
        }
        let data = book.archive.get(name).unwrap_or(&[]);
        let used = file_usage.get(name).cloned().unwrap_or_default();
        if !file_decl.contains_key(name) {
            rep.orphan_files.push(name.clone());
        }
        rep.embedded.push(EmbeddedFont {
            path: name.clone(),
            size: data.len(),
            info: font_info(data),
            declared_as: file_decl.get(name).cloned().unwrap_or_default(),
            used_chars: used.len(),
            missing_glyphs: uncovered_chars(data, &used),
            unused: used.is_empty(),
        });
    }
    rep
}

// ---------------------------------------------------------------------------
// subsetting
// ---------------------------------------------------------------------------

pub(crate) fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

pub(crate) fn table_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut b = [0u8; 4];
        b[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(b));
    }
    sum
}

/// Builds a `cmap` table with one Unicode subtable: format 4 when every code point is in the BMP, format 12
/// otherwise. `map` is code point -> glyph id.
pub fn build_cmap(map: &BTreeMap<u32, u16>) -> Vec<u8> {
    let bmp = map.keys().all(|c| *c <= 0xFFFF);
    let mut sub = Vec::new();
    if bmp {
        // one segment per run of consecutive code points with consecutive glyph ids
        let mut segs: Vec<(u16, u16, u16)> = Vec::new(); // start, end, delta
        for (&c, &g) in map {
            let c = c as u16;
            match segs.last_mut() {
                Some((_, end, delta))
                    if *end != 0xFFFF
                        && c == end.wrapping_add(1)
                        && g.wrapping_sub(c) == *delta =>
                {
                    *end = c;
                }
                _ => segs.push((c, c, g.wrapping_sub(c))),
            }
        }
        if segs.last().map(|s| s.1) != Some(0xFFFF) {
            segs.push((0xFFFF, 0xFFFF, 1));
        }
        let n = segs.len() as u16;
        let mut entry_selector = 0u16;
        while (1u32 << (entry_selector + 1)) <= u32::from(n) {
            entry_selector += 1;
        }
        let search_range = 2u16 << entry_selector;
        let length = 16 + 8 * n;
        sub.extend(be16(4));
        sub.extend(be16(length));
        sub.extend(be16(0));
        sub.extend(be16(n * 2));
        sub.extend(be16(search_range));
        sub.extend(be16(entry_selector));
        sub.extend(be16(n * 2 - search_range));
        for s in &segs {
            sub.extend(be16(s.1));
        }
        sub.extend(be16(0));
        for s in &segs {
            sub.extend(be16(s.0));
        }
        for s in &segs {
            sub.extend(be16(s.2));
        }
        for _ in &segs {
            sub.extend(be16(0));
        }
    } else {
        let mut groups: Vec<(u32, u32, u32)> = Vec::new();
        for (&c, &g) in map {
            let g = u32::from(g);
            match groups.last_mut() {
                Some((sc, ec, sg)) if c == *ec + 1 && g == *sg + (c - *sc) => *ec = c,
                _ => groups.push((c, c, g)),
            }
        }
        sub.extend(be16(12));
        sub.extend(be16(0));
        sub.extend(be32(16 + 12 * groups.len() as u32));
        sub.extend(be32(0));
        sub.extend(be32(groups.len() as u32));
        for (s, e, g) in groups {
            sub.extend(be32(s));
            sub.extend(be32(e));
            sub.extend(be32(g));
        }
    }
    let (platform, encoding) = if bmp { (3u16, 1u16) } else { (3, 10) };
    let mut t = Vec::new();
    t.extend(be16(0));
    t.extend(be16(1));
    t.extend(be16(platform));
    t.extend(be16(encoding));
    t.extend(be32(12));
    t.extend(sub);
    t
}

/// Rebuilds an sfnt file with extra/replaced tables, fixing the table directory, checksums and `head`.
fn rebuild_sfnt(data: &[u8], extra: &[([u8; 4], Vec<u8>)]) -> Result<Vec<u8>> {
    if data.len() < 12 {
        bail!("font data too short");
    }
    let num = u16::from_be_bytes([data[4], data[5]]) as usize;
    let mut tables: BTreeMap<[u8; 4], Vec<u8>> = BTreeMap::new();
    for i in 0..num {
        let rec = data
            .get(12 + i * 16..12 + i * 16 + 16)
            .ok_or_else(|| anyhow!("truncated table directory"))?;
        let tag = [rec[0], rec[1], rec[2], rec[3]];
        let off = u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]) as usize;
        let len = u32::from_be_bytes([rec[12], rec[13], rec[14], rec[15]]) as usize;
        let body = data
            .get(off..off + len)
            .ok_or_else(|| anyhow!("table outside of the file"))?;
        tables.insert(tag, body.to_vec());
    }
    for (tag, body) in extra {
        tables.insert(*tag, body.clone());
    }
    let n = tables.len() as u16;
    let mut entry_selector = 0u16;
    while (1u32 << (entry_selector + 1)) <= u32::from(n) {
        entry_selector += 1;
    }
    let search_range = 16u16 << entry_selector;
    let mut out = Vec::new();
    out.extend_from_slice(&data[0..4]);
    out.extend(be16(n));
    out.extend(be16(search_range));
    out.extend(be16(entry_selector));
    out.extend(be16(n * 16 - search_range));
    let offset = 12 + tables.len() * 16;
    let mut body = Vec::new();
    let mut head_pos = None;
    for (tag, t) in &mut tables {
        if tag == b"head" && t.len() >= 12 {
            t[8..12].fill(0);
            head_pos = Some(body.len() + 8);
        }
        out.extend_from_slice(tag);
        out.extend(be32(table_checksum(t)));
        out.extend(be32((offset + body.len()) as u32));
        out.extend(be32(t.len() as u32));
        body.extend_from_slice(t);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    out.extend(body);
    if let Some(p) = head_pos {
        let adj = 0xB1B0_AFBAu32.wrapping_sub(table_checksum(&out));
        let at = 12 + tables.len() * 16 + p;
        out[at..at + 4].copy_from_slice(&be32(adj));
    }
    Ok(out)
}

/// Subsets a TrueType or OpenType/CFF font to the glyphs of `chars` (plus `.notdef`). The result keeps a Unicode
/// `cmap` (the subsetting library drops it), so it works as a web font. Fails for WOFF/WOFF2 and fonts without
/// outlines.
pub fn subset_font_data(data: &[u8], chars: &BTreeSet<char>) -> Result<Vec<u8>> {
    subset_font_report(data, chars).map(|r| r.0)
}

/// Like [`subset_font_data`], and also keeps kerning (`kern`, `GPOS` pair adjustment) and ligatures (`GSUB`
/// type 4) for the kept glyphs, see [`crate::fontlayout`]. Ligature glyphs whose components are all used are
/// kept too. The report lists what was kept and which lookups could not be preserved.
pub fn subset_font_report(data: &[u8], chars: &BTreeSet<char>) -> Result<(Vec<u8>, LayoutReport)> {
    let face = ttf_parser::Face::parse(data, 0).map_err(|e| {
        anyhow!("cannot read the font ({e}); only TrueType and OpenType files can be subset")
    })?;
    let mut remap = subsetter::GlyphRemapper::new();
    remap.remap(0);
    let mut wanted: BTreeMap<u32, u16> = BTreeMap::new();
    let mut present: BTreeSet<u16> = BTreeSet::from([0]);
    for &c in chars {
        if let Some(g) = face.glyph_index(c) {
            remap.remap(g.0);
            wanted.insert(c as u32, g.0);
            present.insert(g.0);
        }
    }
    let base = present.clone();
    crate::fontlayout::ligature_closure(data, &mut present);
    for g in present.difference(&base) {
        remap.remap(*g);
    }
    let sub = subsetter::subset(data, 0, &remap).map_err(|e| anyhow!("subsetting failed: {e}"))?;
    let mut cmap_map: BTreeMap<u32, u16> = BTreeMap::new();
    for (c, g) in wanted {
        if let Some(new) = remap.get(g) {
            cmap_map.insert(c, new);
        }
    }
    let mut extra: Vec<([u8; 4], Vec<u8>)> = vec![(*b"cmap", build_cmap(&cmap_map))];
    // OS/2 (kept verbatim) is needed by some renderers; the subsetter drops it
    let num = u16::from_be_bytes([data[4], data[5]]) as usize;
    for i in 0..num {
        let rec = &data[12 + i * 16..12 + i * 16 + 16];
        if &rec[0..4] == b"OS/2" {
            let off = u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]) as usize;
            let len = u32::from_be_bytes([rec[12], rec[13], rec[14], rec[15]]) as usize;
            if let Some(b) = data.get(off..off + len) {
                extra.push((*b"OS/2", b.to_vec()));
            }
        }
    }
    let (layout, report) = crate::fontlayout::subset_layout(data, &|g| remap.get(g));
    extra.extend(layout);
    Ok((rebuild_sfnt(&sub, &extra)?, report))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsetResult {
    pub path: String,
    pub before: usize,
    pub after: usize,
    pub chars: usize,
    /// Kerning and ligatures kept, and the lookups that could not be preserved.
    #[serde(default)]
    pub layout: LayoutReport,
}

/// Subsets every embedded TrueType/OpenType font to the characters the book renders with it. Fonts that are
/// unused, unreadable (WOFF) or would not get smaller are left as they are. Returns what was changed.
pub fn subset_embedded_fonts(book: &mut OpenBook) -> Vec<SubsetResult> {
    let decls = font_face_declarations(book);
    let usage = used_characters(book);
    let mut per_file: BTreeMap<String, BTreeSet<char>> = BTreeMap::new();
    for (key, chars) in &usage {
        if key.family.is_empty() {
            continue;
        }
        if let Some(d) = match_declaration(&decls, &key.family, key.weight, key.italic) {
            if let Some(s) = d.sources.iter().find(|s| book.archive.contains(s)) {
                per_file.entry(s.clone()).or_default().extend(chars);
            }
        }
    }
    let mut results = Vec::new();
    for (path, mut chars) in per_file {
        chars.insert(' ');
        let data = book.archive.get(&path).unwrap_or(&[]).to_vec();
        if let Ok((new, layout)) = subset_font_report(&data, &chars) {
            if new.len() < data.len() {
                book.archive.set(&path, new.clone());
                results.push(SubsetResult {
                    path,
                    before: data.len(),
                    after: new.len(),
                    chars: chars.len(),
                    layout,
                });
            }
        }
    }
    results
}

// ---------------------------------------------------------------------------
// embedding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedResult {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub path: String,
    pub bytes: usize,
    pub subset: bool,
}

fn font_media_type(data: &[u8]) -> (&'static str, &'static str) {
    if data.starts_with(b"OTTO") {
        ("otf", "font/otf")
    } else if data.starts_with(b"wOFF") {
        ("woff", "font/woff")
    } else if data.starts_with(b"wOF2") {
        ("woff2", "font/woff2")
    } else {
        ("ttf", "font/ttf")
    }
}

/// Adds one font to the book: file, manifest item and an `@font-face` rule in the first stylesheet.
pub fn embed_font(
    book: &mut OpenBook,
    family: &str,
    weight: u16,
    italic: bool,
    data: Vec<u8>,
) -> Result<EmbedResult> {
    let css_path = css_files(&book.archive)
        .into_iter()
        .next()
        .context("the book has no stylesheet to hold the @font-face rule")?;
    let (ext, media) = font_media_type(&data);
    let base = book.package.base_dir().to_string();
    let safe: String = family
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let style = match (weight, italic) {
        (700, false) => "Bold".to_string(),
        (700, true) => "BoldItalic".to_string(),
        (_, true) => "Italic".to_string(),
        (400, false) => "Regular".to_string(),
        (w, false) => w.to_string(),
    };
    let mut path = join_path(&base, &format!("fonts/{safe}-{style}.{ext}"));
    let mut n = 1;
    while book.archive.contains(&path) {
        n += 1;
        path = join_path(&base, &format!("fonts/{safe}-{style}-{n}.{ext}"));
    }
    let bytes = data.len();
    book.archive.set(&path, data);
    let mut id = format!("font-{}", safe.to_ascii_lowercase());
    let mut k = 1;
    while book.package.item_by_id(&id).is_some() {
        k += 1;
        id = format!("font-{}-{k}", safe.to_ascii_lowercase());
    }
    let pkg = book.package.clone();
    pkg.add_manifest_item(&mut book.archive, &id, &path, media, "")?;
    book.reload_package()?;
    let rule = format!(
        "\n@font-face {{\n  font-family: \"{}\";\n  font-style: {};\n  font-weight: {};\n  src: url({});\n}}\n",
        family.replace('"', ""),
        if italic { "italic" } else { "normal" },
        match weight {
            400 => "normal".to_string(),
            700 => "bold".to_string(),
            w => w.to_string(),
        },
        relative_href(&css_path, &path)
    );
    let mut css = book.archive.get_string(&css_path).unwrap_or_default();
    if !css.ends_with('\n') && !css.is_empty() {
        css.push('\n');
    }
    css.push_str(&rule);
    book.archive.set_string(&css_path, &css);
    Ok(EmbedResult {
        family: family.to_string(),
        weight,
        italic,
        path,
        bytes,
        subset: false,
    })
}

/// `(family lower-cased, weight, italic)` -> font file bytes, supplied by the application.
pub type FontLookup<'a> = dyn Fn(&str, u16, bool) -> Option<Vec<u8>> + 'a;

/// Embeds the fonts the book asks for but does not carry. `lookup(family, weight, italic)` supplies font bytes
/// (the caller searches the system); returning `None` skips that family. With `subset`, only the used
/// characters are kept.
pub fn embed_referenced_fonts(
    book: &mut OpenBook,
    lookup: &FontLookup<'_>,
    subset: bool,
) -> Result<Vec<EmbedResult>> {
    let usage = used_characters(book);
    let decls = font_face_declarations(book);
    let mut out = Vec::new();
    for (key, chars) in &usage {
        if key.family.is_empty()
            || GENERIC_FAMILIES.contains(&key.family.as_str())
            || match_declaration(&decls, &key.family, key.weight, key.italic).is_some()
        {
            continue;
        }
        let Some(data) = lookup(&key.family, key.weight, key.italic) else {
            continue;
        };
        let mut chars = chars.clone();
        chars.insert(' ');
        let (data, did_subset) = if subset {
            match subset_font_data(&data, &chars) {
                Ok(s) => (s, true),
                Err(_) => (data, false),
            }
        } else {
            (data, false)
        };
        let mut r = embed_font(book, &key.family, key.weight, key.italic, data)?;
        r.subset = did_subset;
        out.push(r);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// system fonts and embedding rights
// ---------------------------------------------------------------------------

/// What the font's OS/2 `fsType` says about embedding it in a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Embedding {
    /// May be embedded and installed by the reader.
    Installable,
    /// May be embedded and edited (the document may be modified).
    Editable,
    /// May be embedded for viewing and printing only.
    Preview,
    /// The licence forbids embedding.
    Restricted,
    /// No OS/2 table or unreadable: nothing is known.
    Unknown,
}

impl Embedding {
    /// Embedding is allowed (installable, editable or preview and print).
    pub fn allows_embedding(self) -> bool {
        matches!(
            self,
            Embedding::Installable | Embedding::Editable | Embedding::Preview
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Embedding::Installable => "Embedding allowed",
            Embedding::Editable => "Embedding allowed (editable documents)",
            Embedding::Preview => "Embedding allowed for viewing and printing only",
            Embedding::Restricted => "The font's licence forbids embedding",
            Embedding::Unknown => "The font does not say whether it may be embedded",
        }
    }
}

/// Reads the embedding permission of a TrueType/OpenType font.
pub fn embedding_permission(data: &[u8]) -> Embedding {
    use ttf_parser::Permissions;
    let Ok(face) = ttf_parser::Face::parse(data, 0) else {
        return Embedding::Unknown;
    };
    match face.permissions() {
        Some(Permissions::Installable) => Embedding::Installable,
        Some(Permissions::Editable) => Embedding::Editable,
        Some(Permissions::PreviewAndPrint) => Embedding::Preview,
        Some(Permissions::Restricted) => Embedding::Restricted,
        None => Embedding::Unknown,
    }
}

/// A font file found on the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFont {
    pub path: std::path::PathBuf,
    /// Lower-cased family name.
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub embedding: Embedding,
}

/// The folders fonts are installed in; `$BOOKX_FONT_DIRS` (colon separated) replaces the list (tests).
pub fn system_font_dirs() -> Vec<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("BOOKX_FONT_DIRS") {
        return std::env::split_paths(&d).collect();
    }
    let mut v = vec![
        std::path::PathBuf::from("/usr/share/fonts"),
        std::path::PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        v.push(home.join(".local/share/fonts"));
        v.push(home.join(".fonts"));
    }
    v
}

fn walk_fonts(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 6 {
                walk_fonts(&p, depth + 1, out);
            }
        } else if matches!(
            p.extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("ttf" | "otf")
        ) {
            out.push(p);
        }
    }
}

/// Reads the family, weight and style of the font files under `dirs` (TrueType and OpenType; collections and web
/// fonts are skipped).
pub fn scan_system_fonts(dirs: &[std::path::PathBuf]) -> Vec<SystemFont> {
    let mut files = Vec::new();
    for d in dirs {
        walk_fonts(d, 0, &mut files);
    }
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let Ok(face) = ttf_parser::Face::parse(&data, 0) else {
            continue;
        };
        let Some(family) = font_info(&data).and_then(|i| i.family) else {
            continue;
        };
        out.push(SystemFont {
            path,
            family: family.to_ascii_lowercase(),
            weight: face.weight().to_number(),
            italic: face.is_italic() || face.is_oblique(),
            embedding: embedding_permission(&data),
        });
    }
    out
}

/// The installed face closest to the request: the family must match; then the same style, then the nearest weight.
pub fn best_system_font<'a>(
    fonts: &'a [SystemFont],
    family: &str,
    weight: u16,
    italic: bool,
) -> Option<&'a SystemFont> {
    let fam = family.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    fonts.iter().filter(|f| f.family == fam).min_by_key(|f| {
        (
            f.italic != italic,
            (f.weight as i32 - weight as i32).unsigned_abs(),
        )
    })
}

/// A family the pages ask for but the book does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFace {
    pub key: FaceKey,
    pub chars: usize,
}

/// Faces used in the book that have no `@font-face` declaration (generic families excluded).
pub fn missing_faces(book: &OpenBook) -> Vec<MissingFace> {
    let decls = font_face_declarations(book);
    used_characters(book)
        .into_iter()
        .filter(|(k, _)| {
            !k.family.is_empty()
                && !GENERIC_FAMILIES.contains(&k.family.as_str())
                && match_declaration(&decls, &k.family, k.weight, k.italic).is_none()
        })
        .map(|(key, chars)| MissingFace {
            key,
            chars: chars.len(),
        })
        .collect()
}

/// Deletes embedded font files that no text uses, together with their `@font-face` rules and manifest items.
/// Returns the removed paths.
pub fn remove_unused_fonts(book: &mut OpenBook) -> Result<Vec<String>> {
    let rep = analyze_fonts(book);
    let mut removed = Vec::new();
    for f in rep.embedded.iter().filter(|f| f.unused) {
        if let Some(item) = book.package.item_by_path(&f.path).cloned() {
            let pkg = book.package.clone();
            pkg.remove_manifest_item(&mut book.archive, &item.id)?;
            book.reload_package()?;
        }
        book.archive.remove(&f.path);
        removed.push(f.path.clone());
    }
    if !removed.is_empty() {
        for css_path in css_files(&book.archive) {
            let css = book.archive.get_string(&css_path).unwrap_or_default();
            let dir = parent_dir(&css_path);
            let mut out = String::new();
            for item in parse_css(&css) {
                match item {
                    CssItem::At(s) if s.to_ascii_lowercase().starts_with("@font-face") => {
                        let gone = url_in_src().captures_iter(s).any(|c| {
                            c.get(1)
                                .or(c.get(2))
                                .or(c.get(3))
                                .is_some_and(|m| removed.contains(&resolve_href(dir, m.as_str()).0))
                        });
                        if !gone {
                            out.push_str(s);
                        }
                    }
                    CssItem::At(s) | CssItem::Other(s) => out.push_str(s),
                    CssItem::Rule { selectors, block } => {
                        out.push_str(selectors);
                        out.push_str(block);
                    }
                }
            }
            if out != css {
                book.archive.set_string(&css_path, &out);
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
pub(crate) mod tests {

    /// The test font with its OS/2 `fsType` set to `fs_type`.
    fn font_with_fs_type(fs_type: u16) -> Vec<u8> {
        let mut data = crate::testfont::tiny_ttf(0);
        let n = u16::from_be_bytes([data[4], data[5]]) as usize;
        for k in 0..n {
            let rec = 12 + k * 16;
            if &data[rec..rec + 4] == b"OS/2" {
                let off = u32::from_be_bytes(data[rec + 8..rec + 12].try_into().unwrap()) as usize;
                data[off + 8..off + 10].copy_from_slice(&fs_type.to_be_bytes());
            }
        }
        data
    }

    #[test]
    fn embedding_permission_from_fs_type() {
        assert_eq!(
            embedding_permission(&font_with_fs_type(0)),
            Embedding::Installable
        );
        assert_eq!(
            embedding_permission(&font_with_fs_type(2)),
            Embedding::Restricted
        );
        assert_eq!(
            embedding_permission(&font_with_fs_type(4)),
            Embedding::Preview
        );
        assert_eq!(
            embedding_permission(&font_with_fs_type(8)),
            Embedding::Editable
        );
        assert_eq!(embedding_permission(b"not a font"), Embedding::Unknown);
        assert!(Embedding::Preview.allows_embedding());
        assert!(!Embedding::Restricted.allows_embedding());
        assert!(!Embedding::Unknown.allows_embedding());
    }

    #[test]
    fn scans_and_matches_system_fonts() {
        let dir = std::env::temp_dir().join(format!("bookx-fonts-{}", std::process::id()));
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("tiny.ttf"), font_with_fs_type(2)).unwrap();
        std::fs::write(dir.join("junk.ttf"), b"junk").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        let fonts = scan_system_fonts(std::slice::from_ref(&dir));
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].family, "tiny test");
        assert_eq!(fonts[0].embedding, Embedding::Restricted);
        assert!(best_system_font(&fonts, "Tiny Test", 700, true).is_some());
        assert!(best_system_font(&fonts, "\"tiny test\"", 400, false).is_some());
        assert!(best_system_font(&fonts, "Other", 400, false).is_none());
        assert!(scan_system_fonts(&[dir.join("nothing")]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_faces_ignores_generic_and_declared_families() {
        let mut b = crate::testutil::make_book(&[
            "<p style=\"font-family: Zapf\">abc</p><p style=\"font-family: serif\">x</p>",
        ]);
        let m = missing_faces(&b);
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(m[0].key.family, "zapf");
        let r = embed_referenced_fonts(
            &mut b,
            &|fam, _, _| (fam == "zapf").then(|| crate::testfont::tiny_ttf(0)),
            false,
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        assert!(missing_faces(&b).is_empty());
    }
    use super::*;
    use crate::testfont::tiny_ttf;
    use crate::testutil::make_book;

    fn charset(s: &str) -> BTreeSet<char> {
        s.chars().collect()
    }

    #[test]
    fn test_font_is_a_valid_font() {
        let f = tiny_ttf(0);
        let face = ttf_parser::Face::parse(&f, 0).unwrap();
        assert_eq!(face.number_of_glyphs(), 6);
        assert_eq!(face.glyph_index('A').unwrap().0, 1);
        assert_eq!(face.glyph_index('Á').unwrap().0, 5);
        assert!(face.glyph_index('Z').is_none());
        let mut sink = Sink(0);
        assert!(face
            .outline_glyph(ttf_parser::GlyphId(5), &mut sink)
            .is_some());
        assert!(sink.0 >= 7, "composite has the points of both components");
        let info = font_info(&f).unwrap();
        assert_eq!(info.family.as_deref(), Some("Tiny Test"));
        assert_eq!(info.subfamily.as_deref(), Some("Regular"));
        assert_eq!(info.full_name.as_deref(), Some("Tiny Test Regular"));
        assert_eq!(info.glyph_count, 6);
        assert!(!info.cff);
        assert_eq!(info.units_per_em, 1000);
        assert!(font_info(b"garbage").is_none());
        assert!(font_info(b"wOFF....").is_none());
    }

    struct Sink(usize);
    impl ttf_parser::OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn line_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
            self.0 += 1;
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
            self.0 += 1;
        }
        fn close(&mut self) {}
    }

    #[test]
    fn subset_keeps_exactly_the_used_glyphs() {
        let f = tiny_ttf(300);
        let sub = subset_font_data(&f, &charset("AB")).unwrap();
        assert!(sub.len() < f.len() / 3, "{} -> {}", f.len(), sub.len());
        let face = ttf_parser::Face::parse(&sub, 0).unwrap();
        assert_eq!(face.number_of_glyphs(), 3, ".notdef + A + B");
        let a = face.glyph_index('A').unwrap();
        let b = face.glyph_index('B').unwrap();
        assert_ne!(a, b);
        assert!(face.glyph_index('C').is_none());
        assert!(face.glyph_index('\u{401}').is_none());
        let mut s = Sink(0);
        assert!(face.outline_glyph(a, &mut s).is_some());
        assert_eq!(face.glyph_hor_advance(a), Some(600));
        assert_eq!(face.units_per_em(), 1000);
        assert_eq!(
            face.names()
                .into_iter()
                .filter(|n| n.name_id == 1)
                .find_map(|n| n.to_string())
                .as_deref(),
            Some("Tiny Test")
        );
        assert!(face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"OS/2"))
            .is_some());
    }

    #[test]
    fn subset_pulls_in_composite_components() {
        let f = tiny_ttf(0);
        let sub = subset_font_data(&f, &charset("\u{c1}")).unwrap();
        let face = ttf_parser::Face::parse(&sub, 0).unwrap();
        assert_eq!(
            face.number_of_glyphs(),
            4,
            ".notdef + Á + its components A and accent"
        );
        let g = face.glyph_index('\u{c1}').unwrap();
        let mut s = Sink(0);
        assert!(face.outline_glyph(g, &mut s).is_some());
        assert!(s.0 >= 7);
    }

    #[test]
    fn subset_ignores_unknown_chars_and_rejects_bad_data() {
        let f = tiny_ttf(0);
        let sub = subset_font_data(&f, &charset("Zz€")).unwrap();
        assert_eq!(
            ttf_parser::Face::parse(&sub, 0).unwrap().number_of_glyphs(),
            1
        );
        let e = subset_font_data(b"wOFFxxxxxxxxxxxx", &charset("a")).unwrap_err();
        assert!(e.to_string().contains("only TrueType and OpenType"), "{e}");
    }

    #[test]
    fn cmap_builder_formats() {
        let mut m = BTreeMap::new();
        m.insert(0x41u32, 1u16);
        m.insert(0x42, 2);
        m.insert(0x44, 7);
        let t = build_cmap(&m);
        assert_eq!(&t[0..4], &[0, 0, 0, 1]);
        assert_eq!(&t[4..8], &[0, 3, 0, 1]);
        assert_eq!(u16::from_be_bytes([t[12], t[13]]), 4);
        m.insert(0x1F600, 9);
        let t = build_cmap(&m);
        assert_eq!(&t[4..8], &[0, 3, 0, 10]);
        assert_eq!(u16::from_be_bytes([t[12], t[13]]), 12);
        // ttf-parser reads both back through a rebuilt font
        let f = tiny_ttf(0);
        let mut map = BTreeMap::new();
        map.insert('A' as u32, 1u16);
        map.insert('B' as u32, 2);
        map.insert(0x1F600, 3);
        let rebuilt = rebuild_sfnt(&f, &[(*b"cmap", build_cmap(&map))]).unwrap();
        let face = ttf_parser::Face::parse(&rebuilt, 0).unwrap();
        assert_eq!(face.glyph_index('A').unwrap().0, 1);
        assert_eq!(face.glyph_index('\u{1F600}').unwrap().0, 3);
        assert!(rebuild_sfnt(b"short", &[]).is_err());
        let mut trunc = f.clone();
        trunc.truncate(30);
        assert!(rebuild_sfnt(&trunc, &[]).is_err());
    }

    #[test]
    fn head_checksum_adjustment_is_valid() {
        let f = tiny_ttf(0);
        let sub = subset_font_data(&f, &charset("A")).unwrap();
        assert_eq!(
            table_checksum(&sub),
            0xB1B0_AFBA,
            "whole-file checksum including head.checkSumAdjustment"
        );
    }

    #[test]
    fn weights() {
        assert_eq!(parse_weight("bold"), 700);
        assert_eq!(parse_weight("NORMAL"), 400);
        assert_eq!(parse_weight("300 900"), 300);
        assert_eq!(parse_weight("lighter"), 300);
        assert_eq!(parse_weight("weird"), 400);
    }

    fn font_book() -> OpenBook {
        let mut b = make_book(&[
            "<h1>AAA</h1><p>AB B <em>CA</em></p><p style=\"font-family: Missing Sans, sans-serif\">xyz</p>",
            "<p>plain text</p>",
        ]);
        b.archive.set("OEBPS/fonts/tiny.ttf", tiny_ttf(300));
        b.archive.set("OEBPS/fonts/unused.ttf", tiny_ttf(0));
        b.archive.set_string(
            "OEBPS/style.css",
            "@font-face { font-family: \"Tiny\"; src: url(fonts/tiny.ttf) format(\"truetype\"); }\n@font-face { font-family: Unused; src: url('fonts/unused.ttf'); font-weight: bold }\n@font-face { font-family: Gone; src: url(fonts/gone.ttf) }\nh1, p { font-family: Tiny, serif }\nem { font-style: italic }\n",
        );
        let pkg = b.package.clone();
        pkg.add_manifest_item(
            &mut b.archive,
            "tiny",
            "OEBPS/fonts/tiny.ttf",
            "font/ttf",
            "",
        )
        .unwrap();
        pkg.add_manifest_item(
            &mut b.archive,
            "unused",
            "OEBPS/fonts/unused.ttf",
            "font/ttf",
            "",
        )
        .unwrap();
        b.reload_package().unwrap();
        b
    }

    #[test]
    fn declarations_parsed() {
        let b = font_book();
        let d = font_face_declarations(&b);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].family, "Tiny");
        assert_eq!(d[0].sources, vec!["OEBPS/fonts/tiny.ttf"]);
        assert_eq!(d[1].weight, 700);
        assert_eq!(d[1].sources, vec!["OEBPS/fonts/unused.ttf"]);
        assert!(!d[0].italic);
    }

    #[test]
    fn usage_follows_the_cascade() {
        let b = font_book();
        let u = used_characters(&b);
        let normal = u
            .get(&FaceKey {
                family: "tiny".into(),
                weight: 400,
                italic: false,
            })
            .unwrap();
        assert!(normal.contains(&'A') && normal.contains(&'B') && normal.contains(&'p'));
        assert!(!normal.contains(&'C'), "C only appears in the italic <em>");
        let italic = u
            .get(&FaceKey {
                family: "tiny".into(),
                weight: 400,
                italic: true,
            })
            .unwrap();
        assert_eq!(
            italic.iter().collect::<String>(),
            "ACA"
                .chars()
                .collect::<BTreeSet<_>>()
                .iter()
                .collect::<String>()
        );
        let missing = u
            .get(&FaceKey {
                family: "missing sans".into(),
                weight: 400,
                italic: false,
            })
            .unwrap();
        assert_eq!(missing.iter().collect::<String>(), "xyz");
        let bold = u
            .get(&FaceKey {
                family: "tiny".into(),
                weight: 700,
                italic: false,
            })
            .unwrap();
        assert_eq!(
            bold.iter().collect::<String>(),
            "A",
            "<h1> is bold by the built-in stylesheet"
        );
    }

    #[test]
    fn analysis_reports_missing_unused_and_undeclared() {
        let b = font_book();
        let r = analyze_fonts(&b);
        assert_eq!(r.embedded.len(), 2);
        let tiny = r
            .embedded
            .iter()
            .find(|f| f.path.ends_with("tiny.ttf"))
            .unwrap();
        assert!(!tiny.unused);
        assert!(tiny.used_chars > 5);
        assert_eq!(tiny.declared_as, vec![("Tiny".to_string(), 400, false)]);
        assert!(
            tiny.missing_glyphs.contains(&'p'),
            "the tiny font has no 'p': {:?}",
            tiny.missing_glyphs
        );
        assert!(!tiny.missing_glyphs.contains(&'A'));
        assert_eq!(
            tiny.info.as_ref().unwrap().family.as_deref(),
            Some("Tiny Test")
        );
        let unused = r
            .embedded
            .iter()
            .find(|f| f.path.ends_with("unused.ttf"))
            .unwrap();
        assert!(unused.unused);
        assert_eq!(
            r.missing_files,
            vec![(
                "OEBPS/style.css".to_string(),
                "Gone".to_string(),
                "OEBPS/fonts/gone.ttf".to_string()
            )]
        );
        assert_eq!(r.undeclared_families.get("missing sans"), Some(&3));
        assert!(!r.undeclared_families.contains_key("serif"));
        assert!(r.orphan_files.is_empty());
        let mut c = font_book();
        c.archive.set("OEBPS/fonts/stray.otf", vec![1, 2, 3]);
        assert_eq!(
            analyze_fonts(&c).orphan_files,
            vec!["OEBPS/fonts/stray.otf"]
        );
    }

    #[test]
    fn subset_embedded_fonts_in_book() {
        let mut b = font_book();
        let before = b.archive.size_of("OEBPS/fonts/tiny.ttf");
        let res = subset_embedded_fonts(&mut b);
        assert_eq!(res.len(), 1, "unused font untouched");
        assert_eq!(res[0].path, "OEBPS/fonts/tiny.ttf");
        assert_eq!(res[0].before, before);
        assert!(res[0].after < before / 2);
        let face =
            ttf_parser::Face::parse(b.archive.get("OEBPS/fonts/tiny.ttf").unwrap(), 0).unwrap();
        assert!(face.glyph_index('A').is_some() && face.glyph_index('C').is_some());
        assert!(face.glyph_index('\u{450}').is_none());
        // running again does not shrink further
        assert!(subset_embedded_fonts(&mut b).is_empty());
        assert_eq!(
            b.archive.size_of("OEBPS/fonts/unused.ttf"),
            tiny_ttf(0).len()
        );
    }

    #[test]
    fn embed_font_adds_file_manifest_and_rule() {
        let mut b = make_book(&["<p>x</p>"]);
        let r = embed_font(&mut b, "My \"Font\"", 700, true, tiny_ttf(0)).unwrap();
        assert_eq!(r.path, "OEBPS/fonts/My--Font--BoldItalic.ttf");
        let item = b.package.item_by_path(&r.path).unwrap();
        assert_eq!(item.media_type, "font/ttf");
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(css.contains("font-family: \"My Font\""));
        assert!(css.contains("font-style: italic"));
        assert!(css.contains("font-weight: bold"));
        assert!(css.contains("src: url(fonts/My--Font--BoldItalic.ttf)"));
        let d = font_face_declarations(&b);
        assert_eq!(d.last().unwrap().sources, vec![r.path.clone()]);
        let r2 = embed_font(&mut b, "My \"Font\"", 700, true, b"OTTO....".to_vec()).unwrap();
        assert!(r2.path.ends_with(".otf"));
        assert_eq!(
            b.package.item_by_path(&r2.path).unwrap().media_type,
            "font/otf"
        );
        let mut no_css = make_book(&["<p>x</p>"]);
        no_css.archive.remove("OEBPS/style.css");
        assert!(embed_font(&mut no_css, "F", 400, false, vec![0])
            .unwrap_err()
            .to_string()
            .contains("no stylesheet"));
    }

    #[test]
    fn embed_referenced_uses_lookup_and_subsets() {
        let mut b = make_book(&["<p style=\"font-family: 'Tiny Test', serif\">ABAB</p><p style=\"font-family: Other\">q</p>"]);
        let big = tiny_ttf(300);
        let calls = std::cell::RefCell::new(Vec::new());
        let lookup = |fam: &str, w: u16, i: bool| {
            calls.borrow_mut().push((fam.to_string(), w, i));
            (fam == "tiny test").then(|| big.clone())
        };
        let r = embed_referenced_fonts(&mut b, &lookup, true).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].subset);
        assert!(r[0].bytes < big.len() / 3);
        assert!(calls.borrow().contains(&("other".to_string(), 400, false)));
        let face = ttf_parser::Face::parse(b.archive.get(&r[0].path).unwrap(), 0).unwrap();
        assert!(face.glyph_index('A').is_some());
        // now declared: a second run embeds nothing
        let again = embed_referenced_fonts(&mut b, &|_, _, _| Some(big.clone()), false).unwrap();
        assert!(again.iter().all(|x| x.family != "tiny test"));
        // without subsetting, the whole font goes in
        let mut c = make_book(&["<p style=\"font-family: Tiny\">A</p>"]);
        let r = embed_referenced_fonts(&mut c, &|_, _, _| Some(big.clone()), false).unwrap();
        assert_eq!(r[0].bytes, big.len());
        assert!(!r[0].subset);
        // unsubsettable data is embedded as is
        let mut d = make_book(&["<p style=\"font-family: Tiny\">A</p>"]);
        let r =
            embed_referenced_fonts(&mut d, &|_, _, _| Some(b"wOFF....".to_vec()), true).unwrap();
        assert!(!r[0].subset);
        assert!(r[0].path.ends_with(".woff"));
    }

    #[test]
    fn remove_unused_fonts_cleans_css_and_manifest() {
        let mut b = font_book();
        let removed = remove_unused_fonts(&mut b).unwrap();
        assert_eq!(removed, vec!["OEBPS/fonts/unused.ttf"]);
        assert!(!b.archive.contains("OEBPS/fonts/unused.ttf"));
        assert!(b.package.item_by_path("OEBPS/fonts/unused.ttf").is_none());
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(!css.contains("Unused"));
        assert!(css.contains("font-family: \"Tiny\""));
        assert!(css.contains("h1, p { font-family: Tiny, serif }"));
        assert!(remove_unused_fonts(&mut b).unwrap().is_empty());
    }

    #[test]
    fn content_strings_and_text_transform_widen_usage() {
        let mut b = make_book(&["<p>ab</p>"]);
        b.archive.set_string("OEBPS/style.css", "p { font-family: F; text-transform: uppercase }\np::before { content: \"\u{2022} \" }\n");
        let u = used_characters(&b);
        let set = u
            .get(&FaceKey {
                family: "f".into(),
                weight: 400,
                italic: false,
            })
            .unwrap();
        assert!(set.contains(&'A') && set.contains(&'a') && set.contains(&'\u{2022}'));
    }

    #[test]
    fn match_declaration_prefers_style_then_weight() {
        let d = |w: u16, i: bool| FontFaceDecl {
            family: "F".into(),
            weight: w,
            italic: i,
            sources: vec![],
            css_file: String::new(),
        };
        let all = vec![d(400, false), d(700, false), d(400, true)];
        assert_eq!(
            match_declaration(&all, "f", 700, false).unwrap().weight,
            700
        );
        assert!(match_declaration(&all, "f", 700, true).unwrap().italic);
        assert!(match_declaration(&all, "g", 400, false).is_none());
        assert_eq!(
            match_declaration(&all, "F", 600, false).unwrap().weight,
            700
        );
        assert!(is_font_path("a/B.OTF") && !is_font_path("a.png"));
        assert_eq!(uncovered_chars(b"x", &charset("a")), Vec::<char>::new());
    }
}
