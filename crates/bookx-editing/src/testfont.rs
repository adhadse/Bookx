//! A tiny TrueType font built byte by byte, for tests of the font tools (also used by the integration tests).
//!
//! Construction: tables `OS/2`, `cmap` (format 4), `glyf`, `head`, `hhea`, `hmtx`, `loca` (long), `maxp`, `name`
//! and `post` (format 3) are written by hand; checksums are computed. Glyphs: 0 `.notdef` (square), 1 `A`
//! (triangle), 2 `B` (square), 3 `C` (triangle), 4 an accent mark, 5 `Á` (U+00C1) as a composite of glyphs 1 and
//! 4, followed by `extra` filler squares mapped to U+0400 and up. Nothing is taken from a real font.

use crate::fonts::{be16, table_checksum};

// ---- a font built by hand -------------------------------------------------------------------------------
//
// Tables: head, hhea, maxp, hmtx, loca (long), glyf, cmap (format 4), name, post (format 3), OS/2 (minimal).
// Glyphs: 0 .notdef (square), 1 'A' (triangle), 2 'B' (square), 3 'C' (triangle), 4 acute accent (small
// square), 5 'Á' = composite of glyph 1 and glyph 4, then `extra` filler squares mapped to U+0400...
// The checksums are computed, nothing else is taken from a real font. The cmap here is written by the test,
// independently of `build_cmap`.

fn w16(v: &mut Vec<u8>, x: u16) {
    v.extend(x.to_be_bytes());
}
fn w32(v: &mut Vec<u8>, x: u32) {
    v.extend(x.to_be_bytes());
}

fn simple_glyph(points: &[(i16, i16)]) -> Vec<u8> {
    let (xmin, xmax) = (
        points.iter().map(|p| p.0).min().unwrap(),
        points.iter().map(|p| p.0).max().unwrap(),
    );
    let (ymin, ymax) = (
        points.iter().map(|p| p.1).min().unwrap(),
        points.iter().map(|p| p.1).max().unwrap(),
    );
    let mut g = Vec::new();
    w16(&mut g, 1);
    for v in [xmin, ymin, xmax, ymax] {
        w16(&mut g, v as u16);
    }
    w16(&mut g, points.len() as u16 - 1);
    w16(&mut g, 0);
    g.extend(std::iter::repeat_n(0x01u8, points.len()));
    let mut prev = 0i16;
    for p in points {
        w16(&mut g, (p.0 - prev) as u16);
        prev = p.0;
    }
    prev = 0;
    for p in points {
        w16(&mut g, (p.1 - prev) as u16);
        prev = p.1;
    }
    while g.len() % 4 != 0 {
        g.push(0);
    }
    g
}

fn composite_glyph(parts: &[(u16, i16, i16)]) -> Vec<u8> {
    let mut g = Vec::new();
    w16(&mut g, 0xFFFF);
    for v in [0i16, 0, 600, 900] {
        w16(&mut g, v as u16);
    }
    for (i, (gid, dx, dy)) in parts.iter().enumerate() {
        let more = if i + 1 < parts.len() { 0x20 } else { 0 };
        w16(&mut g, 0x0003 | more);
        w16(&mut g, *gid);
        w16(&mut g, *dx as u16);
        w16(&mut g, *dy as u16);
    }
    while g.len() % 4 != 0 {
        g.push(0);
    }
    g
}

pub fn tiny_ttf(extra: usize) -> Vec<u8> {
    let mut segs: Vec<(u16, u16, i32)> = vec![(0x41, 0x43, 1 - 0x41), (0xC1, 0xC1, 5 - 0xC1)];
    if extra > 0 {
        segs.push((0x400, 0x400 + extra as u16 - 1, 6 - 0x400));
    }
    build_ttf(extra, segs, Vec::new())
}

/// The TrueType builder behind [`tiny_ttf`]: `segs` are cmap format 4 segments (start, end, delta, sorted, no
/// 0xFFFF terminator), `more` are extra tables (kern, GPOS, GSUB ...).
fn build_ttf(extra: usize, segs: Vec<(u16, u16, i32)>, more: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let square = [(0, 0), (500, 0), (500, 700), (0, 700)];
    let tri = [(0, 0), (500, 0), (250, 700)];
    let small = [(0, 0), (80, 0), (80, 80), (0, 80)];
    let mut glyphs: Vec<Vec<u8>> = vec![
        simple_glyph(&square),
        simple_glyph(&tri),
        simple_glyph(&square),
        simple_glyph(&tri),
        simple_glyph(&small),
        composite_glyph(&[(1, 0, 0), (4, 100, 800)]),
    ];
    for _ in 0..extra {
        glyphs.push(simple_glyph(&square));
    }
    let n = glyphs.len() as u16;
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        w32(&mut loca, glyf.len() as u32);
        glyf.extend(g);
    }
    w32(&mut loca, glyf.len() as u32);

    let mut head = Vec::new();
    w32(&mut head, 0x0001_0000);
    w32(&mut head, 0x0001_0000);
    w32(&mut head, 0);
    w32(&mut head, 0x5F0F_3CF5);
    w16(&mut head, 0);
    w16(&mut head, 1000);
    head.extend([0u8; 16]);
    for v in [0i16, 0, 600, 900] {
        w16(&mut head, v as u16);
    }
    w16(&mut head, 0);
    w16(&mut head, 8);
    w16(&mut head, 2);
    w16(&mut head, 1); // long loca
    w16(&mut head, 0);

    let mut hhea = Vec::new();
    w32(&mut hhea, 0x0001_0000);
    for v in [800i16, -200, 0] {
        w16(&mut hhea, v as u16);
    }
    w16(&mut hhea, 600);
    for _ in 0..11 {
        w16(&mut hhea, 0);
    }
    w16(&mut hhea, n);

    let mut maxp = Vec::new();
    w32(&mut maxp, 0x0001_0000);
    w16(&mut maxp, n);
    for v in [4u16, 1, 6, 2, 2, 0, 0, 0, 0, 0, 0, 2, 1] {
        w16(&mut maxp, v);
    }

    let mut hmtx = Vec::new();
    for _ in 0..n {
        w16(&mut hmtx, 600);
        w16(&mut hmtx, 0);
    }

    let cmap = cmap_format4(segs);
    let name = name_table();

    let mut post = Vec::new();
    w32(&mut post, 0x0003_0000);
    post.extend([0u8; 28]);

    let mut os2 = vec![0u8; 86];
    os2[0..2].copy_from_slice(&be16(1));

    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    tables.extend(more);
    assemble_sfnt(0x0001_0000, tables)
}

/// Writes the table directory (sorted by tag) and the table bodies with computed checksums.
fn assemble_sfnt(version: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|t| t.0);
    let mut out = Vec::new();
    w32(&mut out, version);
    w16(&mut out, tables.len() as u16);
    let n = tables.len() as u16;
    let mut es = 0u16;
    while (1u32 << (es + 1)) <= u32::from(n) {
        es += 1;
    }
    w16(&mut out, 16 << es);
    w16(&mut out, es);
    w16(&mut out, n * 16 - (16 << es));
    let off = 12 + tables.len() * 16;
    let mut body = Vec::new();
    for (tag, t) in &tables {
        out.extend(tag);
        w32(&mut out, table_checksum(t));
        w32(&mut out, (off + body.len()) as u32);
        w32(&mut out, t.len() as u32);
        body.extend(t);
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }
    out.extend(body);
    out
}

/// A `cmap` with one format 4 subtable (3,1) for the given sorted segments; adds the 0xFFFF terminator.
fn cmap_format4(mut segs: Vec<(u16, u16, i32)>) -> Vec<u8> {
    segs.push((0xFFFF, 0xFFFF, 1));
    let sc = segs.len() as u16;
    let mut sub = Vec::new();
    w16(&mut sub, 4);
    w16(&mut sub, 16 + 8 * sc);
    w16(&mut sub, 0);
    w16(&mut sub, sc * 2);
    let mut es = 0;
    while (1 << (es + 1)) <= sc {
        es += 1;
    }
    w16(&mut sub, 2 << es);
    w16(&mut sub, es);
    w16(&mut sub, sc * 2 - (2 << es));
    for s in &segs {
        w16(&mut sub, s.1);
    }
    w16(&mut sub, 0);
    for s in &segs {
        w16(&mut sub, s.0);
    }
    for s in &segs {
        w16(&mut sub, s.2 as i16 as u16);
    }
    for _ in &segs {
        w16(&mut sub, 0);
    }
    let mut cmap = Vec::new();
    w16(&mut cmap, 0);
    w16(&mut cmap, 1);
    w16(&mut cmap, 3);
    w16(&mut cmap, 1);
    w32(&mut cmap, 12);
    cmap.extend(sub);
    cmap
}

/// A `name` table with family, subfamily and full name (Windows, English).
fn name_table() -> Vec<u8> {
    let strings = ["Tiny Test", "Regular", "Tiny Test Regular"];
    let ids = [1u16, 2, 4];
    let mut name = Vec::new();
    w16(&mut name, 0);
    w16(&mut name, 3);
    w16(&mut name, 6 + 36);
    let mut pool = Vec::new();
    for (s, id) in strings.iter().zip(ids) {
        let u: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
        w16(&mut name, 3);
        w16(&mut name, 1);
        w16(&mut name, 0x409);
        w16(&mut name, id);
        w16(&mut name, u.len() as u16);
        w16(&mut name, pool.len() as u16);
        pool.extend(u);
    }
    name.extend(pool);
    name
}

// ---- an OpenType/CFF font (OTTO) built by hand ------------------------------------------------------------

/// A Type 2 charstring number operand.
fn cs_num(out: &mut Vec<u8>, v: i32) {
    match v {
        -107..=107 => out.push((v + 139) as u8),
        108..=1131 => {
            let w = v - 108;
            out.push((w >> 8) as u8 + 247);
            out.push((w & 0xFF) as u8);
        }
        -1131..=-108 => {
            let w = -v - 108;
            out.push((w >> 8) as u8 + 251);
            out.push((w & 0xFF) as u8);
        }
        _ => {
            out.push(28);
            out.extend((v as i16).to_be_bytes());
        }
    }
}

fn charstring(ops: &[(&[i32], u8)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (args, op) in ops {
        for a in *args {
            cs_num(&mut out, *a);
        }
        out.push(*op);
    }
    out
}

/// A CFF INDEX with 2-byte offsets.
fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    w16(&mut out, items.len() as u16);
    if items.is_empty() {
        return out;
    }
    out.push(2);
    let mut off = 1u16;
    w16(&mut out, off);
    for i in items {
        off += i.len() as u16;
        w16(&mut out, off);
    }
    for i in items {
        out.extend(i);
    }
    out
}

/// DICT integer operand with a fixed 5-byte encoding (so offsets can be patched without moving anything).
fn dict_int(out: &mut Vec<u8>, v: i32) {
    out.push(29);
    out.extend(v.to_be_bytes());
}

/// Number of glyphs in [`tiny_otf`].
pub const TINY_OTF_GLYPHS: u16 = 5;

/// A tiny OpenType font with CFF outlines: `.notdef`, `A` (triangle), `B` (box drawn through a global
/// subroutine), `C` (triangle with `hlineto`/`vlineto`), `D` (box). Tables: CFF (header, Name, Top DICT,
/// String and Global Subr INDEXes, charset, CharStrings, Private DICT), OS/2, cmap, head, hhea, hmtx, maxp
/// (version 0.5), name, post (format 3).
pub fn tiny_otf() -> Vec<u8> {
    // Type 2 operators: rmoveto 21, rlineto 5, hlineto 6, vlineto 7, callgsubr 29, return 11, endchar 14
    let charstrings = vec![
        charstring(&[(&[], 14)]),
        charstring(&[(&[0, 0], 21), (&[500, 0], 5), (&[-250, 700], 5), (&[], 14)]),
        charstring(&[(&[0, 0], 21), (&[-107], 29), (&[], 14)]),
        charstring(&[(&[0, 0], 21), (&[500], 6), (&[700], 7), (&[], 14)]),
        charstring(&[(&[0, 0], 21), (&[500, 0], 5), (&[0, 700], 5), (&[], 14)]),
    ];
    // global subr 0 (bias 107): the rest of a box
    let gsubr = charstring(&[(&[500, 0], 5), (&[0, 700], 5), (&[-500, 0], 5), (&[], 11)]);

    let header = vec![1u8, 0, 4, 2];
    let name_index = cff_index(&[b"TinyCFF".to_vec()]);
    let string_index = cff_index(&[b"Tiny CFF Regular".to_vec()]);
    let gsubr_index = cff_index(&[gsubr]);
    let mut charset = vec![0u8]; // format 0: one SID per glyph after .notdef (A = 34 ...)
    for sid in 34u16..34 + (TINY_OTF_GLYPHS - 1) {
        w16(&mut charset, sid);
    }
    let cs_index = cff_index(&charstrings);
    let mut private = Vec::new();
    dict_int(&mut private, 500);
    private.push(20); // defaultWidthX
    dict_int(&mut private, 500);
    private.push(21); // nominalWidthX

    let top = |charset_off: i32, cs_off: i32, priv_off: i32| {
        let mut d = Vec::new();
        dict_int(&mut d, 391);
        d.push(2); // FullName (first custom string)
        for v in [0, -200, 600, 900] {
            dict_int(&mut d, v);
        }
        d.push(5); // FontBBox
        dict_int(&mut d, charset_off);
        d.push(15);
        dict_int(&mut d, cs_off);
        d.push(17);
        dict_int(&mut d, private.len() as i32);
        dict_int(&mut d, priv_off);
        d.push(18);
        d
    };
    let top_len = cff_index(&[top(0, 0, 0)]).len();
    let charset_off =
        header.len() + name_index.len() + top_len + string_index.len() + gsubr_index.len();
    let cs_off = charset_off + charset.len();
    let priv_off = cs_off + cs_index.len();
    let top_index = cff_index(&[top(charset_off as i32, cs_off as i32, priv_off as i32)]);
    let mut cff = header;
    for part in [
        &name_index,
        &top_index,
        &string_index,
        &gsubr_index,
        &charset,
        &cs_index,
        &private,
    ] {
        cff.extend(part);
    }

    let n = TINY_OTF_GLYPHS;
    let mut head = Vec::new();
    w32(&mut head, 0x0001_0000);
    w32(&mut head, 0x0001_0000);
    w32(&mut head, 0);
    w32(&mut head, 0x5F0F_3CF5);
    w16(&mut head, 0);
    w16(&mut head, 1000);
    head.extend([0u8; 16]);
    for v in [0i16, -200, 600, 900] {
        w16(&mut head, v as u16);
    }
    w16(&mut head, 0);
    w16(&mut head, 8);
    w16(&mut head, 2);
    w16(&mut head, 0); // short loca (unused)
    w16(&mut head, 0);
    let mut hhea = Vec::new();
    w32(&mut hhea, 0x0001_0000);
    for v in [800i16, -200, 0] {
        w16(&mut hhea, v as u16);
    }
    w16(&mut hhea, 600);
    for _ in 0..11 {
        w16(&mut hhea, 0);
    }
    w16(&mut hhea, n);
    let mut maxp = Vec::new();
    w32(&mut maxp, 0x0000_5000);
    w16(&mut maxp, n);
    let mut hmtx = Vec::new();
    for _ in 0..n {
        w16(&mut hmtx, 500);
        w16(&mut hmtx, 0);
    }
    let cmap = cmap_format4(vec![(0x41, 0x44, 1 - 0x41)]);
    let mut post = Vec::new();
    w32(&mut post, 0x0003_0000);
    post.extend([0u8; 28]);
    let mut os2 = vec![0u8; 78];
    os2[0..2].copy_from_slice(&be16(1));
    assemble_sfnt(
        u32::from_be_bytes(*b"OTTO"),
        vec![
            (*b"CFF ", cff),
            (*b"OS/2", os2),
            (*b"cmap", cmap),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"maxp", maxp),
            (*b"name", name_table()),
            (*b"post", post),
        ],
    )
}

// ---- a TrueType font with kern, GPOS and GSUB ------------------------------------------------------------

/// Glyph ids of [`tiny_ttf_layout`].
pub mod layout_glyphs {
    pub const A: u16 = 1;
    pub const B: u16 = 2;
    pub const C: u16 = 3;
    pub const F: u16 = 6;
    pub const I: u16 = 7;
    pub const V: u16 = 8;
    pub const O: u16 = 9;
    /// ligature `fi`, not mapped by the cmap
    pub const FI: u16 = 10;
    /// ligature `ffi`, not mapped by the cmap
    pub const FFI: u16 = 11;
    pub const X: u16 = 12;
}

fn cov1(glyphs: &[u16]) -> Vec<u8> {
    let mut c = Vec::new();
    w16(&mut c, 1);
    w16(&mut c, glyphs.len() as u16);
    for g in glyphs {
        w16(&mut c, *g);
    }
    c
}

/// A lookup with `kind` and subtables; `ext` wraps every subtable into an extension subtable of type `ext_kind`.
fn lookup_bytes(kind: u16, ext_kind: u16, subtables: Vec<Vec<u8>>, ext: bool) -> Vec<u8> {
    let n = subtables.len();
    let mut out = Vec::new();
    w16(&mut out, if ext { ext_kind } else { kind });
    w16(&mut out, 0);
    w16(&mut out, n as u16);
    let head = 6 + 2 * n;
    let mut offs = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    if ext {
        let ext_len = 8 * n;
        let mut real: Vec<u8> = Vec::new();
        for (i, st) in subtables.iter().enumerate() {
            offs.push((head + 8 * i) as u16);
            w16(&mut body, 1);
            w16(&mut body, kind);
            w32(&mut body, (ext_len - 8 * i + real.len()) as u32);
            real.extend(st);
        }
        body.extend(real);
    } else {
        for st in &subtables {
            offs.push((head + body.len()) as u16);
            body.extend(st);
        }
    }
    for o in offs {
        w16(&mut out, o);
    }
    out.extend(body);
    out
}

/// GSUB/GPOS table with one default script and the given `(tag, lookup indices)` features.
fn layout_table(features: &[(&[u8; 4], &[u16])], lookups: Vec<Vec<u8>>) -> Vec<u8> {
    let mut script = Vec::new();
    w16(&mut script, 1);
    script.extend(b"DFLT");
    w16(&mut script, 8);
    w16(&mut script, 4); // default LangSys
    w16(&mut script, 0);
    w16(&mut script, 0); // lookupOrder
    w16(&mut script, 0xFFFF);
    w16(&mut script, features.len() as u16);
    for i in 0..features.len() as u16 {
        w16(&mut script, i);
    }
    let mut flist = Vec::new();
    w16(&mut flist, features.len() as u16);
    let mut ftabs = Vec::new();
    let base = 2 + 6 * features.len();
    for (tag, idx) in features {
        flist.extend(*tag);
        w16(&mut flist, (base + ftabs.len()) as u16);
        w16(&mut ftabs, 0);
        w16(&mut ftabs, idx.len() as u16);
        for i in *idx {
            w16(&mut ftabs, *i);
        }
    }
    flist.extend(ftabs);
    let mut llist = Vec::new();
    w16(&mut llist, lookups.len() as u16);
    let mut body: Vec<u8> = Vec::new();
    let base = 2 + 2 * lookups.len();
    for l in &lookups {
        w16(&mut llist, (base + body.len()) as u16);
        body.extend(l);
    }
    llist.extend(body);
    let mut t = Vec::new();
    w32(&mut t, 0x0001_0000);
    w16(&mut t, 10);
    w16(&mut t, (10 + script.len()) as u16);
    w16(&mut t, (10 + script.len() + flist.len()) as u16);
    t.extend(script);
    t.extend(flist);
    t.extend(llist);
    t
}

/// A TrueType font (glyphs: see [`layout_glyphs`]; cmap: A B C V f i o x, U+00C1) with
/// - `kern` (format 0): A-V -80, A-B 15, f-i -20, C-x -5, V-x -33,
/// - `GPOS`: lookup 0 PairPos format 1 (A-V -70, A-x -11, f-i -25, f-x -9), lookup 1 PairPos format 2
///   (classes: C 1, V 2 against o 1, x 2; C-o -40, C-x -41, V-o -60, V-x -61), lookup 2 a SinglePos (not
///   preservable); features `kern` (0, 1) and `case` (2),
/// - `GSUB`: lookup 0 ligatures f+i -> fi and f+f+i -> ffi, lookup 1 a SingleSubst (not preservable); features
///   `liga` (0) and `smcp` (1).
///
/// With `ext` the preservable lookups are wrapped into extension lookups.
pub fn tiny_ttf_layout(ext: bool) -> Vec<u8> {
    use layout_glyphs::*;
    let segs = vec![
        (0x41, 0x43, i32::from(A) - 0x41),
        (0x56, 0x56, i32::from(V) - 0x56),
        (0x66, 0x66, i32::from(F) - 0x66),
        (0x69, 0x69, i32::from(I) - 0x69),
        (0x6F, 0x6F, i32::from(O) - 0x6F),
        (0x78, 0x78, i32::from(X) - 0x78),
        (0xC1, 0xC1, 5 - 0xC1),
    ];
    // kern
    let mut pairs: Vec<(u16, u16, i16)> = vec![
        (A, B, 15),
        (A, V, -80),
        (C, X, -5),
        (F, I, -20),
        (V, X, -33),
    ];
    pairs.sort();
    let mut sub = Vec::new();
    w16(&mut sub, 0);
    w16(&mut sub, 14 + 6 * pairs.len() as u16);
    w16(&mut sub, 1);
    w16(&mut sub, pairs.len() as u16);
    w16(&mut sub, 6 * 4); // searchRange (not used by the readers here)
    w16(&mut sub, 2);
    w16(&mut sub, 6 * pairs.len() as u16 - 24);
    for (l, r, v) in &pairs {
        w16(&mut sub, *l);
        w16(&mut sub, *r);
        w16(&mut sub, *v as u16);
    }
    let mut kern = Vec::new();
    w16(&mut kern, 0);
    w16(&mut kern, 1);
    kern.extend(sub);

    // GPOS
    let mut p1 = Vec::new();
    w16(&mut p1, 1);
    let sets: Vec<Vec<u8>> = [vec![(V, -70i16), (X, -11)], vec![(I, -25), (X, -9)]]
        .iter()
        .map(|recs| {
            let mut s = Vec::new();
            w16(&mut s, recs.len() as u16);
            for (g, v) in recs {
                w16(&mut s, *g);
                w16(&mut s, *v as u16);
            }
            s
        })
        .collect();
    let head_len = 10 + 4;
    let sets_len: usize = sets.iter().map(|s| s.len()).sum();
    w16(&mut p1, (head_len + sets_len) as u16);
    w16(&mut p1, 4);
    w16(&mut p1, 0);
    w16(&mut p1, 2);
    let mut off = head_len;
    for s in &sets {
        w16(&mut p1, off as u16);
        off += s.len();
    }
    for s in &sets {
        p1.extend(s);
    }
    p1.extend(cov1(&[A, F]));

    let mut p2 = Vec::new();
    let classdef = |glyphs: &[(u16, u16)]| {
        let mut c = Vec::new();
        w16(&mut c, 2);
        w16(&mut c, glyphs.len() as u16);
        for (g, k) in glyphs {
            w16(&mut c, *g);
            w16(&mut c, *g);
            w16(&mut c, *k);
        }
        c
    };
    let cd1 = classdef(&[(C, 1), (V, 2)]);
    let cd2 = classdef(&[(O, 1), (X, 2)]);
    let matrix: [[i16; 3]; 3] = [[0, 0, 0], [0, -40, -41], [0, -60, -61]];
    let fixed = 16 + 18;
    w16(&mut p2, 2);
    w16(&mut p2, fixed as u16);
    w16(&mut p2, 4);
    w16(&mut p2, 0);
    w16(&mut p2, (fixed + 8) as u16);
    w16(&mut p2, (fixed + 8 + cd1.len()) as u16);
    w16(&mut p2, 3);
    w16(&mut p2, 3);
    for row in matrix {
        for v in row {
            w16(&mut p2, v as u16);
        }
    }
    p2.extend(cov1(&[C, V]));
    p2.extend(&cd1);
    p2.extend(&cd2);

    let mut p3 = Vec::new(); // SinglePos format 1
    w16(&mut p3, 1);
    w16(&mut p3, 8);
    w16(&mut p3, 4);
    w16(&mut p3, 7);
    p3.extend(cov1(&[A]));

    let gpos = layout_table(
        &[(b"kern", &[0, 1]), (b"case", &[2])],
        vec![
            lookup_bytes(2, 9, vec![p1], ext),
            lookup_bytes(2, 9, vec![p2], ext),
            lookup_bytes(1, 9, vec![p3], false),
        ],
    );

    // GSUB
    let mut lig = Vec::new();
    w16(&mut lig, 1);
    let ligature = |glyph: u16, rest: &[u16]| {
        let mut l = Vec::new();
        w16(&mut l, glyph);
        w16(&mut l, rest.len() as u16 + 1);
        for g in rest {
            w16(&mut l, *g);
        }
        l
    };
    let ligs = [ligature(FFI, &[F, I]), ligature(FI, &[I])];
    let mut set = Vec::new();
    w16(&mut set, 2);
    w16(&mut set, 6);
    w16(&mut set, (6 + ligs[0].len()) as u16);
    set.extend(&ligs[0]);
    set.extend(&ligs[1]);
    w16(&mut lig, 8 + set.len() as u16);
    w16(&mut lig, 1);
    w16(&mut lig, 8);
    lig.extend(set);
    lig.extend(cov1(&[F]));
    let mut single = Vec::new();
    w16(&mut single, 1);
    w16(&mut single, 6);
    w16(&mut single, 1);
    single.extend(cov1(&[A]));
    let gsub = layout_table(
        &[(b"liga", &[0]), (b"smcp", &[1])],
        vec![
            lookup_bytes(4, 7, vec![lig], ext),
            lookup_bytes(1, 7, vec![single], false),
        ],
    );
    build_ttf(
        7,
        segs,
        vec![(*b"kern", kern), (*b"GPOS", gpos), (*b"GSUB", gsub)],
    )
}
