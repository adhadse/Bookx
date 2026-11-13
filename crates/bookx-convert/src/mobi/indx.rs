//! INDX/TAGX/CNCX records for the table of contents (the "NCX index" of MOBI files).

/// Forward-encoded variable-width integer: 7 bits per byte, the last byte has bit 7 set.
pub fn vwi(mut n: usize) -> Vec<u8> {
    let mut bytes = vec![(n & 0x7f) as u8 | 0x80];
    n >>= 7;
    while n > 0 {
        bytes.push((n & 0x7f) as u8);
        n >>= 7;
    }
    bytes.reverse();
    bytes
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcxEntry {
    pub title: String,
    pub offset: usize,
    pub length: usize,
    pub depth: usize,
    pub parent: Option<usize>,
    pub first_child: Option<usize>,
    pub last_child: Option<usize>,
}

/// Computes lengths and parent/child links from titles, offsets and depths in document order.
pub fn link_entries(items: &[(String, usize, usize)], text_len: usize) -> Vec<NcxEntry> {
    let mut entries: Vec<NcxEntry> = items
        .iter()
        .map(|(title, offset, depth)| NcxEntry {
            title: title.clone(),
            offset: *offset,
            length: 0,
            depth: *depth,
            parent: None,
            first_child: None,
            last_child: None,
        })
        .collect();
    for i in 0..entries.len() {
        let next = entries[i + 1..]
            .iter()
            .map(|e| e.offset)
            .find(|o| *o > entries[i].offset)
            .unwrap_or(text_len);
        entries[i].length = next.saturating_sub(entries[i].offset);
    }
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..entries.len() {
        while stack
            .last()
            .is_some_and(|p| entries[*p].depth >= entries[i].depth)
        {
            stack.pop();
        }
        if let Some(&p) = stack.last() {
            entries[i].parent = Some(p);
            if entries[p].first_child.is_none() {
                entries[p].first_child = Some(i);
            }
            entries[p].last_child = Some(i);
        }
        stack.push(i);
    }
    entries
}

const TAG_OFFSET: u8 = 1;
const TAG_LENGTH: u8 = 2;
const TAG_LABEL: u8 = 3;
const TAG_DEPTH: u8 = 4;
const TAG_PARENT: u8 = 21;
const TAG_FIRST_CHILD: u8 = 22;
const TAG_LAST_CHILD: u8 = 23;

pub(crate) fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

pub(crate) fn put_u32(v: &mut [u8], at: usize, val: u32) {
    v[at..at + 4].copy_from_slice(&val.to_be_bytes());
}

fn indx_header(len: usize) -> Vec<u8> {
    let mut h = vec![0u8; len];
    h[..4].copy_from_slice(b"INDX");
    put_u32(&mut h, 4, len as u32);
    h
}

/// One tag of a TAGX table: `(tag, values per group, bit mask)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagDef(pub u8, pub u8, pub u8);

/// An index entry: its key and the already-encoded control byte(s) + values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub key: Vec<u8>,
    pub body: Vec<u8>,
}

/// Packs strings (each prefixed with its length) into 64 KiB CNCX records.
#[derive(Debug, Default)]
pub struct Cncx {
    records: Vec<Vec<u8>>,
}

impl Cncx {
    /// Adds a string and returns the offset to store in index entries
    /// (`record << 16 | offset in record`).
    pub fn add(&mut self, text: &str) -> usize {
        let bytes = text.as_bytes();
        let mut s = vwi(bytes.len());
        s.extend_from_slice(bytes);
        if self.records.is_empty() {
            self.records.push(Vec::new());
        }
        if self.records.last().map_or(0, |r| r.len()) + s.len() > 0xffff {
            pad4(self.records.last_mut().expect("non-empty"));
            self.records.push(Vec::new());
        }
        let rec = self.records.len() - 1;
        let off = rec * 0x10000 + self.records[rec].len();
        self.records[rec].extend_from_slice(&s);
        off
    }

    pub fn finish(mut self) -> Vec<Vec<u8>> {
        for r in self.records.iter_mut() {
            pad4(r);
        }
        self.records
    }
}

/// Builds `[index header, data records..., CNCX records...]` from encoded entries.
pub fn build_index(tags: &[TagDef], entries: &[RawEntry], cncx: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let encoded: Vec<Vec<u8>> = entries
        .iter()
        .map(|e| {
            let mut b = vec![e.key.len() as u8];
            b.extend_from_slice(&e.key);
            b.extend_from_slice(&e.body);
            b
        })
        .collect();

    // data records
    let mut data_records: Vec<(Vec<u8>, Vec<u8>, usize)> = Vec::new(); // (record, last key, count)
    let mut i = 0;
    while i < encoded.len() {
        let mut body: Vec<u8> = Vec::new();
        let mut offsets: Vec<u16> = Vec::new();
        let mut count = 0;
        while i < encoded.len()
            && (count == 0
                || 192 + body.len() + encoded[i].len() + 4 + 2 * (count + 1) + 4 <= 0xf000)
        {
            offsets.push((192 + body.len()) as u16);
            body.extend_from_slice(&encoded[i]);
            count += 1;
            i += 1;
        }
        let mut rec = indx_header(192);
        rec.extend_from_slice(&body);
        pad4(&mut rec);
        let idxt = rec.len();
        rec.extend_from_slice(b"IDXT");
        for o in &offsets {
            rec.extend_from_slice(&o.to_be_bytes());
        }
        pad4(&mut rec);
        put_u32(&mut rec, 12, 1);
        put_u32(&mut rec, 20, idxt as u32);
        put_u32(&mut rec, 24, count as u32);
        put_u32(&mut rec, 28, 0xffff_ffff);
        put_u32(&mut rec, 32, 0xffff_ffff);
        data_records.push((rec, entries[i - 1].key.clone(), count));
    }

    // header record
    let mut header = indx_header(192);
    header.extend_from_slice(b"TAGX");
    header.extend_from_slice(&((12 + (tags.len() + 1) * 4) as u32).to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    for t in tags {
        header.extend_from_slice(&[t.0, t.1, t.2, 0]);
    }
    header.extend_from_slice(&[0, 0, 0, 1]);
    let mut offsets: Vec<u16> = Vec::new();
    for (_, key, count) in &data_records {
        offsets.push(header.len() as u16);
        header.push(key.len() as u8);
        header.extend_from_slice(key);
        header.extend_from_slice(&(*count as u16).to_be_bytes());
    }
    pad4(&mut header);
    let idxt = header.len();
    header.extend_from_slice(b"IDXT");
    for o in &offsets {
        header.extend_from_slice(&o.to_be_bytes());
    }
    pad4(&mut header);
    put_u32(&mut header, 16, 2);
    put_u32(&mut header, 20, idxt as u32);
    put_u32(&mut header, 24, data_records.len() as u32);
    put_u32(&mut header, 28, 65001);
    put_u32(&mut header, 32, 0xffff_ffff);
    put_u32(&mut header, 36, entries.len() as u32);
    if !cncx.is_empty() {
        put_u32(&mut header, 52, cncx.len() as u32);
    }
    put_u32(&mut header, 180, 192);

    let mut out = vec![header];
    out.extend(data_records.into_iter().map(|(r, _, _)| r));
    out.extend(cncx);
    out
}

/// Builds the MOBI 6 NCX index: `[header, data..., CNCX...]`.
pub fn build_ncx_index(entries: &[NcxEntry]) -> Vec<Vec<u8>> {
    let mut cncx = Cncx::default();
    let label_offsets: Vec<usize> = entries.iter().map(|e| cncx.add(&e.title)).collect();
    let hierarchical = entries
        .iter()
        .any(|e| e.parent.is_some() || e.first_child.is_some());
    let digits = format!("{:X}", entries.len().saturating_sub(1))
        .len()
        .max(2);

    let mut tags = vec![
        TagDef(TAG_OFFSET, 1, 1),
        TagDef(TAG_LENGTH, 1, 2),
        TagDef(TAG_LABEL, 1, 4),
        TagDef(TAG_DEPTH, 1, 8),
    ];
    if hierarchical {
        tags.push(TagDef(TAG_PARENT, 1, 16));
        tags.push(TagDef(TAG_FIRST_CHILD, 1, 32));
        tags.push(TagDef(TAG_LAST_CHILD, 1, 64));
    }
    let raw: Vec<RawEntry> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut control = 0u8;
            let mut values: Vec<u8> = Vec::new();
            let mut put = |bit: u8, v: usize| {
                control |= bit;
                values.extend(vwi(v));
            };
            put(1, e.offset);
            put(2, e.length);
            put(4, label_offsets[i]);
            put(8, e.depth);
            if hierarchical {
                if let Some(p) = e.parent {
                    put(16, p);
                }
                if let Some(c) = e.first_child {
                    put(32, c);
                }
                if let Some(c) = e.last_child {
                    put(64, c);
                }
            }
            let mut body = vec![control];
            body.extend(values);
            RawEntry {
                key: format!("{i:0digits$X}").into_bytes(),
                body,
            }
        })
        .collect();
    build_index(&tags, &raw, cncx.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vwi_matches_the_documented_examples() {
        // 0x11111 forward-encoded is 04 22 91 (MobileRead wiki)
        assert_eq!(vwi(0x11111), vec![0x04, 0x22, 0x91]);
        assert_eq!(vwi(0), vec![0x80]);
        assert_eq!(vwi(109), vec![0xed]);
        assert_eq!(vwi(700), vec![0x05, 0xbc]);
        assert_eq!(vwi(127), vec![0xff]);
        assert_eq!(vwi(128), vec![0x01, 0x80]);
    }

    fn items() -> Vec<(String, usize, usize)> {
        vec![
            ("Part A".into(), 100, 0),
            ("A one".into(), 200, 1),
            ("A two".into(), 300, 1),
            ("Part B".into(), 400, 0),
        ]
    }

    #[test]
    fn entries_get_lengths_and_family_links() {
        let e = link_entries(&items(), 500);
        assert_eq!(
            e.iter().map(|e| e.length).collect::<Vec<_>>(),
            vec![100, 100, 100, 100]
        );
        assert_eq!(e[0].first_child, Some(1));
        assert_eq!(e[0].last_child, Some(2));
        assert_eq!(e[1].parent, Some(0));
        assert_eq!(e[2].parent, Some(0));
        assert_eq!(e[3].parent, None);
        assert_eq!(e[3].first_child, None);
    }

    #[test]
    fn equal_offsets_do_not_give_zero_lengths() {
        let e = link_entries(
            &[
                ("a".into(), 10, 0),
                ("b".into(), 10, 0),
                ("c".into(), 50, 0),
            ],
            90,
        );
        assert_eq!(e[0].length, 40);
        assert_eq!(e[1].length, 40);
        assert_eq!(e[2].length, 40);
    }

    #[test]
    fn record_shapes() {
        let recs = build_ncx_index(&link_entries(&items(), 500));
        assert_eq!(recs.len(), 3, "header + 1 data + 1 cncx");
        assert!(recs[0].starts_with(b"INDX") && recs[1].starts_with(b"INDX"));
        assert_eq!(recs[0].len() % 4, 0);
        assert_eq!(recs[1].len() % 4, 0);
        assert_eq!(recs[2].len() % 4, 0);
    }

    #[test]
    fn many_entries_split_into_several_data_records() {
        let many: Vec<(String, usize, usize)> = (0..6000)
            .map(|i| (format!("Chapter number {i}"), i * 10, 0))
            .collect();
        let recs = build_ncx_index(&link_entries(&many, 70000));
        let data = recs.iter().filter(|r| r.starts_with(b"INDX")).count() - 1;
        assert!(data >= 2, "{data}");
        assert!(
            recs.iter().all(|r| r.len() <= 0x10000),
            "{:?}",
            recs.iter().map(|r| r.len()).collect::<Vec<_>>()
        );
        let cncx = recs.iter().filter(|r| !r.starts_with(b"INDX")).count();
        assert!(cncx >= 2, "label strings exceed one 64 KiB record: {cncx}");
    }

    #[test]
    fn cncx_offsets_and_padding() {
        let mut c = Cncx::default();
        assert_eq!(c.add("abc"), 0);
        assert_eq!(c.add("de"), 4);
        let recs = c.finish();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0], vec![0x83, b'a', b'b', b'c', 0x82, b'd', b'e', 0]);
        assert!(Cncx::default().finish().is_empty());
    }

    #[test]
    fn cncx_starts_a_new_record_before_overflowing() {
        let mut c = Cncx::default();
        let long = "x".repeat(40_000);
        assert_eq!(c.add(&long), 0);
        let second = c.add(&long);
        assert_eq!(second, 0x10000, "second string opens record 1");
        assert_eq!(c.finish().len(), 2);
    }

    #[test]
    fn generic_index_has_tagx_entries_and_idxt() {
        let tags = [TagDef(1, 1, 1), TagDef(2, 2, 6)];
        let entries = vec![
            RawEntry {
                key: b"a".to_vec(),
                body: vec![1, 0x81],
            },
            RawEntry {
                key: b"b".to_vec(),
                body: vec![1, 0x82],
            },
        ];
        let recs = build_index(&tags, &entries, Vec::new());
        assert_eq!(recs.len(), 2);
        let h = &recs[0];
        assert_eq!(&h[192..196], b"TAGX");
        assert_eq!(
            u32::from_be_bytes([h[196], h[197], h[198], h[199]]),
            12 + 12
        );
        assert_eq!(&h[204..216], &[1, 1, 1, 0, 2, 2, 6, 0, 0, 0, 0, 1]);
        assert_eq!(u32::from_be_bytes([h[36], h[37], h[38], h[39]]), 2);
        let d = &recs[1];
        let want: Vec<u8> = vec![
            1, b'a', 1, 0x81, 1, b'b', 1, 0x82, b'I', b'D', b'X', b'T', 0, 192, 0, 196,
        ];
        assert_eq!(&d[192..192 + want.len()], want.as_slice());
    }
}
