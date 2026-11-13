//! `INDX` tables of MOBI / KF8 files: the NCX (table of contents) and, for KF8, the skeleton and
//! fragment tables that say how the HTML parts are assembled. Implemented from the public format
//! description (the layout is the one mobileread.com documents); nothing here trusts offsets
//! without bounds checks.

use std::collections::HashMap;

fn be16(b: &[u8], off: usize) -> Option<usize> {
    b.get(off..off + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]) as usize)
}

fn be32(b: &[u8], off: usize) -> Option<usize> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Reads a forward variable width integer: seven bits per byte, the last byte has the high bit
/// set. Returns the value and the number of bytes used.
pub fn read_vwi(data: &[u8]) -> Option<(usize, usize)> {
    let mut v: usize = 0;
    for (i, b) in data.iter().take(8).enumerate() {
        v = v.checked_mul(128)?.checked_add((b & 0x7F) as usize)?;
        if b & 0x80 != 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// One index entry: its key and the values of each tag.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub key: Vec<u8>,
    pub tags: HashMap<u8, Vec<usize>>,
}

impl Entry {
    pub fn first(&self, tag: u8) -> Option<usize> {
        self.tags.get(&tag).and_then(|v| v.first().copied())
    }

    pub fn pair(&self, tag: u8) -> Option<(usize, usize)> {
        let v = self.tags.get(&tag)?;
        Some((*v.first()?, *v.get(1)?))
    }
}

pub struct Index {
    pub entries: Vec<Entry>,
    /// Strings of the CNCX records, by their offset.
    pub cncx: HashMap<usize, String>,
}

struct TagDef {
    tag: u8,
    per_entry: usize,
    mask: u8,
    end: bool,
}

fn parse_tagx(rec: &[u8], header_len: usize) -> Option<(Vec<TagDef>, usize)> {
    let t = rec.get(header_len..)?;
    if t.get(0..4) != Some(b"TAGX") {
        return None;
    }
    let first = be32(t, 4)?;
    let control_bytes = be32(t, 8)?;
    let mut defs = Vec::new();
    let mut at = 12;
    while at + 4 <= first.min(t.len()) {
        defs.push(TagDef {
            tag: t[at],
            per_entry: t[at + 1] as usize,
            mask: t[at + 2],
            end: t[at + 3] & 1 != 0,
        });
        at += 4;
    }
    Some((defs, control_bytes))
}

fn decode_entry(
    data: &[u8],
    defs: &[TagDef],
    control_bytes: usize,
    cncx_ok: bool,
) -> Option<Entry> {
    let _ = cncx_ok;
    let key_len = *data.first()? as usize;
    let key = data.get(1..1 + key_len)?.to_vec();
    let mut ptr = 1 + key_len;
    let controls = data.get(ptr..ptr + control_bytes)?;
    ptr += control_bytes;
    struct Want {
        tag: u8,
        per_entry: usize,
        count: Option<usize>,
        bytes: Option<usize>,
    }
    let mut wants = Vec::new();
    let mut cb = 0usize;
    for d in defs {
        if d.end {
            cb += 1;
            continue;
        }
        let Some(&control) = controls.get(cb) else {
            break;
        };
        let mut value = control & d.mask;
        if value == 0 {
            continue;
        }
        if value == d.mask && d.mask.count_ones() > 1 {
            let (n, used) = read_vwi(data.get(ptr..)?)?;
            ptr += used;
            wants.push(Want {
                tag: d.tag,
                per_entry: d.per_entry,
                count: None,
                bytes: Some(n),
            });
            continue;
        }
        let mut m = d.mask;
        while m & 1 == 0 {
            m >>= 1;
            value >>= 1;
        }
        wants.push(Want {
            tag: d.tag,
            per_entry: d.per_entry,
            count: Some(value as usize),
            bytes: None,
        });
    }
    let mut tags: HashMap<u8, Vec<usize>> = HashMap::new();
    for w in wants {
        let list = tags.entry(w.tag).or_default();
        if let Some(count) = w.count {
            for _ in 0..count.checked_mul(w.per_entry.max(1))?.min(4096) {
                let (v, used) = read_vwi(data.get(ptr..)?)?;
                ptr += used;
                list.push(v);
            }
        } else if let Some(total) = w.bytes {
            let mut got = 0;
            while got < total && list.len() < 4096 {
                let (v, used) = read_vwi(data.get(ptr..)?)?;
                ptr += used;
                got += used;
                list.push(v);
            }
        }
    }
    Some(Entry { key, tags })
}

/// Reads the index whose first record is `records[0]`. `records` yields the raw records from
/// that position on.
pub fn read_index(record: &dyn Fn(usize) -> Option<Vec<u8>>, first: usize) -> Option<Index> {
    let head = record(first)?;
    if head.get(0..4) != Some(b"INDX") {
        return None;
    }
    let header_len = be32(&head, 4)?;
    let data_records = be32(&head, 24)?;
    let n_cncx = be32(&head, 52).unwrap_or(0);
    if data_records > 4096 || n_cncx > 4096 {
        return None;
    }
    let (defs, control_bytes) = parse_tagx(&head, header_len)?;
    let mut entries = Vec::new();
    for r in 1..=data_records {
        let rec = record(first + r)?;
        if rec.get(0..4) != Some(b"INDX") {
            return None;
        }
        let rec_header = be32(&rec, 4)?;
        let idxt = be32(&rec, 20)?;
        let count = be32(&rec, 24)?;
        if rec.get(idxt..idxt + 4) != Some(b"IDXT") || count > 65536 {
            continue;
        }
        let offsets: Vec<usize> = (0..count)
            .filter_map(|i| be16(&rec, idxt + 4 + 2 * i))
            .collect();
        for (i, &start) in offsets.iter().enumerate() {
            let end = offsets.get(i + 1).copied().unwrap_or(idxt);
            if start < rec_header || start >= end || end > rec.len() {
                continue;
            }
            if let Some(e) = decode_entry(&rec[start..end], &defs, control_bytes, true) {
                entries.push(e);
            }
        }
    }
    let mut cncx = HashMap::new();
    for c in 0..n_cncx {
        let Some(rec) = record(first + 1 + data_records + c) else {
            break;
        };
        let mut off = 0;
        while off < rec.len() {
            let Some((len, used)) = read_vwi(&rec[off..]) else {
                break;
            };
            let s = off + used;
            let Some(bytes) = rec.get(s..s + len) else {
                break;
            };
            cncx.insert(
                c * 0x10000 + off,
                String::from_utf8_lossy(bytes).into_owned(),
            );
            off = s + len;
        }
    }
    Some(Index { entries, cncx })
}

/// A table of contents entry of the NCX index.
#[derive(Debug, Clone)]
pub struct NcxEntry {
    pub title: String,
    /// Offset in the text (MOBI 6) — also set for KF8 where it is the virtual position.
    pub pos: usize,
    /// KF8: fragment index and offset in the fragment.
    pub pos_fid: Option<(usize, usize)>,
    pub parent: Option<usize>,
    pub depth: usize,
}

pub fn ncx_entries(index: &Index) -> Vec<NcxEntry> {
    index
        .entries
        .iter()
        .map(|e| NcxEntry {
            title: e
                .first(3)
                .and_then(|o| index.cncx.get(&o))
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                .unwrap_or_default(),
            pos: e.first(1).unwrap_or(usize::MAX),
            pos_fid: e.pair(6),
            parent: e.first(21),
            depth: e.first(4).unwrap_or(0),
        })
        .collect()
}

/// The skeleton table: `(fragment count, start, length)` per HTML file.
pub fn skeletons(index: &Index) -> Vec<(usize, usize, usize)> {
    index
        .entries
        .iter()
        .filter_map(|e| {
            let count = e.first(1)?;
            let (start, len) = e.pair(6)?;
            Some((count, start, len))
        })
        .collect()
}

/// A fragment: where it is inserted in the final text and its length.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub insert_pos: usize,
    pub file: usize,
    pub length: usize,
}

pub fn fragments(index: &Index) -> Vec<Fragment> {
    index
        .entries
        .iter()
        .filter_map(|e| {
            let insert_pos = std::str::from_utf8(&e.key).ok()?.trim().parse().ok()?;
            let (_, length) = e.pair(6)?;
            Some(Fragment {
                insert_pos,
                file: e.first(3).unwrap_or(0),
                length,
            })
        })
        .collect()
}

/// Nested table of contents built from the parent links (or the depths when links are missing).
#[derive(Debug, Clone)]
pub struct TocNode {
    pub title: String,
    pub entry: usize,
    pub children: Vec<TocNode>,
}

pub fn toc_tree(entries: &[NcxEntry]) -> Vec<TocNode> {
    let n = entries.len();
    let mut parent: Vec<Option<usize>> = entries
        .iter()
        .map(|e| e.parent.filter(|p| *p < n))
        .collect();
    if parent.iter().all(Option::is_none) && entries.iter().any(|e| e.depth > 0) {
        // no parent links: derive them from the depth in position order
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|i| entries[*i].pos);
        let mut stack: Vec<usize> = Vec::new();
        for &i in &order {
            while stack
                .last()
                .is_some_and(|p| entries[*p].depth >= entries[i].depth)
            {
                stack.pop();
            }
            parent[i] = stack.last().copied();
            stack.push(i);
        }
    }
    // break cycles
    for i in 0..n {
        let mut cur = parent[i];
        let mut steps = 0;
        while let Some(p) = cur {
            steps += 1;
            if p == i || steps > n {
                parent[i] = None;
                break;
            }
            cur = parent[p];
        }
    }
    fn build(i: usize, entries: &[NcxEntry], kids: &HashMap<usize, Vec<usize>>) -> TocNode {
        let mut children: Vec<usize> = kids.get(&i).cloned().unwrap_or_default();
        children.sort_by_key(|c| entries[*c].pos);
        TocNode {
            title: entries[i].title.clone(),
            entry: i,
            children: children
                .into_iter()
                .map(|c| build(c, entries, kids))
                .collect(),
        }
    }
    let mut kids: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for (i, p) in parent.iter().enumerate() {
        match p {
            Some(p) => kids.entry(*p).or_default().push(i),
            None => roots.push(i),
        }
    }
    roots.sort_by_key(|r| entries[*r].pos);
    roots
        .into_iter()
        .map(|r| build(r, entries, &kids))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vwi_reads_last_byte_with_high_bit() {
        assert_eq!(read_vwi(&[0x81]), Some((1, 1)));
        assert_eq!(read_vwi(&[0x01, 0x80]), Some((128, 2)));
        assert_eq!(read_vwi(&[0x00, 0x00]), None);
        assert_eq!(read_vwi(&[]), None);
    }

    fn e(title: &str, pos: usize, parent: Option<usize>, depth: usize) -> NcxEntry {
        NcxEntry {
            title: title.into(),
            pos,
            pos_fid: None,
            parent,
            depth,
        }
    }

    #[test]
    fn tree_from_parent_links_in_any_storage_order() {
        // level ordered storage: roots first, children after
        let v = vec![
            e("A", 0, None, 0),
            e("B", 100, None, 0),
            e("A1", 10, Some(0), 1),
            e("A2", 50, Some(0), 1),
            e("B1", 110, Some(1), 1),
        ];
        let t = toc_tree(&v);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "A");
        assert_eq!(
            t[0].children
                .iter()
                .map(|c| c.title.as_str())
                .collect::<Vec<_>>(),
            vec!["A1", "A2"]
        );
        assert_eq!(t[1].children[0].title, "B1");
    }

    #[test]
    fn tree_from_depths_when_links_are_missing() {
        let v = vec![
            e("A", 0, None, 0),
            e("A1", 10, None, 1),
            e("B", 100, None, 0),
        ];
        let t = toc_tree(&v);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].children[0].title, "A1");
    }

    #[test]
    fn parent_cycles_do_not_hang() {
        let v = vec![e("A", 0, Some(1), 0), e("B", 1, Some(0), 0)];
        assert!(!toc_tree(&v).is_empty());
    }

    #[test]
    fn garbage_is_not_an_index() {
        let rec = |_i: usize| Some(vec![0u8; 64]);
        assert!(read_index(&rec, 0).is_none());
        let rec = |_i: usize| {
            Some(
                b"INDX"
                    .iter()
                    .copied()
                    .chain([0xFF; 80])
                    .collect::<Vec<u8>>(),
            )
        };
        assert!(read_index(&rec, 0).is_none());
    }
}
