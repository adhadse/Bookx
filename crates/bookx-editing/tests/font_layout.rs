//! Subsetting keeps kerning and ligatures (kern, GPOS PairPos, GSUB ligatures) with remapped glyph ids, and
//! subsets OpenType/CFF fonts. Fonts are built byte by byte in `bookx_editing::testfont`; everything is read
//! back with `ttf-parser`, an independent implementation.

mod common;

use std::collections::BTreeSet;

use bookx_editing::fonts::{subset_font_data, subset_font_report};
use bookx_editing::testfont::{TINY_OTF_GLYPHS, layout_glyphs as g, tiny_otf, tiny_ttf_layout};
use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
use ttf_parser::gsub::SubstitutionSubtable;
use ttf_parser::{Face, GlyphId, Tag};

fn chars(s: &str) -> BTreeSet<char> {
    s.chars().collect()
}

fn gid(face: &Face, c: char) -> GlyphId {
    face.glyph_index(c)
        .unwrap_or_else(|| panic!("{c:?} is not mapped"))
}

/// Horizontal kerning from the `kern` table.
fn kern_value(face: &Face, l: GlyphId, r: GlyphId) -> Option<i16> {
    face.tables()
        .kern?
        .subtables
        .into_iter()
        .filter(|s| s.horizontal && !s.variable)
        .find_map(|s| s.glyphs_kerning(l, r))
}

/// X advance adjustments for the pair from every GPOS PairPos subtable that covers it.
fn gpos_values(face: &Face, l: GlyphId, r: GlyphId) -> Vec<i16> {
    let mut out = Vec::new();
    let Some(gpos) = face.tables().gpos else {
        return out;
    };
    for lookup in gpos.lookups {
        for st in lookup.subtables.into_iter::<PositioningSubtable>() {
            if let PositioningSubtable::Pair(p) = st {
                match p {
                    PairAdjustment::Format1 { coverage, sets } => {
                        if let Some(set) = coverage.get(l).and_then(|i| sets.get(i)) {
                            if let Some((v, _)) = set.get(r) {
                                out.push(v.x_advance);
                            }
                        }
                    }
                    PairAdjustment::Format2 {
                        coverage,
                        classes,
                        matrix,
                    } => {
                        if coverage.contains(l) {
                            if let Some((v, _)) = matrix.get((classes.0.get(l), classes.1.get(r))) {
                                out.push(v.x_advance);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// The glyph a GSUB ligature `first + rest` produces.
fn ligature(face: &Face, first: GlyphId, rest: &[GlyphId]) -> Option<GlyphId> {
    let gsub = face.tables().gsub?;
    for lookup in gsub.lookups {
        for st in lookup.subtables.into_iter::<SubstitutionSubtable>() {
            if let SubstitutionSubtable::Ligature(l) = st {
                let set = l.coverage.get(first).and_then(|i| l.ligature_sets.get(i))?;
                for lig in set {
                    if lig.components.into_iter().eq(rest.iter().copied()) {
                        return Some(lig.glyph);
                    }
                }
            }
        }
    }
    None
}

fn has_feature(layout: &ttf_parser::opentype_layout::LayoutTable, tag: &[u8; 4]) -> bool {
    layout.features.find(Tag::from_bytes(tag)).is_some()
}

#[test]
fn the_layout_fixture_itself_is_valid_and_complete() {
    for ext in [false, true] {
        let data = tiny_ttf_layout(ext);
        let f = Face::parse(&data, 0).unwrap();
        assert_eq!(f.number_of_glyphs(), 13);
        let (a, v, fg, i, x, c, o) = (
            gid(&f, 'A'),
            gid(&f, 'V'),
            gid(&f, 'f'),
            gid(&f, 'i'),
            gid(&f, 'x'),
            gid(&f, 'C'),
            gid(&f, 'o'),
        );
        assert_eq!(kern_value(&f, a, v), Some(-80));
        assert_eq!(kern_value(&f, a, gid(&f, 'B')), Some(15));
        assert_eq!(kern_value(&f, fg, i), Some(-20));
        assert_eq!(kern_value(&f, v, x), Some(-33));
        assert_eq!(kern_value(&f, v, a), None);
        assert_eq!(gpos_values(&f, a, v), vec![-70]);
        assert_eq!(gpos_values(&f, a, x), vec![-11]);
        assert_eq!(gpos_values(&f, fg, i), vec![-25]);
        assert_eq!(gpos_values(&f, c, o), vec![-40]);
        assert_eq!(gpos_values(&f, v, x), vec![-61]);
        assert_eq!(ligature(&f, fg, &[i]), Some(GlyphId(g::FI)));
        assert_eq!(ligature(&f, fg, &[fg, i]), Some(GlyphId(g::FFI)));
        assert_eq!(ligature(&f, i, &[fg]), None);
        let gsub = f.tables().gsub.unwrap();
        assert!(has_feature(&gsub, b"liga") && has_feature(&gsub, b"smcp"));
    }
}

#[test]
fn kerning_and_ligatures_survive_with_remapped_ids() {
    for ext in [false, true] {
        let data = tiny_ttf_layout(ext);
        let (sub, report) = subset_font_report(&data, &chars("AVfio ")).unwrap();
        let f = Face::parse(&sub, 0).unwrap();
        let (a, v, fg, i, o) = (
            gid(&f, 'A'),
            gid(&f, 'V'),
            gid(&f, 'f'),
            gid(&f, 'i'),
            gid(&f, 'o'),
        );
        // glyph ids were remapped: the old ids are not the new ones
        assert_ne!(v.0, g::V);
        // notdef + A V f i o + fi + ffi (space is not in the font)
        assert_eq!(f.number_of_glyphs(), 8);

        // kern: A-V and f-i survive with their values; A-B (B dropped) and V-x are gone
        assert_eq!(kern_value(&f, a, v), Some(-80));
        assert_eq!(kern_value(&f, fg, i), Some(-20));
        assert_eq!(kern_value(&f, v, a), None);
        assert_eq!(report.kern_pairs, 2, "{report:?}");
        assert!(f.tables().kern.unwrap().subtables.into_iter().count() == 1);

        // GPOS: format 1 pairs A-V and f-i; format 2 keeps V-o (C is gone)
        assert_eq!(gpos_values(&f, a, v), vec![-70], "ext={ext}");
        assert_eq!(gpos_values(&f, fg, i), vec![-25]);
        assert_eq!(gpos_values(&f, v, o), vec![-60]);
        assert_eq!(gpos_values(&f, a, a), Vec::<i16>::new());
        assert_eq!(report.gpos_pairs, 2, "{report:?}");
        assert_eq!(report.gpos_class_subtables, 1);
        let gpos = f.tables().gpos.unwrap();
        assert!(has_feature(&gpos, b"kern"));
        assert!(
            !has_feature(&gpos, b"case"),
            "the dropped lookup's feature is gone"
        );
        assert_eq!(gpos.lookups.len(), 2);

        // GSUB: fi and ffi with new ids; the ligature glyphs were added to the font
        let fi = ligature(&f, fg, &[i]).expect("fi ligature kept");
        let ffi = ligature(&f, fg, &[fg, i]).expect("ffi ligature kept");
        assert_ne!(fi, ffi);
        assert!(fi.0 < f.number_of_glyphs() && ffi.0 < f.number_of_glyphs());
        assert!(
            f.outline_glyph(fi, &mut Sink).is_some(),
            "fi has an outline"
        );
        assert_eq!(report.ligatures, 2);
        let gsub = f.tables().gsub.unwrap();
        assert!(has_feature(&gsub, b"liga"));
        assert!(!has_feature(&gsub, b"smcp"));

        // what could not be kept is named
        assert_eq!(report.dropped.len(), 2, "{:?}", report.dropped);
        assert!(report.dropped[0].starts_with("GPOS lookup 2: type 1 (single adjustment)"));
        assert!(report.dropped[1].starts_with("GSUB lookup 1: type 1 (single substitution)"));
    }
}

struct Sink;
impl ttf_parser::OutlineBuilder for Sink {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

#[test]
fn only_pairs_of_kept_glyphs_are_written() {
    let data = tiny_ttf_layout(false);
    let (sub, report) = subset_font_report(&data, &chars("AB")).unwrap();
    let f = Face::parse(&sub, 0).unwrap();
    assert_eq!(kern_value(&f, gid(&f, 'A'), gid(&f, 'B')), Some(15));
    assert_eq!(report.kern_pairs, 1);
    assert_eq!(f.number_of_glyphs(), 3);
    // no kept GPOS pair (A-V is gone) and no ligature: the tables are not written at all
    assert!(f.tables().gpos.is_none());
    assert!(f.tables().gsub.is_none());
    assert_eq!(report.gpos_pairs, 0);
    assert_eq!(report.ligatures, 0);
}

#[test]
fn a_ligature_needs_all_its_components() {
    let data = tiny_ttf_layout(false);
    let (sub, report) = subset_font_report(&data, &chars("f")).unwrap();
    let f = Face::parse(&sub, 0).unwrap();
    assert_eq!(
        f.number_of_glyphs(),
        2,
        "no ligature glyph is added for f alone"
    );
    assert!(f.tables().gsub.is_none());
    assert_eq!(report.ligatures, 0);
    // with i the ligature glyphs come along without being asked for
    let (sub, _) = subset_font_report(&data, &chars("fi")).unwrap();
    assert_eq!(Face::parse(&sub, 0).unwrap().number_of_glyphs(), 5);
}

#[test]
fn subset_font_data_keeps_layout_too() {
    let data = tiny_ttf_layout(false);
    let sub = subset_font_data(&data, &chars("AV")).unwrap();
    let f = Face::parse(&sub, 0).unwrap();
    assert_eq!(kern_value(&f, gid(&f, 'A'), gid(&f, 'V')), Some(-80));
    assert_eq!(gpos_values(&f, gid(&f, 'A'), gid(&f, 'V')), vec![-70]);
}

#[test]
fn fonts_without_layout_tables_report_nothing() {
    let data = bookx_editing::testfont::tiny_ttf(0);
    let (_, report) = subset_font_report(&data, &chars("AB")).unwrap();
    assert_eq!(report, Default::default());
}

#[test]
fn mutated_layout_tables_never_panic() {
    let data = tiny_ttf_layout(true);
    let wanted = chars("AVfioxBC");
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    // the layout tables are at the end of the file (they are the largest tables after glyf)
    let tail = data.len() - 700;
    for _ in 0..600 {
        let mut d = data.clone();
        for _ in 0..1 + next() % 4 {
            let at = tail + next() % (d.len() - tail);
            d[at] = (next() % 256) as u8;
        }
        if next() % 5 == 0 {
            d.truncate(tail + next() % (d.len() - tail));
        }
        let _ = subset_font_report(&d, &wanted);
    }
}

// ---- OpenType / CFF -----------------------------------------------------------------------------------------

fn bbox(face: &Face, id: GlyphId) -> (i16, i16, i16, i16) {
    let r = face.glyph_bounding_box(id).expect("outline");
    (r.x_min, r.y_min, r.x_max, r.y_max)
}

#[test]
fn the_cff_fixture_parses_with_ttf_parser() {
    let data = tiny_otf();
    assert_eq!(&data[0..4], b"OTTO");
    let f = Face::parse(&data, 0).unwrap();
    assert!(f.tables().cff.is_some() && f.tables().glyf.is_none());
    assert_eq!(f.number_of_glyphs(), TINY_OTF_GLYPHS);
    assert_eq!(f.units_per_em(), 1000);
    for (c, id) in [('A', 1), ('B', 2), ('C', 3), ('D', 4)] {
        assert_eq!(f.glyph_index(c), Some(GlyphId(id)));
    }
    assert_eq!(bbox(&f, GlyphId(1)), (0, 0, 500, 700)); // triangle
    assert_eq!(bbox(&f, GlyphId(2)), (0, 0, 500, 700)); // box through callgsubr
    assert_eq!(bbox(&f, GlyphId(3)), (0, 0, 500, 700)); // hlineto / vlineto
    assert_eq!(bbox(&f, GlyphId(4)), (0, 0, 500, 700));
    assert!(
        f.outline_glyph(GlyphId(0), &mut Sink).is_none(),
        ".notdef is empty"
    );
    assert_eq!(f.glyph_hor_advance(GlyphId(2)), Some(500));
    assert_eq!(f.glyph_name(GlyphId(1)), Some("A"));
}

#[test]
fn cff_subset_keeps_exactly_the_used_glyphs() {
    let data = tiny_otf();
    let sub = subset_font_data(&data, &chars("AB")).unwrap();
    assert_eq!(&sub[0..4], b"OTTO");
    let f = Face::parse(&sub, 0).unwrap();
    assert!(f.tables().cff.is_some());
    assert_eq!(f.number_of_glyphs(), 3, "notdef + A + B");
    let (a, b) = (gid(&f, 'A'), gid(&f, 'B'));
    assert_eq!((a.0, b.0), (1, 2));
    assert!(f.glyph_index('C').is_none() && f.glyph_index('D').is_none());
    // the outlines are the original ones (B still resolves its global subroutine)
    assert_eq!(bbox(&f, a), (0, 0, 500, 700));
    assert_eq!(bbox(&f, b), (0, 0, 500, 700));
    assert!(sub.len() < data.len());
    // outlines can be walked to the end
    assert!(f.outline_glyph(b, &mut Sink).is_some());
}

#[test]
fn cff_subset_to_a_later_glyph_remaps_ids() {
    let data = tiny_otf();
    let sub = subset_font_data(&data, &chars("D")).unwrap();
    let f = Face::parse(&sub, 0).unwrap();
    assert_eq!(f.number_of_glyphs(), 2);
    assert_eq!(gid(&f, 'D'), GlyphId(1));
    assert_eq!(bbox(&f, GlyphId(1)), (0, 0, 500, 700));
    let (_, report) = subset_font_report(&data, &chars("D")).unwrap();
    assert_eq!(
        report,
        Default::default(),
        "CFF fixture has no layout tables"
    );
}

#[test]
fn cff_subset_without_any_known_char_keeps_notdef_only() {
    let data = tiny_otf();
    let sub = subset_font_data(&data, &chars("Z")).unwrap();
    let f = Face::parse(&sub, 0).unwrap();
    assert_eq!(f.number_of_glyphs(), 1);
}

#[test]
fn truncated_cff_fonts_fail_cleanly() {
    let data = tiny_otf();
    for cut in [0, 5, 20, 100, data.len() / 2, data.len() - 3] {
        let _ = subset_font_data(&data[..cut], &chars("A"));
    }
}

// ---- through the book-level API ---------------------------------------------------------------------------

#[test]
fn book_subsetting_reports_what_layout_was_kept() {
    let mut book = common::book_from_bodies(&["<p>AV fi</p>".to_string()]);
    book.archive
        .set("OEBPS/fonts/lay.ttf", tiny_ttf_layout(false));
    book.archive.set_string(
        "OEBPS/style.css",
        "@font-face { font-family: Lay; src: url(fonts/lay.ttf) }\np { font-family: Lay }\n",
    );
    let pkg = book.package.clone();
    pkg.add_manifest_item(
        &mut book.archive,
        "lay",
        "OEBPS/fonts/lay.ttf",
        "font/ttf",
        "",
    )
    .unwrap();
    book.reload_package().unwrap();
    let results = bookx_editing::fonts::subset_embedded_fonts(&mut book);
    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert_eq!(r.path, "OEBPS/fonts/lay.ttf");
    assert!(r.after < r.before);
    assert_eq!(r.layout.kern_pairs, 2);
    assert_eq!(r.layout.ligatures, 2);
    assert_eq!(r.layout.dropped.len(), 2);
    // the font in the book really has the kerning
    let data = book.archive.get("OEBPS/fonts/lay.ttf").unwrap();
    let f = Face::parse(data, 0).unwrap();
    assert_eq!(kern_value(&f, gid(&f, 'A'), gid(&f, 'V')), Some(-80));
    assert!(ligature(&f, gid(&f, 'f'), &[gid(&f, 'i')]).is_some());
    // the serialised report carries the layout section and old reports without it still load
    let json = serde_json::to_string(r).unwrap();
    assert!(json.contains("\"layout\""));
    let old: bookx_editing::fonts::SubsetResult =
        serde_json::from_str(r#"{"path":"a","before":2,"after":1,"chars":1}"#).unwrap();
    assert_eq!(old.layout, Default::default());
}
