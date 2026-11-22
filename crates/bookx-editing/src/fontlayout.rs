//! Keeps kerning and ligatures when a font is subset: `kern` format 0, `GPOS` pair adjustment (formats 1 and 2)
//! and `GSUB` ligature substitution (type 4), restricted to the kept glyphs and written with the new glyph ids.
//!
//! The subsetting library drops `GSUB`, `GPOS` and `kern`. This module reads them from the original font with
//! its own bounds-checked reader and writes new, smaller tables:
//!
//! * `kern`: format 0, horizontal, non-cross-stream subtables; pairs whose glyphs are both kept (sorted by the
//!   new ids). Other subtable formats (and the Apple `kern` header) are reported and dropped.
//! * `GPOS`: lookups made only of `PairPos` subtables (also inside extension lookups) without device tables.
//!   Format 1 keeps the records of kept glyph pairs; format 2 keeps the class matrix and restricts coverage and
//!   both class definitions. Every other lookup type is dropped and reported.
//! * `GSUB`: lookups made only of ligature substitutions; a ligature survives when its first glyph, all its
//!   components and the ligature glyph are kept. [`ligature_closure`] adds the ligature glyphs to the kept set
//!   beforehand. Every other lookup type is dropped and reported.
//!
//! Scripts and language systems are not preserved: the new tables have one `DFLT` script whose default language
//! system enables all kept features (features are kept by tag, with their lookup lists reduced to the kept
//! lookups). Lookups that use a mark filtering set (which refers to the dropped `GDEF`) are dropped and reported.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// What was kept and what could not be preserved when the layout tables were rebuilt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutReport {
    /// Kerning pairs written to the new `kern` table.
    pub kern_pairs: usize,
    /// Pair records kept in `GPOS` PairPos format 1 subtables.
    pub gpos_pairs: usize,
    /// `GPOS` PairPos format 2 (class based) subtables kept.
    pub gpos_class_subtables: usize,
    /// Ligature substitutions kept in `GSUB`.
    pub ligatures: usize,
    /// Lookups (or tables) that could not be preserved, with the reason, e.g. `GPOS lookup 2: type 1 (single
    /// adjustment) is not preserved`.
    pub dropped: Vec<String>,
}

/// Bounds-checked big-endian reader over a byte slice.
#[derive(Clone, Copy)]
struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b = self.0.get(o..o.checked_add(2)?)?;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&self, o: usize) -> Option<i16> {
        self.u16(o).map(|v| v as i16)
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let b = self.0.get(o..o.checked_add(4)?)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn sub(&self, o: usize) -> Option<Rd<'a>> {
        self.0.get(o..).map(Rd)
    }
    fn bytes(&self, o: usize, n: usize) -> Option<&'a [u8]> {
        self.0.get(o..o.checked_add(n)?)
    }
}

/// The body of table `tag` of an sfnt file.
pub fn sfnt_table<'a>(data: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let rd = Rd(data);
    let num = rd.u16(4)? as usize;
    for i in 0..num {
        let rec = rd.bytes(12 + i * 16, 16)?;
        if &rec[0..4] == tag {
            let off = u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]) as usize;
            let len = u32::from_be_bytes([rec[12], rec[13], rec[14], rec[15]]) as usize;
            return rd.bytes(off, len);
        }
    }
    None
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

/// Glyph ids of a Coverage table in coverage-index order.
fn coverage(rd: Rd, off: usize) -> Option<Vec<u16>> {
    let c = rd.sub(off)?;
    match c.u16(0)? {
        1 => {
            let n = c.u16(2)? as usize;
            (0..n).map(|i| c.u16(4 + 2 * i)).collect()
        }
        2 => {
            let n = c.u16(2)? as usize;
            let mut out = Vec::new();
            for i in 0..n {
                let (s, e, idx) = (c.u16(4 + 6 * i)?, c.u16(6 + 6 * i)?, c.u16(8 + 6 * i)?);
                if e < s || idx as usize != out.len() || out.len() + (e - s) as usize >= 65536 {
                    return None;
                }
                out.extend(s..=e);
            }
            Some(out)
        }
        _ => None,
    }
}

/// A ClassDef table as glyph -> class (class 0 entries omitted).
fn classdef(rd: Rd, off: usize) -> Option<BTreeMap<u16, u16>> {
    let c = rd.sub(off)?;
    let mut out = BTreeMap::new();
    match c.u16(0)? {
        1 => {
            let start = c.u16(2)?;
            let n = c.u16(4)? as usize;
            for i in 0..n {
                let class = c.u16(6 + 2 * i)?;
                let g = u32::from(start) + i as u32;
                if class != 0 && g <= 0xFFFF {
                    out.insert(g as u16, class);
                }
            }
        }
        2 => {
            let n = c.u16(2)? as usize;
            let mut total = 0usize;
            for i in 0..n {
                let (s, e, class) = (c.u16(4 + 6 * i)?, c.u16(6 + 6 * i)?, c.u16(8 + 6 * i)?);
                if e < s {
                    return None;
                }
                total += (e - s) as usize + 1;
                if total > 65536 * 2 {
                    return None;
                }
                if class != 0 {
                    for g in s..=e {
                        out.insert(g, class);
                    }
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

fn write_coverage(glyphs: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(glyphs.len() as u16));
    for g in glyphs {
        out.extend(be16(*g));
    }
    out
}

fn write_classdef(map: &BTreeMap<u16, u16>) -> Vec<u8> {
    let mut ranges: Vec<(u16, u16, u16)> = Vec::new();
    for (&g, &c) in map {
        match ranges.last_mut() {
            Some((_, e, k)) if *k == c && e.checked_add(1) == Some(g) => *e = g,
            _ => ranges.push((g, g, c)),
        }
    }
    let mut out = Vec::new();
    out.extend(be16(2));
    out.extend(be16(ranges.len() as u16));
    for (s, e, c) in ranges {
        out.extend(be16(s));
        out.extend(be16(e));
        out.extend(be16(c));
    }
    out
}

/// One lookup resolved through extension subtables.
struct Lookup<'a> {
    kind: u16,
    flag: u16,
    subtables: Vec<Rd<'a>>,
}

/// Reads lookup `index` of the lookup list at `list_off`. `ext_kind` is 7 for GSUB, 9 for GPOS.
fn read_lookup(rd: Rd<'_>, list_off: usize, index: usize, ext_kind: u16) -> Option<Lookup<'_>> {
    let list = rd.sub(list_off)?;
    let l = list.sub(list.u16(2 + 2 * index)? as usize)?;
    let mut kind = l.u16(0)?;
    let flag = l.u16(2)?;
    let n = l.u16(4)? as usize;
    let mut subtables = Vec::new();
    let mut real_kind = None;
    for i in 0..n {
        let st = l.sub(l.u16(6 + 2 * i)? as usize)?;
        if kind == ext_kind {
            if st.u16(0)? != 1 {
                return None;
            }
            let real = st.u16(2)?;
            if real == ext_kind || *real_kind.get_or_insert(real) != real {
                return None;
            }
            subtables.push(st.sub(st.u32(4)? as usize)?);
        } else {
            subtables.push(st);
        }
    }
    if kind == ext_kind {
        kind = real_kind?;
    }
    Some(Lookup {
        kind,
        flag,
        subtables,
    })
}

fn lookup_count(rd: Rd, list_off: usize) -> usize {
    rd.sub(list_off)
        .and_then(|l| l.u16(0))
        .map(usize::from)
        .unwrap_or(0)
}

/// `(script list offset, feature list offset, lookup list offset)` of a GSUB/GPOS table.
fn layout_header(rd: Rd) -> Option<(usize, usize, usize)> {
    let v = rd.u32(0)?;
    if v != 0x0001_0000 && v != 0x0001_0001 {
        return None;
    }
    Some((
        rd.u16(4)? as usize,
        rd.u16(6)? as usize,
        rd.u16(8)? as usize,
    ))
}

type Ligature = (u16, u16, Vec<u16>);

/// First glyph and its `(second glyph, value record bytes)` pairs.
type PairSet = (u16, Vec<(u16, Vec<u8>)>);

/// Tables to add to a font: tag and body.
pub type TableList = Vec<([u8; 4], Vec<u8>)>;

/// The ligature substitutions of a type 4 subtable: `(first, ligature glyph, components after the first)`.
fn read_ligatures(st: Rd) -> Option<Vec<Ligature>> {
    if st.u16(0)? != 1 {
        return None;
    }
    let cov = coverage(st, st.u16(2)? as usize)?;
    if st.u16(4)? as usize != cov.len() {
        return None;
    }
    let mut out = Vec::new();
    for (i, first) in cov.iter().enumerate() {
        let set = st.sub(st.u16(6 + 2 * i)? as usize)?;
        let count = set.u16(0)? as usize;
        for j in 0..count {
            let lig = set.sub(set.u16(2 + 2 * j)? as usize)?;
            let glyph = lig.u16(0)?;
            let comps = lig.u16(2)? as usize;
            if comps == 0 {
                return None;
            }
            let rest: Option<Vec<u16>> = (0..comps - 1).map(|k| lig.u16(4 + 2 * k)).collect();
            out.push((*first, glyph, rest?));
        }
    }
    Some(out)
}

/// Adds to `present` the glyphs of every `GSUB` ligature whose first glyph and components are all present
/// (repeated until nothing changes, so ligatures of ligatures work). Returns how many glyphs were added.
pub fn ligature_closure(data: &[u8], present: &mut BTreeSet<u16>) -> usize {
    let Some(gsub) = sfnt_table(data, b"GSUB") else {
        return 0;
    };
    let rd = Rd(gsub);
    let Some((_, _, list_off)) = layout_header(rd) else {
        return 0;
    };
    let mut ligs: Vec<Ligature> = Vec::new();
    for i in 0..lookup_count(rd, list_off) {
        if let Some(l) = read_lookup(rd, list_off, i, 7) {
            if l.kind == 4 {
                for st in l.subtables {
                    ligs.extend(read_ligatures(st).unwrap_or_default());
                }
            }
        }
    }
    let before = present.len();
    loop {
        let n = present.len();
        for (first, glyph, rest) in &ligs {
            if present.contains(first) && rest.iter().all(|g| present.contains(g)) {
                present.insert(*glyph);
            }
        }
        if present.len() == n {
            break;
        }
    }
    present.len() - before
}

// ---------------------------------------------------------------------------
// kern
// ---------------------------------------------------------------------------

/// Rebuilds `kern` (format 0 subtables only).
fn rebuild_kern(
    kern: &[u8],
    map: &dyn Fn(u16) -> Option<u16>,
    report: &mut LayoutReport,
) -> Option<Vec<u8>> {
    let rd = Rd(kern);
    let (Some(version), Some(n)) = (rd.u16(0), rd.u16(2)) else {
        report.dropped.push("kern: table is truncated".into());
        return None;
    };
    if version != 0 {
        report.dropped.push(format!(
            "kern: header version {version} (Apple kern) is not preserved"
        ));
        return None;
    }
    let mut pos = 4usize;
    let mut subtables: Vec<Vec<u8>> = Vec::new();
    for i in 0..n {
        let (Some(len), Some(cov)) = (rd.u16(pos + 2), rd.u16(pos + 4)) else {
            report.dropped.push(format!("kern subtable {i}: truncated"));
            break;
        };
        let format = cov >> 8;
        let (horizontal, minimum, cross) = (cov & 1 != 0, cov & 2 != 0, cov & 4 != 0);
        let this = pos;
        pos = if len == 0 {
            kern.len()
        } else {
            pos + len as usize
        };
        if format != 0 || !horizontal || minimum || cross {
            report.dropped.push(format!(
                "kern subtable {i}: format {format} (horizontal {horizontal}, minimum {minimum}, cross-stream {cross}) is not preserved"
            ));
            continue;
        }
        let Some(np) = rd.u16(this + 6) else {
            report.dropped.push(format!("kern subtable {i}: truncated"));
            continue;
        };
        let mut pairs: Vec<(u16, u16, i16)> = Vec::new();
        let mut ok = true;
        for k in 0..np as usize {
            let at = this + 14 + 6 * k;
            let (Some(l), Some(r), Some(v)) = (rd.u16(at), rd.u16(at + 2), rd.i16(at + 4)) else {
                ok = false;
                break;
            };
            if let (Some(l), Some(r)) = (map(l), map(r)) {
                pairs.push((l, r, v));
            }
        }
        if !ok {
            report
                .dropped
                .push(format!("kern subtable {i}: truncated pair list"));
            continue;
        }
        pairs.sort_by_key(|p| (p.0, p.1));
        pairs.dedup_by_key(|p| (p.0, p.1));
        pairs.truncate((0xFFFF - 14) / 6);
        if pairs.is_empty() {
            continue;
        }
        let np = pairs.len() as u16;
        let mut es = 0u16;
        while (1u32 << (es + 1)) <= u32::from(np) {
            es += 1;
        }
        let search = 6u16 << es;
        let mut sub = Vec::new();
        sub.extend(be16(0));
        sub.extend(be16(14 + 6 * np));
        sub.extend(be16(1));
        sub.extend(be16(np));
        sub.extend(be16(search));
        sub.extend(be16(es));
        sub.extend(be16(6 * np - search));
        for (l, r, v) in &pairs {
            sub.extend(be16(*l));
            sub.extend(be16(*r));
            sub.extend(be16(*v as u16));
        }
        report.kern_pairs += pairs.len();
        subtables.push(sub);
    }
    if subtables.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    out.extend(be16(0));
    out.extend(be16(subtables.len() as u16));
    for s in subtables {
        out.extend(s);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// GPOS / GSUB
// ---------------------------------------------------------------------------

enum Sub {
    Empty,
    Unsupported(String),
    Ok(Vec<u8>),
}

fn vr_size(format: u16) -> usize {
    2 * (format & 0x00FF).count_ones() as usize
}

fn restrict_pair_pos(st: Rd, map: &dyn Fn(u16) -> Option<u16>, report: &mut LayoutReport) -> Sub {
    let (Some(format), Some(cov_off), Some(vf1), Some(vf2)) =
        (st.u16(0), st.u16(2), st.u16(4), st.u16(6))
    else {
        return Sub::Unsupported("truncated PairPos subtable".into());
    };
    if (vf1 | vf2) & 0x00F0 != 0 {
        return Sub::Unsupported("PairPos with device tables".into());
    }
    let rec = vr_size(vf1) + vr_size(vf2);
    let Some(cov) = coverage(st, cov_off as usize) else {
        return Sub::Unsupported("unreadable coverage".into());
    };
    match format {
        1 => {
            let Some(n) = st.u16(8) else {
                return Sub::Unsupported("truncated PairPos".into());
            };
            if n as usize != cov.len() {
                return Sub::Unsupported("PairPos set count differs from coverage".into());
            }
            let mut sets: Vec<PairSet> = Vec::new();
            for (i, first) in cov.iter().enumerate() {
                let Some(set) = st.u16(10 + 2 * i).and_then(|o| st.sub(o as usize)) else {
                    return Sub::Unsupported("unreadable PairSet".into());
                };
                let Some(count) = set.u16(0) else {
                    return Sub::Unsupported("unreadable PairSet".into());
                };
                let Some(nf) = map(*first) else { continue };
                let mut recs = Vec::new();
                for k in 0..count as usize {
                    let at = 2 + k * (2 + rec);
                    let (Some(second), Some(vals)) = (set.u16(at), set.bytes(at + 2, rec)) else {
                        return Sub::Unsupported("truncated PairSet".into());
                    };
                    if let Some(ns) = map(second) {
                        recs.push((ns, vals.to_vec()));
                    }
                }
                if !recs.is_empty() {
                    recs.sort_by_key(|r| r.0);
                    recs.dedup_by_key(|r| r.0);
                    sets.push((nf, recs));
                }
            }
            if sets.is_empty() {
                return Sub::Empty;
            }
            sets.sort_by_key(|s| s.0);
            sets.dedup_by_key(|s| s.0);
            let set_bytes: Vec<Vec<u8>> = sets
                .iter()
                .map(|(_, recs)| {
                    let mut b = Vec::new();
                    b.extend(be16(recs.len() as u16));
                    for (g, v) in recs {
                        b.extend(be16(*g));
                        b.extend(v);
                    }
                    b
                })
                .collect();
            let head = 10 + 2 * sets.len();
            let total: usize = set_bytes.iter().map(Vec::len).sum();
            let covered: Vec<u16> = sets.iter().map(|s| s.0).collect();
            let cov_bytes = write_coverage(&covered);
            if head + total + cov_bytes.len() > 0xFFFF {
                return Sub::Unsupported("PairPos subtable too large after rebuilding".into());
            }
            let mut out = Vec::new();
            out.extend(be16(1));
            out.extend(be16((head + total) as u16));
            out.extend(be16(vf1));
            out.extend(be16(vf2));
            out.extend(be16(sets.len() as u16));
            let mut off = head;
            for b in &set_bytes {
                out.extend(be16(off as u16));
                off += b.len();
            }
            for b in &set_bytes {
                out.extend(b);
            }
            out.extend(cov_bytes);
            report.gpos_pairs += sets.iter().map(|s| s.1.len()).sum::<usize>();
            Sub::Ok(out)
        }
        2 => {
            let (Some(cd1_off), Some(cd2_off), Some(c1), Some(c2)) =
                (st.u16(8), st.u16(10), st.u16(12), st.u16(14))
            else {
                return Sub::Unsupported("truncated PairPos".into());
            };
            let (Some(cd1), Some(cd2)) = (
                classdef(st, cd1_off as usize),
                classdef(st, cd2_off as usize),
            ) else {
                return Sub::Unsupported("unreadable class definition".into());
            };
            let Some(matrix) = st.bytes(16, c1 as usize * c2 as usize * rec) else {
                return Sub::Unsupported("truncated class matrix".into());
            };
            let mut kept_cov: Vec<u16> = cov.iter().filter_map(|g| map(*g)).collect();
            if kept_cov.is_empty() {
                return Sub::Empty;
            }
            kept_cov.sort_unstable();
            kept_cov.dedup();
            let map_class = |cd: &BTreeMap<u16, u16>| -> BTreeMap<u16, u16> {
                cd.iter()
                    .filter_map(|(g, c)| map(*g).map(|n| (n, *c)))
                    .collect()
            };
            let new1 = write_classdef(&map_class(&cd1));
            let new2 = write_classdef(&map_class(&cd2));
            let cov_bytes = write_coverage(&kept_cov);
            let head = 16 + matrix.len();
            if head + cov_bytes.len() + new1.len() + new2.len() > 0xFFFF {
                return Sub::Unsupported("PairPos subtable too large after rebuilding".into());
            }
            let mut out = Vec::new();
            out.extend(be16(2));
            out.extend(be16(head as u16));
            out.extend(be16(vf1));
            out.extend(be16(vf2));
            out.extend(be16((head + cov_bytes.len()) as u16));
            out.extend(be16((head + cov_bytes.len() + new1.len()) as u16));
            out.extend(be16(c1));
            out.extend(be16(c2));
            out.extend(matrix);
            out.extend(cov_bytes);
            out.extend(new1);
            out.extend(new2);
            report.gpos_class_subtables += 1;
            Sub::Ok(out)
        }
        f => Sub::Unsupported(format!("PairPos format {f} is not preserved")),
    }
}

fn restrict_ligature(st: Rd, map: &dyn Fn(u16) -> Option<u16>, report: &mut LayoutReport) -> Sub {
    let Some(ligs) = read_ligatures(st) else {
        return Sub::Unsupported("unreadable ligature subtable".into());
    };
    let mut by_first: BTreeMap<u16, Vec<(u16, Vec<u16>)>> = BTreeMap::new();
    // the original priority order is kept inside each set
    for (first, glyph, rest) in ligs {
        let (Some(nf), Some(ng)) = (map(first), map(glyph)) else {
            continue;
        };
        let nrest: Option<Vec<u16>> = rest.iter().map(|g| map(*g)).collect();
        if let Some(nrest) = nrest {
            by_first.entry(nf).or_default().push((ng, nrest));
        }
    }
    if by_first.is_empty() {
        return Sub::Empty;
    }
    let sets: Vec<Vec<u8>> = by_first
        .values()
        .map(|ligs| {
            let bodies: Vec<Vec<u8>> = ligs
                .iter()
                .map(|(g, rest)| {
                    let mut b = Vec::new();
                    b.extend(be16(*g));
                    b.extend(be16(rest.len() as u16 + 1));
                    for r in rest {
                        b.extend(be16(*r));
                    }
                    b
                })
                .collect();
            let mut set = Vec::new();
            set.extend(be16(bodies.len() as u16));
            let mut off = 2 + 2 * bodies.len();
            for b in &bodies {
                set.extend(be16(off as u16));
                off += b.len();
            }
            for b in &bodies {
                set.extend(b);
            }
            set
        })
        .collect();
    let firsts: Vec<u16> = by_first.keys().copied().collect();
    let head = 6 + 2 * sets.len();
    let total: usize = sets.iter().map(Vec::len).sum();
    let cov_bytes = write_coverage(&firsts);
    if head + total + cov_bytes.len() > 0xFFFF {
        return Sub::Unsupported("ligature subtable too large after rebuilding".into());
    }
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16((head + total) as u16));
    out.extend(be16(sets.len() as u16));
    let mut off = head;
    for s in &sets {
        out.extend(be16(off as u16));
        off += s.len();
    }
    for s in &sets {
        out.extend(s);
    }
    out.extend(cov_bytes);
    report.ligatures += by_first.values().map(Vec::len).sum::<usize>();
    Sub::Ok(out)
}

struct OutLookup {
    kind: u16,
    flag: u16,
    subtables: Vec<Vec<u8>>,
}

fn lookup_bytes(l: &OutLookup) -> Option<Vec<u8>> {
    let head = 6 + 2 * l.subtables.len();
    let mut out = Vec::new();
    out.extend(be16(l.kind));
    out.extend(be16(l.flag));
    out.extend(be16(l.subtables.len() as u16));
    let mut off = head;
    for s in &l.subtables {
        out.extend(be16(u16::try_from(off).ok()?));
        off += s.len();
    }
    for s in &l.subtables {
        out.extend(s);
    }
    Some(out)
}

fn lookup_kind_name(gsub: bool, kind: u16) -> &'static str {
    match (gsub, kind) {
        (true, 1) => "single substitution",
        (true, 2) => "multiple substitution",
        (true, 3) => "alternate substitution",
        (true, 5) | (true, 6) => "contextual substitution",
        (true, 8) => "reverse chaining substitution",
        (false, 1) => "single adjustment",
        (false, 3) => "cursive attachment",
        (false, 4..=6) => "mark attachment",
        (false, 7) | (false, 8) => "contextual positioning",
        _ => "this kind",
    }
}

/// Rebuilds a GSUB (`gsub == true`) or GPOS table.
fn rebuild_layout(
    table: &[u8],
    gsub: bool,
    map: &dyn Fn(u16) -> Option<u16>,
    report: &mut LayoutReport,
) -> Option<Vec<u8>> {
    let name = if gsub { "GSUB" } else { "GPOS" };
    let want_kind = if gsub { 4 } else { 2 };
    let ext_kind = if gsub { 7 } else { 9 };
    let rd = Rd(table);
    let Some((_, feat_off, list_off)) = layout_header(rd) else {
        report
            .dropped
            .push(format!("{name}: unsupported or truncated header"));
        return None;
    };
    let mut kept: Vec<(usize, OutLookup)> = Vec::new();
    for i in 0..lookup_count(rd, list_off) {
        let Some(l) = read_lookup(rd, list_off, i, ext_kind) else {
            report
                .dropped
                .push(format!("{name} lookup {i}: unreadable"));
            continue;
        };
        if l.kind != want_kind {
            report.dropped.push(format!(
                "{name} lookup {i}: type {} ({}) is not preserved",
                l.kind,
                lookup_kind_name(gsub, l.kind)
            ));
            continue;
        }
        if l.flag & 0x0010 != 0 {
            report.dropped.push(format!(
                "{name} lookup {i}: uses a mark filtering set (GDEF is not kept)"
            ));
            continue;
        }
        let mut subs = Vec::new();
        let mut failed = None;
        // counters are applied only when the whole lookup survives
        let mut local = LayoutReport::default();
        for st in &l.subtables {
            let r = if gsub {
                restrict_ligature(*st, map, &mut local)
            } else {
                restrict_pair_pos(*st, map, &mut local)
            };
            match r {
                Sub::Ok(b) => subs.push(b),
                Sub::Empty => {}
                Sub::Unsupported(why) => {
                    failed = Some(why);
                    break;
                }
            }
        }
        if let Some(why) = failed {
            report.dropped.push(format!("{name} lookup {i}: {why}"));
            continue;
        }
        if subs.is_empty() {
            continue;
        }
        let out = OutLookup {
            kind: want_kind,
            flag: l.flag,
            subtables: subs,
        };
        if lookup_bytes(&out).is_none() {
            report
                .dropped
                .push(format!("{name} lookup {i}: too large after rebuilding"));
            continue;
        }
        report.gpos_pairs += local.gpos_pairs;
        report.gpos_class_subtables += local.gpos_class_subtables;
        report.ligatures += local.ligatures;
        kept.push((i, out));
    }
    if kept.is_empty() {
        return None;
    }
    let new_index: BTreeMap<usize, u16> = kept
        .iter()
        .enumerate()
        .map(|(new, (old, _))| (*old, new as u16))
        .collect();

    // features of the original, reduced to the kept lookups
    let mut features: Vec<([u8; 4], Vec<u16>)> = Vec::new();
    if let Some(fl) = rd.sub(feat_off) {
        let count = fl.u16(0).unwrap_or(0) as usize;
        for f in 0..count {
            let (Some(tag), Some(off)) = (fl.bytes(2 + 6 * f, 4), fl.u16(6 + 6 * f)) else {
                break;
            };
            let Some(ft) = fl.sub(off as usize) else {
                continue;
            };
            let k = ft.u16(2).unwrap_or(0) as usize;
            let mut idx: Vec<u16> = (0..k)
                .filter_map(|j| ft.u16(4 + 2 * j))
                .filter_map(|l| new_index.get(&(l as usize)).copied())
                .collect();
            idx.sort_unstable();
            idx.dedup();
            let tag = [tag[0], tag[1], tag[2], tag[3]];
            if !idx.is_empty() && !features.iter().any(|(t, i)| *t == tag && *i == idx) {
                features.push((tag, idx));
            }
        }
    }
    if features.is_empty() {
        report.dropped.push(format!(
            "{name}: no feature refers to a kept lookup, the table is not written"
        ));
        return None;
    }

    let mut script = Vec::new();
    script.extend(be16(1));
    script.extend(b"DFLT");
    script.extend(be16(8));
    script.extend(be16(4));
    script.extend(be16(0));
    script.extend(be16(0));
    script.extend(be16(0xFFFF));
    script.extend(be16(features.len() as u16));
    for i in 0..features.len() as u16 {
        script.extend(be16(i));
    }
    let mut flist = Vec::new();
    flist.extend(be16(features.len() as u16));
    let mut ftabs: Vec<u8> = Vec::new();
    let base = 2 + 6 * features.len();
    for (tag, idx) in &features {
        flist.extend(tag);
        flist.extend(be16((base + ftabs.len()) as u16));
        ftabs.extend(be16(0));
        ftabs.extend(be16(idx.len() as u16));
        for i in idx {
            ftabs.extend(be16(*i));
        }
    }
    flist.extend(ftabs);
    let mut llist = Vec::new();
    llist.extend(be16(kept.len() as u16));
    let base = 2 + 2 * kept.len();
    let mut body: Vec<u8> = Vec::new();
    for (_, l) in &kept {
        match u16::try_from(base + body.len()) {
            Ok(o) => llist.extend(be16(o)),
            Err(_) => {
                report
                    .dropped
                    .push(format!("{name}: too large after rebuilding"));
                return None;
            }
        }
        body.extend(lookup_bytes(l)?);
    }
    llist.extend(body);
    let list_start = 10 + script.len() + flist.len();
    let mut t = Vec::new();
    t.extend(0x0001_0000u32.to_be_bytes());
    t.extend(be16(10));
    t.extend(be16((10 + script.len()) as u16));
    t.extend(be16(u16::try_from(list_start).ok()?));
    t.extend(script);
    t.extend(flist);
    t.extend(llist);
    Some(t)
}

/// Rebuilds `kern`, `GPOS` and `GSUB` of `data` for the kept glyphs; `map` gives the new id of an old glyph id
/// (`None` = glyph dropped). Returns the tables to add to the subset font and what was kept or dropped.
pub fn subset_layout(data: &[u8], map: &dyn Fn(u16) -> Option<u16>) -> (TableList, LayoutReport) {
    let mut report = LayoutReport::default();
    let mut tables = Vec::new();
    if let Some(kern) = sfnt_table(data, b"kern") {
        if let Some(t) = rebuild_kern(kern, map, &mut report) {
            tables.push((*b"kern", t));
        }
    }
    if let Some(gpos) = sfnt_table(data, b"GPOS") {
        if let Some(t) = rebuild_layout(gpos, false, map, &mut report) {
            tables.push((*b"GPOS", t));
        }
    }
    if let Some(gsub) = sfnt_table(data, b"GSUB") {
        if let Some(t) = rebuild_layout(gsub, true, map, &mut report) {
            tables.push((*b"GSUB", t));
        }
    }
    (tables, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfont::{layout_glyphs as g, tiny_ttf, tiny_ttf_layout};

    /// Offset of table `tag` inside the font file.
    fn table_at(data: &[u8], tag: &[u8; 4]) -> usize {
        let t = sfnt_table(data, tag).expect("table");
        t.as_ptr() as usize - data.as_ptr() as usize
    }

    fn put16(data: &mut [u8], at: usize, v: u16) {
        data[at..at + 2].copy_from_slice(&v.to_be_bytes());
    }

    fn get16(data: &[u8], at: usize) -> usize {
        u16::from_be_bytes([data[at], data[at + 1]]) as usize
    }

    /// Offset (in the file) of lookup `i` of GPOS/GSUB.
    fn lookup_at(data: &[u8], tag: &[u8; 4], i: usize) -> usize {
        let base = table_at(data, tag);
        let list = base + get16(data, base + 8);
        list + get16(data, list + 2 + 2 * i)
    }

    fn keep_all(gid: u16) -> Option<u16> {
        Some(gid)
    }

    fn run(data: &[u8]) -> (TableList, LayoutReport) {
        subset_layout(data, &keep_all)
    }

    #[test]
    fn sfnt_table_lookup() {
        let d = tiny_ttf_layout(false);
        assert!(sfnt_table(&d, b"kern").is_some());
        assert!(sfnt_table(&d, b"glyf").is_some());
        assert!(sfnt_table(&d, b"CFF ").is_none());
        assert!(sfnt_table(&d[..10], b"kern").is_none());
        assert!(sfnt_table(&d[..40], b"kern").is_none());
        assert!(sfnt_table(&[], b"kern").is_none());
        // a table directory entry pointing outside the file
        let mut bad = d.clone();
        let rec = 12; // first table record
        bad[rec + 8..rec + 12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(sfnt_table(&bad, &[bad[rec], bad[rec + 1], bad[rec + 2], bad[rec + 3]]).is_none());
    }

    #[test]
    fn keeping_everything_keeps_everything() {
        let d = tiny_ttf_layout(false);
        let (tables, rep) = run(&d);
        let tags: Vec<&[u8; 4]> = tables.iter().map(|t| &t.0).collect();
        assert_eq!(tags, [b"kern", b"GPOS", b"GSUB"]);
        assert_eq!(rep.kern_pairs, 5);
        assert_eq!(rep.gpos_pairs, 4);
        assert_eq!(rep.gpos_class_subtables, 1);
        assert_eq!(rep.ligatures, 2);
        assert_eq!(rep.dropped.len(), 2);
    }

    #[test]
    fn identity_rebuild_is_stable() {
        // rebuilding the rebuilt tables changes nothing: the writer and the reader agree
        let d = tiny_ttf_layout(false);
        let (once, r1) = run(&d);
        let font = {
            let mut extra = once.clone();
            extra.sort_by_key(|t| t.0);
            // a font made of only the rebuilt tables
            let mut out = vec![0, 1, 0, 0];
            out.extend((extra.len() as u16).to_be_bytes());
            out.extend([0u8; 6]);
            let mut body: Vec<u8> = Vec::new();
            let off = 12 + 16 * extra.len();
            for (tag, t) in &extra {
                out.extend(tag);
                out.extend([0u8; 4]);
                out.extend(((off + body.len()) as u32).to_be_bytes());
                out.extend((t.len() as u32).to_be_bytes());
                body.extend(t);
            }
            out.extend(body);
            out
        };
        let (twice, r2) = run(&font);
        assert_eq!(once, twice);
        assert_eq!(
            (r1.kern_pairs, r1.gpos_pairs, r1.ligatures),
            (r2.kern_pairs, r2.gpos_pairs, r2.ligatures)
        );
        assert!(r2.dropped.is_empty());
    }

    #[test]
    fn ligature_closure_adds_only_complete_ligatures() {
        let d = tiny_ttf_layout(false);
        let mut s = BTreeSet::from([0, g::F, g::I]);
        assert_eq!(ligature_closure(&d, &mut s), 2);
        assert!(s.contains(&g::FI) && s.contains(&g::FFI));
        assert_eq!(ligature_closure(&d, &mut s), 0, "idempotent");
        let mut s = BTreeSet::from([0, g::F]);
        assert_eq!(ligature_closure(&d, &mut s), 0);
        let mut s = BTreeSet::from([0, g::I]);
        assert_eq!(ligature_closure(&d, &mut s), 0);
        assert_eq!(ligature_closure(&tiny_ttf(0), &mut s), 0, "no GSUB");
        assert_eq!(ligature_closure(&[], &mut s), 0);
        // wrong GSUB version
        let mut bad = tiny_ttf_layout(false);
        let at = table_at(&bad, b"GSUB");
        put16(&mut bad, at, 2);
        let mut s = BTreeSet::from([0, g::F, g::I]);
        assert_eq!(ligature_closure(&bad, &mut s), 0);
    }

    #[test]
    fn unsupported_lookups_are_dropped_and_named() {
        // mark filtering set
        let mut d = tiny_ttf_layout(false);
        let l = lookup_at(&d, b"GPOS", 0);
        put16(&mut d, l + 2, 0x0010);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped
                .iter()
                .any(|m| m == "GPOS lookup 0: uses a mark filtering set (GDEF is not kept)"),
            "{:?}",
            rep.dropped
        );
        assert_eq!(rep.gpos_pairs, 0, "its pairs are not counted");
        assert_eq!(rep.gpos_class_subtables, 1, "the other lookup stays");

        // device tables in a value record
        let mut d = tiny_ttf_layout(false);
        let l = lookup_at(&d, b"GPOS", 0);
        let st = l + get16(&d, l + 6);
        put16(&mut d, st + 4, 0x0014);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped
                .iter()
                .any(|m| m == "GPOS lookup 0: PairPos with device tables"),
            "{:?}",
            rep.dropped
        );

        // PairPos format 3 does not exist
        let mut d = tiny_ttf_layout(false);
        let l = lookup_at(&d, b"GPOS", 1);
        let st = l + get16(&d, l + 6);
        put16(&mut d, st, 3);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped
                .iter()
                .any(|m| m == "GPOS lookup 1: PairPos format 3 is not preserved"),
            "{:?}",
            rep.dropped
        );

        // unreadable coverage
        let mut d = tiny_ttf_layout(false);
        let l = lookup_at(&d, b"GPOS", 0);
        let st = l + get16(&d, l + 6);
        let cov = st + get16(&d, st + 2);
        put16(&mut d, cov, 9);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped
                .iter()
                .any(|m| m == "GPOS lookup 0: unreadable coverage"),
            "{:?}",
            rep.dropped
        );

        // other lookup types
        let mut d = tiny_ttf_layout(false);
        for (kind, name) in [
            (3u16, "cursive attachment"),
            (5, "mark attachment"),
            (7, "contextual positioning"),
            (99, "this kind"),
        ] {
            let l = lookup_at(&d, b"GPOS", 2);
            put16(&mut d, l, kind);
            let (_, rep) = run(&d);
            let want = format!("GPOS lookup 2: type {kind} ({name}) is not preserved");
            assert!(rep.dropped.contains(&want), "{want} in {:?}", rep.dropped);
        }
        for (kind, name) in [
            (2u16, "multiple substitution"),
            (3, "alternate substitution"),
            (5, "contextual substitution"),
            (8, "reverse chaining substitution"),
        ] {
            let l = lookup_at(&d, b"GSUB", 1);
            put16(&mut d, l, kind);
            let (_, rep) = run(&d);
            let want = format!("GSUB lookup 1: type {kind} ({name}) is not preserved");
            assert!(rep.dropped.contains(&want), "{want} in {:?}", rep.dropped);
        }
    }

    #[test]
    fn broken_headers_and_lookups_are_reported() {
        for tag in [b"GPOS", b"GSUB"] {
            let mut d = tiny_ttf_layout(false);
            let at = table_at(&d, tag);
            put16(&mut d, at, 7);
            let (tables, rep) = run(&d);
            assert!(!tables.iter().any(|t| &t.0 == tag));
            assert_eq!(
                rep.dropped
                    .iter()
                    .filter(|m| m.ends_with("unsupported or truncated header"))
                    .count(),
                1
            );
        }
        // a lookup offset past the end of the table
        let mut d = tiny_ttf_layout(false);
        let base = table_at(&d, b"GSUB");
        let list = base + get16(&d, base + 8);
        put16(&mut d, list + 2, 0xFFFF);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped.iter().any(|m| m == "GSUB lookup 0: unreadable"),
            "{:?}",
            rep.dropped
        );
        // an extension subtable with a wrong format
        let mut d = tiny_ttf_layout(true);
        let l = lookup_at(&d, b"GSUB", 0);
        let ext = l + get16(&d, l + 6);
        put16(&mut d, ext, 2);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped.iter().any(|m| m == "GSUB lookup 0: unreadable"),
            "{:?}",
            rep.dropped
        );
        // an extension subtable wrapping another extension lookup
        let mut d = tiny_ttf_layout(true);
        let l = lookup_at(&d, b"GSUB", 0);
        let ext = l + get16(&d, l + 6);
        put16(&mut d, ext + 2, 7);
        let (_, rep) = run(&d);
        assert!(
            rep.dropped.iter().any(|m| m == "GSUB lookup 0: unreadable"),
            "{:?}",
            rep.dropped
        );
    }

    #[test]
    fn features_without_kept_lookups_are_not_written() {
        // keep only glyphs that no GSUB ligature uses: no GSUB at all, and nothing named in the report for it
        let d = tiny_ttf_layout(false);
        let (tables, rep) = subset_layout(&d, &|gid| (gid == g::B || gid == 0).then_some(gid));
        assert!(tables.iter().all(|t| &t.0 != b"GSUB" && &t.0 != b"GPOS"));
        assert_eq!(rep.ligatures, 0);
        // a lookup that survives but whose feature points at a dropped lookup only
        let mut d = tiny_ttf_layout(false);
        let base = table_at(&d, b"GPOS");
        let feat = base + get16(&d, base + 6);
        // feature 0 ('kern') -> change its lookup indices to point at lookup 2 only (dropped), count 1
        let f0 = feat + get16(&d, feat + 2 + 4);
        put16(&mut d, f0 + 2, 1);
        put16(&mut d, f0 + 4, 2);
        let (tables, rep) = run(&d);
        // lookups 0 and 1 are kept but no feature uses them
        assert!(tables.iter().all(|t| &t.0 != b"GPOS"));
        assert!(
            rep.dropped
                .iter()
                .any(|m| m == "GPOS: no feature refers to a kept lookup, the table is not written"),
            "{:?}",
            rep.dropped
        );
    }

    // ---- kern ----------------------------------------------------------------------------------------------

    fn kern_font(kern: Vec<u8>) -> Vec<u8> {
        // replace the kern table body inside a copy of the layout font (same length needed): build a font that
        // only has this table
        let mut out = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        out.extend(b"kern");
        out.extend([0u8; 4]);
        out.extend(28u32.to_be_bytes());
        out.extend((kern.len() as u32).to_be_bytes());
        out.extend(kern);
        out
    }

    fn kern_subtable(coverage: u16, pairs: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(0u16.to_be_bytes());
        s.extend((14 + 6 * pairs.len() as u16).to_be_bytes());
        s.extend(coverage.to_be_bytes());
        s.extend((pairs.len() as u16).to_be_bytes());
        s.extend([0u8; 6]);
        for (l, r, v) in pairs {
            s.extend(l.to_be_bytes());
            s.extend(r.to_be_bytes());
            s.extend(v.to_be_bytes());
        }
        s
    }

    fn kern_table(subtables: &[Vec<u8>]) -> Vec<u8> {
        let mut t = vec![0, 0];
        t.extend((subtables.len() as u16).to_be_bytes());
        for s in subtables {
            t.extend(s);
        }
        t
    }

    #[test]
    fn kern_pairs_are_restricted_sorted_and_deduplicated() {
        let t = kern_table(&[kern_subtable(
            1,
            &[(9, 3, 5), (1, 2, -7), (1, 2, -9), (5, 6, 1)],
        )]);
        let font = kern_font(t);
        let (tables, rep) = subset_layout(&font, &|g| match g {
            1 => Some(10),
            2 => Some(11),
            9 => Some(2),
            3 => Some(3),
            _ => None,
        });
        assert_eq!(rep.kern_pairs, 2);
        let k = &tables[0].1;
        // two pairs, sorted by (left, right): (2,3) then (10,11)
        assert_eq!(&k[8..10], &1u16.to_be_bytes(), "coverage");
        assert_eq!(get16(k, 10), 2);
        assert_eq!((get16(k, 18), get16(k, 20)), (2, 3));
        assert_eq!((get16(k, 24), get16(k, 26)), (10, 11));
        assert_eq!(k[28..30], (-7i16).to_be_bytes(), "first duplicate wins");
    }

    #[test]
    fn kern_subtables_that_cannot_be_kept_are_reported() {
        let ok = kern_subtable(1, &[(1, 2, -5)]);
        let cases: [(Vec<u8>, &str); 4] = [
            (
                kern_subtable(0x0201, &[(1, 2, 3)]),
                "kern subtable 1: format 2 (horizontal true, minimum false, cross-stream false) is not preserved",
            ),
            (
                kern_subtable(0x0000, &[(1, 2, 3)]),
                "kern subtable 1: format 0 (horizontal false, minimum false, cross-stream false) is not preserved",
            ),
            (
                kern_subtable(0x0003, &[(1, 2, 3)]),
                "kern subtable 1: format 0 (horizontal true, minimum true, cross-stream false) is not preserved",
            ),
            (
                kern_subtable(0x0005, &[(1, 2, 3)]),
                "kern subtable 1: format 0 (horizontal true, minimum false, cross-stream true) is not preserved",
            ),
        ];
        for (bad, msg) in cases {
            let (tables, rep) = run(&kern_font(kern_table(&[ok.clone(), bad])));
            assert_eq!(rep.kern_pairs, 1);
            assert_eq!(rep.dropped, vec![msg.to_string()]);
            assert_eq!(tables.len(), 1);
        }
    }

    #[test]
    fn kern_headers_and_truncation() {
        // Apple header (version 1.0)
        let (t, rep) = run(&kern_font(vec![0, 1, 0, 0, 0, 0, 0, 1]));
        assert!(t.is_empty());
        assert_eq!(
            rep.dropped,
            vec!["kern: header version 1 (Apple kern) is not preserved"]
        );
        // too short for a header
        let (t, rep) = run(&kern_font(vec![0, 0]));
        assert!(t.is_empty());
        assert_eq!(rep.dropped, vec!["kern: table is truncated"]);
        // fewer subtables than announced
        let (t, rep) = run(&kern_font(vec![0, 0, 0, 2]));
        assert!(t.is_empty());
        assert_eq!(rep.dropped, vec!["kern subtable 0: truncated"]);
        // pair list cut short
        let mut s = kern_subtable(1, &[(1, 2, 3), (2, 3, 4)]);
        s.truncate(s.len() - 3);
        let (t, rep) = run(&kern_font(kern_table(&[s])));
        assert!(t.is_empty());
        assert_eq!(rep.dropped, vec!["kern subtable 0: truncated pair list"]);
        // a subtable whose header is cut
        let (_, rep) = run(&kern_font(vec![0, 0, 0, 1, 0, 0, 0, 20, 0, 1]));
        assert_eq!(rep.dropped, vec!["kern subtable 0: truncated"]);
        // nothing left after restriction: no table
        let (t, rep) = subset_layout(
            &kern_font(kern_table(&[kern_subtable(1, &[(1, 2, 3)])])),
            &|_| None,
        );
        assert!(t.is_empty());
        assert!(rep.dropped.is_empty());
        // a zero length subtable (single huge subtable) reads to the end of the table
        let mut s = kern_subtable(1, &[(1, 2, 3)]);
        s[2..4].copy_from_slice(&0u16.to_be_bytes());
        let (t, rep) = run(&kern_font(kern_table(&[s])));
        assert_eq!(rep.kern_pairs, 1);
        assert_eq!(t.len(), 1);
    }

    // ---- coverage / class definitions ----------------------------------------------------------------------

    #[test]
    fn coverage_and_classdef_formats() {
        // coverage format 2: ranges 3..5 (index 0) and 9..9 (index 3)
        let c2 = [0, 2, 0, 2, 0, 3, 0, 5, 0, 0, 0, 9, 0, 9, 0, 3];
        assert_eq!(coverage(Rd(&c2), 0), Some(vec![3, 4, 5, 9]));
        // wrong start index
        let bad = [0, 2, 0, 1, 0, 3, 0, 5, 0, 1];
        assert_eq!(coverage(Rd(&bad), 0), None);
        // end before start
        let bad = [0, 2, 0, 1, 0, 5, 0, 3, 0, 0];
        assert_eq!(coverage(Rd(&bad), 0), None);
        // unknown format, truncated, offset outside
        assert_eq!(coverage(Rd(&[0, 7, 0, 0]), 0), None);
        assert_eq!(coverage(Rd(&[0, 1, 0, 5, 0, 1]), 0), None);
        assert_eq!(coverage(Rd(&[0, 1]), 9), None);
        // class def format 1: glyphs 10, 11, 12 -> classes 1, 0, 2
        let cd1 = [0, 1, 0, 10, 0, 3, 0, 1, 0, 0, 0, 2];
        let m = classdef(Rd(&cd1), 0).unwrap();
        assert_eq!(m.into_iter().collect::<Vec<_>>(), vec![(10, 1), (12, 2)]);
        // format 2: ranges
        let cd2 = [0, 2, 0, 2, 0, 5, 0, 6, 0, 3, 0, 9, 0, 9, 0, 0];
        let m = classdef(Rd(&cd2), 0).unwrap();
        assert_eq!(m.into_iter().collect::<Vec<_>>(), vec![(5, 3), (6, 3)]);
        assert!(classdef(Rd(&[0, 2, 0, 1, 0, 6, 0, 5, 0, 1]), 0).is_none());
        assert!(classdef(Rd(&[0, 9]), 0).is_none());
        // the written form reads back identically
        let written = write_classdef(&BTreeMap::from([(3, 1), (4, 1), (5, 2), (9, 1)]));
        let back = classdef(Rd(&written), 0).unwrap();
        assert_eq!(
            back.into_iter().collect::<Vec<_>>(),
            vec![(3, 1), (4, 1), (5, 2), (9, 1)]
        );
        let back = coverage(Rd(&write_coverage(&[1, 5, 9])), 0);
        assert_eq!(back, Some(vec![1, 5, 9]));
    }

    #[test]
    fn rd_is_bounds_checked() {
        let r = Rd(&[1, 2, 3]);
        assert_eq!(r.u16(0), Some(0x0102));
        assert_eq!(r.u16(2), None);
        assert_eq!(r.i16(1), Some(0x0203));
        assert_eq!(r.u32(0), None);
        assert_eq!(r.u16(usize::MAX), None);
        assert!(r.sub(4).is_none());
        assert_eq!(r.bytes(1, 2), Some(&[2u8, 3][..]));
        assert!(r.bytes(1, usize::MAX).is_none());
    }
}
