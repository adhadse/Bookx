//! The Manage Fonts dialog (CE-420): the embedded fonts with family, style, size and use, renaming a family in
//! every `@font-face` and `font-family` rule, removing fonts with their rules, and a vector sample of a font.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{Result, bail};
use bookx_convert::transforms::css::{CssItem, parse_css};
use bookx_core::book::OpenBook;
use bookx_core::util::{parent_dir, resolve_href};

use crate::fonts::{Embedding, analyze_fonts, embedding_permission};
use crate::markup::css_files;
use crate::transform_css::{CssTransform, transform_book};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedFont {
    pub path: String,
    pub bytes: usize,
    /// Family and style stored in the font file.
    pub file_family: String,
    pub file_style: String,
    /// Family names of the `@font-face` rules that use the file, as written.
    pub declared: Vec<String>,
    pub used_chars: usize,
    pub unused: bool,
    pub missing_glyphs: usize,
    pub embedding: Embedding,
}

impl ManagedFont {
    /// The name the book calls this font by (first declaration), else the one in the file.
    pub fn display_family(&self) -> &str {
        self.declared
            .first()
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.file_family)
    }
}

pub fn list_fonts(book: &OpenBook) -> Vec<ManagedFont> {
    let rep = analyze_fonts(book);
    rep.embedded
        .into_iter()
        .map(|f| {
            let mut declared: Vec<String> = Vec::new();
            for (fam, _, _) in &f.declared_as {
                if !declared.contains(fam) {
                    declared.push(fam.clone());
                }
            }
            let embedding = book
                .archive
                .get(&f.path)
                .map(embedding_permission)
                .unwrap_or(Embedding::Unknown);
            ManagedFont {
                bytes: f.size,
                file_family: f
                    .info
                    .as_ref()
                    .and_then(|i| i.family.clone())
                    .unwrap_or_default(),
                file_style: f
                    .info
                    .as_ref()
                    .and_then(|i| i.subfamily.clone())
                    .unwrap_or_default(),
                declared,
                used_chars: f.used_chars,
                unused: f.unused,
                missing_glyphs: f.missing_glyphs.len(),
                embedding,
                path: f.path,
            }
        })
        .collect()
}

/// A family name the rename accepts: no quotes, braces, semicolons, commas or angle brackets, not empty, not
/// a generic family.
pub fn valid_family_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        bail!("the font name is empty");
    }
    if n.chars().count() > 100 {
        bail!("the font name is too long");
    }
    if n.contains([
        '"', '\'', '{', '}', ';', ',', '<', '>', '\\', '/', '\n', '\r',
    ]) {
        bail!("the font name must not contain quotes, commas, semicolons, slashes or brackets");
    }
    if crate::fonts::GENERIC_FAMILIES.contains(&n.to_ascii_lowercase().as_str()) {
        bail!("\"{n}\" is a generic family; pick another name");
    }
    Ok(n.to_string())
}

/// Renames a family everywhere: `@font-face`, `font-family` and `font` in stylesheets, `<style>` blocks and
/// style attributes. Returns the number of changed declarations.
pub fn rename_family(book: &mut OpenBook, from: &str, to: &str) -> Result<usize> {
    let to = valid_family_name(to)?;
    if from.trim().is_empty() {
        bail!("there is no font to rename");
    }
    if from.trim().eq_ignore_ascii_case(&to) && from.trim() == to {
        return Ok(0);
    }
    let rep = transform_book(
        book,
        &[CssTransform::RenameFontFamily {
            from: from.trim().to_string(),
            to,
        }],
    );
    Ok(rep.total_changes)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveResult {
    pub files: Vec<String>,
    /// `@font-face` rules removed.
    pub rules: usize,
}

/// Removes font files together with their manifest entries and the `@font-face` rules that point at them.
/// The text that used them falls back to the next family of its `font-family` list.
pub fn remove_fonts(book: &mut OpenBook, paths: &[String]) -> Result<RemoveResult> {
    let mut res = RemoveResult::default();
    for p in paths {
        if !crate::fonts::is_font_path(p) || !book.archive.contains(p) {
            bail!("{p} is not a font of this book");
        }
    }
    for p in paths {
        if let Some(item) = book.package.item_by_path(p).cloned() {
            let pkg = book.package.clone();
            pkg.remove_manifest_item(&mut book.archive, &item.id)?;
            book.reload_package()?;
        }
        book.archive.remove(p);
        res.files.push(p.clone());
    }
    let url = regex::Regex::new(r#"url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)"#)?;
    for css_path in css_files(&book.archive) {
        let css = book.archive.get_string(&css_path).unwrap_or_default();
        let dir = parent_dir(&css_path);
        let mut out = String::new();
        let mut removed_here = 0;
        for item in parse_css(&css) {
            match item {
                CssItem::At(s) if s.to_ascii_lowercase().starts_with("@font-face") => {
                    let gone = url.captures_iter(s).any(|c| {
                        c.get(1)
                            .or(c.get(2))
                            .or(c.get(3))
                            .is_some_and(|m| res.files.contains(&resolve_href(dir, m.as_str()).0))
                    });
                    if gone {
                        removed_here += 1;
                    } else {
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
        if removed_here > 0 {
            res.rules += removed_here;
            book.archive.set_string(&css_path, &out);
        }
    }
    Ok(res)
}

/// A line of text drawn with the outlines of a font as an SVG image (`width` x `height` pixels), so the
/// sample shows the real glyphs of an embedded font without installing it. `None` when the font cannot be read.
/// Characters the font lacks are skipped.
pub fn sample_svg(data: &[u8], text: &str, px: f32, width: u32) -> Option<String> {
    struct Path(String);
    impl ttf_parser::OutlineBuilder for Path {
        fn move_to(&mut self, x: f32, y: f32) {
            let _ = write!(self.0, "M{x:.1} {y:.1}");
        }
        fn line_to(&mut self, x: f32, y: f32) {
            let _ = write!(self.0, "L{x:.1} {y:.1}");
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            let _ = write!(self.0, "Q{x1:.1} {y1:.1} {x:.1} {y:.1}");
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            let _ = write!(self.0, "C{x1:.1} {y1:.1} {x2:.1} {y2:.1} {x:.1} {y:.1}");
        }
        fn close(&mut self) {
            self.0.push('Z');
        }
    }
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let upem = f32::from(face.units_per_em());
    if upem <= 0.0 || !(4.0..=400.0).contains(&px) {
        return None;
    }
    let scale = px / upem;
    let line_h = px * 1.4;
    let ascent = f32::from(face.ascender()) * scale;
    let (mut x, mut y) = (0.0f32, ascent + px * 0.1);
    let mut d = String::new();
    for ch in text.chars().take(400) {
        if ch == '\n' {
            x = 0.0;
            y += line_h;
            continue;
        }
        let Some(g) = face.glyph_index(ch) else {
            continue;
        };
        let adv = f32::from(face.glyph_hor_advance(g).unwrap_or(0)) * scale;
        if x + adv > width as f32 && x > 0.0 {
            x = 0.0;
            y += line_h;
        }
        let mut p = Path(String::new());
        if face.outline_glyph(g, &mut p).is_some() && !p.0.is_empty() {
            let _ = write!(
                d,
                "<path transform=\"translate({x:.1} {y:.1}) scale({scale:.5} -{scale:.5})\" d=\"{}\"/>",
                p.0
            );
        }
        x += adv;
    }
    if d.is_empty() {
        return None;
    }
    let height = (y + px * 0.4).ceil() as u32;
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><g fill=\"#000\">{d}</g></svg>"
    ))
}

/// The distinct family names declared in the book (for the rename suggestions).
pub fn declared_families(book: &OpenBook) -> BTreeSet<String> {
    list_fonts(book)
        .into_iter()
        .flat_map(|f| f.declared)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    /// A book with the tiny test font embedded as "Tiny Test", used by a paragraph style and a stylesheet.
    fn book_with_font() -> OpenBook {
        let mut b = make_book(&[
            "<p style=\"font-family: 'Tiny Test', serif\">ABC</p>",
            "<p>plain</p>",
        ]);
        let ttf = crate::testfont::tiny_ttf(0);
        crate::files::add_file(&mut b, "tiny.ttf", ttf, None).unwrap();
        let font_path = b
            .archive
            .names()
            .iter()
            .find(|n| n.ends_with("tiny.ttf"))
            .unwrap()
            .clone();
        let css_path = "OEBPS/style.css".to_string();
        let css = b.archive.get_string(&css_path).unwrap_or_default();
        let rel = bookx_core::util::relative_href(&css_path, &font_path);
        b.archive.set_string(
            &css_path,
            &format!(
                "{css}\n@font-face {{ font-family: 'Tiny Test'; src: url('{rel}'); }}\nbody {{ font-family: \"Tiny Test\", serif }}\n"
            ),
        );
        b
    }

    #[test]
    fn lists_the_embedded_font_with_family_and_use() {
        let b = book_with_font();
        let l = list_fonts(&b);
        assert_eq!(l.len(), 1, "{l:?}");
        assert_eq!(l[0].declared, vec!["Tiny Test".to_string()]);
        assert_eq!(l[0].display_family(), "Tiny Test");
        assert!(l[0].bytes > 100 && l[0].used_chars > 0, "{:?}", l[0]);
        assert!(!l[0].unused);
        assert_eq!(declared_families(&b).len(), 1);
    }

    #[test]
    fn rename_changes_face_and_every_use() {
        let mut b = book_with_font();
        let n = rename_family(&mut b, "Tiny Test", "Pocket Serif").unwrap();
        assert!(n >= 3, "face, stylesheet rule and inline style: {n}");
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(
            !css.contains("Tiny Test") && css.contains("Pocket Serif"),
            "{css}"
        );
        let page = b.archive.get_string(&b.spine_paths()[0]).unwrap();
        assert!(
            page.contains("Pocket Serif") && !page.contains("Tiny Test"),
            "{page}"
        );
        let l = list_fonts(&b);
        assert_eq!(l[0].declared, vec!["Pocket Serif".to_string()]);
        assert!(!l[0].unused, "the font is still found by its new name");
    }

    #[test]
    fn rename_is_validated() {
        let mut b = book_with_font();
        for bad in ["", "  ", "a;b", "a\"b", "x{y}", "serif", "a,b", "<b>"] {
            assert!(rename_family(&mut b, "Tiny Test", bad).is_err(), "{bad:?}");
        }
        assert!(rename_family(&mut b, "", "Ok").is_err());
        assert_eq!(rename_family(&mut b, "Not There", "Other").unwrap(), 0);
        assert!(valid_family_name("  Open Sans ").unwrap() == "Open Sans");
    }

    #[test]
    fn remove_takes_file_manifest_and_rule_but_keeps_other_rules() {
        let mut b = book_with_font();
        let css_before = b.archive.get_string("OEBPS/style.css").unwrap();
        let font = list_fonts(&b)[0].path.clone();
        let r = remove_fonts(&mut b, std::slice::from_ref(&font)).unwrap();
        assert_eq!(r.files, vec![font.clone()]);
        assert_eq!(r.rules, 1);
        assert!(!b.archive.contains(&font));
        assert!(b.package.item_by_path(&font).is_none());
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(
            !css.contains("@font-face") && css.contains("font-family"),
            "{css}"
        );
        assert!(css.len() < css_before.len());
        assert!(list_fonts(&b).is_empty());
        assert!(remove_fonts(&mut b, &["OEBPS/x.ttf".into()]).is_err());
        assert!(remove_fonts(&mut b, &["OEBPS/style.css".into()]).is_err());
        assert!(OpenBook::from_archive(b.archive.clone()).is_ok());
    }

    #[test]
    fn sample_draws_glyph_outlines() {
        let ttf = crate::testfont::tiny_ttf(0);
        let svg = sample_svg(&ttf, "ABC", 32.0, 200);
        if let Some(s) = svg {
            assert!(s.starts_with("<svg") && s.contains("<path"), "{s}");
            assert!(bookx_core::util::parse_xml(&s).is_ok());
        }
        assert!(sample_svg(b"junk", "A", 32.0, 200).is_none());
        assert!(sample_svg(&ttf, "A", 2.0, 200).is_none());
    }
}
