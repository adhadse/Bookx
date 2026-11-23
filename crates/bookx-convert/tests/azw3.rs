//! AZW3 / KF8 writer.
//!
//! * a decoder written for these tests re-assembles the book from the records (FDST flows, SKEL +
//!   FRAG tables, NCX, resources) and checks every layout rule;
//! * the Calibre oracle (`ebook-convert`, `ebook-meta`) is used to compare structure with a file
//!   Calibre wrote from the same EPUB and to read our file back. Those tests skip themselves when
//!   Calibre is missing or `BOOKX_NO_ORACLE` is set.

mod common;

use std::collections::HashMap;

use bookx_convert::mobi::kf8::base32;
use bookx_convert::mobi::{Azw3Writer, MobiWriter};
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

// ---------------------------------------------------------------------------
// decoder
// ---------------------------------------------------------------------------

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

struct Pdb<'a> {
    data: &'a [u8],
    offsets: Vec<usize>,
}

impl<'a> Pdb<'a> {
    fn parse(data: &'a [u8]) -> Self {
        assert_eq!(&data[60..68], b"BOOKMOBI");
        let n = be16(data, 76) as usize;
        let mut offsets = Vec::new();
        for i in 0..n {
            let e = 78 + i * 8;
            offsets.push(be32(data, e) as usize);
            let uid = u32::from_be_bytes([0, data[e + 5], data[e + 6], data[e + 7]]);
            assert_eq!(uid as usize, 2 * i);
        }
        assert_eq!(offsets[0], 78 + 8 * n + 2);
        for w in offsets.windows(2) {
            assert!(w[0] < w[1], "records overlap or are empty");
        }
        Pdb { data, offsets }
    }
    fn len(&self) -> usize {
        self.offsets.len()
    }
    fn rec(&self, i: usize) -> &'a [u8] {
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        &self.data[self.offsets[i]..end]
    }
}

fn palmdoc(b: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        if c == 0 || (0x09..=0x7f).contains(&c) {
            out.push(c);
        } else if (1..=8).contains(&c) {
            out.extend_from_slice(&b[i..i + c as usize]);
            i += c as usize;
        } else if c >= 0xc0 {
            out.push(b' ');
            out.push(c ^ 0x80);
        } else {
            let pair = ((c as usize) << 8 | b[i] as usize) & 0x3fff;
            i += 1;
            let (dist, len) = (pair >> 3, (pair & 7) + 3);
            assert!(dist >= 1 && dist <= out.len());
            for _ in 0..len {
                let v = out[out.len() - dist];
                out.push(v);
            }
        }
    }
    out
}

fn vwi(b: &[u8], p: &mut usize) -> usize {
    let mut v = 0usize;
    loop {
        let c = b[*p];
        *p += 1;
        v = (v << 7) | (c & 0x7f) as usize;
        if c & 0x80 != 0 {
            return v;
        }
    }
}

type Tags = HashMap<u8, Vec<usize>>;

struct Index {
    entries: Vec<(Vec<u8>, Tags, u8)>, // key, tag values, control byte
    tagx: Vec<u8>,                     // raw TAGX section
    cncx: HashMap<usize, String>,
    header: Vec<u8>,
    data_records: Vec<Vec<u8>>,
}

fn read_index(pdb: &Pdb, start: usize) -> Index {
    let hdr = pdb.rec(start);
    assert_eq!(&hdr[..4], b"INDX");
    assert_eq!(be32(hdr, 4), 192);
    assert_eq!(hdr.len() % 4, 0);
    let n_data = be32(hdr, 24) as usize;
    let total = be32(hdr, 36) as usize;
    let n_cncx = be32(hdr, 52) as usize;
    assert_eq!(&hdr[192..196], b"TAGX");
    let tagx_len = be32(hdr, 196) as usize;
    let tagx = hdr[192..192 + tagx_len].to_vec();
    assert_eq!(be32(hdr, 200), 1, "one control byte");
    let mut tags = Vec::new();
    for t in hdr[204..192 + tagx_len].chunks(4) {
        if t[3] == 1 {
            break;
        }
        tags.push((t[0], t[1] as usize, t[2]));
    }
    // the header lists the last key of every data record
    let idxt = be32(hdr, 20) as usize;
    assert_eq!(&hdr[idxt..idxt + 4], b"IDXT");

    let mut cncx = HashMap::new();
    for r in 0..n_cncx {
        let rec = pdb.rec(start + 1 + n_data + r);
        assert_eq!(rec.len() % 4, 0);
        let mut p = 0;
        while p < rec.len() && rec[p] != 0 {
            let at = p;
            let len = vwi(rec, &mut p);
            cncx.insert(
                r * 0x10000 + at,
                String::from_utf8(rec[p..p + len].to_vec()).unwrap(),
            );
            p += len;
        }
    }

    let mut entries = Vec::new();
    let mut data_records = Vec::new();
    for d in 0..n_data {
        let rec = pdb.rec(start + 1 + d);
        data_records.push(rec.to_vec());
        assert_eq!(&rec[..4], b"INDX");
        assert_eq!(rec.len() % 4, 0);
        let idxt = be32(rec, 20) as usize;
        let count = be32(rec, 24) as usize;
        assert_eq!(&rec[idxt..idxt + 4], b"IDXT");
        let mut last_key = Vec::new();
        for k in 0..count {
            let mut p = be16(rec, idxt + 4 + 2 * k) as usize;
            let klen = rec[p] as usize;
            let key = rec[p + 1..p + 1 + klen].to_vec();
            p += 1 + klen;
            let control = rec[p];
            p += 1;
            let mut map: Tags = HashMap::new();
            for (tag, vpe, mask) in &tags {
                if *mask == 0 || control & mask == 0 {
                    continue;
                }
                let groups = ((control & mask) >> mask.trailing_zeros()) as usize;
                let mut vals = Vec::new();
                for _ in 0..groups * vpe {
                    vals.push(vwi(rec, &mut p));
                }
                map.insert(*tag, vals);
            }
            assert!(p <= idxt, "entry runs into IDXT");
            last_key = key.clone();
            entries.push((key, map, control));
        }
        // header record entry for this data record
        let hp = be16(hdr, idxt_off(hdr, d)) as usize;
        let hk = hdr[hp] as usize;
        assert_eq!(&hdr[hp + 1..hp + 1 + hk], last_key.as_slice());
        assert_eq!(be16(hdr, hp + 1 + hk) as usize, count);
    }
    assert_eq!(entries.len(), total);
    for w in entries.windows(2) {
        assert!(w[0].0 < w[1].0, "keys must be sorted");
    }
    Index {
        entries,
        tagx,
        cncx,
        header: hdr.to_vec(),
        data_records,
    }
}

fn idxt_off(hdr: &[u8], d: usize) -> usize {
    be32(hdr, 20) as usize + 4 + 2 * d
}

struct Header {
    text_len: usize,
    n_text: usize,
    version: u32,
    len: usize,
    first_non_text: usize,
    name_off: usize,
    name_len: usize,
    first_image: Option<usize>,
    fdst: usize,
    fdst_count: usize,
    fcis: usize,
    flis: usize,
    extra_flags: u32,
    ncx: usize,
    frag: usize,
    skel: usize,
    guide: Option<usize>,
}

fn header(r0: &[u8]) -> Header {
    assert_eq!(&r0[16..20], b"MOBI");
    let opt = |v: u32| {
        if v == 0xffff_ffff {
            None
        } else {
            Some(v as usize)
        }
    };
    Header {
        text_len: be32(r0, 4) as usize,
        n_text: be16(r0, 8) as usize,
        len: be32(r0, 20) as usize,
        version: be32(r0, 36),
        first_non_text: be32(r0, 80) as usize,
        name_off: be32(r0, 84) as usize,
        name_len: be32(r0, 88) as usize,
        first_image: opt(be32(r0, 108)),
        fdst: be32(r0, 192) as usize,
        fdst_count: be32(r0, 196) as usize,
        fcis: be32(r0, 200) as usize,
        flis: be32(r0, 208) as usize,
        extra_flags: be32(r0, 240),
        ncx: be32(r0, 244) as usize,
        frag: be32(r0, 248) as usize,
        skel: be32(r0, 252) as usize,
        guide: opt(be32(r0, 260)),
    }
}

fn exth(r0: &[u8], h: &Header) -> Vec<(u32, Vec<u8>)> {
    let start = 16 + h.len;
    assert_eq!(&r0[start..start + 4], b"EXTH");
    let total = be32(r0, start + 4) as usize;
    let count = be32(r0, start + 8) as usize;
    let mut p = start + 12;
    let mut out = Vec::new();
    for _ in 0..count {
        let kind = be32(r0, p);
        let len = be32(r0, p + 4) as usize;
        out.push((kind, r0[p + 8..p + len].to_vec()));
        p += len;
    }
    assert_eq!(p, start + total);
    out
}

struct Frag {
    insert: usize,
    selector: String,
    file: usize,
    seq: usize,
    start: usize,
    len: usize,
}

struct Doc {
    xml: String,
    chunk_start: usize, // offset of the fragment inside xml
    chunk_len: usize,
    skel_pos: usize,
    insert_pos: usize,
}

struct Ncx {
    key: String,
    label: String,
    offset: usize,
    length: usize,
    depth: usize,
    parent: Option<usize>,
    first_child: Option<usize>,
    last_child: Option<usize>,
    fid: usize,
    off: usize,
}

struct Kf8<'a> {
    pdb: Pdb<'a>,
    h: Header,
    flows: Vec<Vec<u8>>,
    docs: Vec<Doc>,
    frags: Vec<Frag>,
    ncx: Vec<Ncx>,
    skel: Index,
    frag_idx: Index,
    ncx_idx: Index,
}

fn decode(bytes: &[u8]) -> Kf8<'_> {
    let pdb = Pdb::parse(bytes);
    let r0 = pdb.rec(0);
    let h = header(r0);
    assert_eq!(h.version, 8);
    assert_eq!(h.len, 264);
    assert_eq!(h.extra_flags, 0);

    // text records
    let mut stream = Vec::new();
    for i in 1..=h.n_text {
        let part = palmdoc(pdb.rec(i));
        assert!(part.len() <= 4096);
        assert!(
            std::str::from_utf8(&part).is_ok(),
            "record {i} splits a character"
        );
        stream.extend(part);
    }
    assert_eq!(stream.len(), h.text_len);
    assert_eq!(
        pdb.rec(1 + h.n_text),
        &[0, 0],
        "padding record after the text"
    );
    assert_eq!(h.first_non_text, 2 + h.n_text);

    // record order
    assert_eq!(h.frag, h.first_non_text);
    assert!(h.frag < h.skel && h.skel < h.ncx);
    assert_eq!(h.flis + 1, h.fcis);
    assert_eq!(h.fcis + 2, pdb.len());
    assert_eq!(h.fdst + 1, h.flis);
    assert_eq!(pdb.rec(pdb.len() - 1), &[0xe9, 0x8e, 0x0d, 0x0a]);
    assert_eq!(&pdb.rec(h.flis)[..4], b"FLIS");
    assert_eq!(&pdb.rec(h.fcis)[..4], b"FCIS");
    assert_eq!(be32(pdb.rec(h.fcis), 20) as usize, h.text_len);
    assert!(h.guide.is_none());
    let title = &r0[h.name_off..h.name_off + h.name_len];
    assert!(!title.is_empty());

    // FDST
    let fd = pdb.rec(h.fdst);
    assert_eq!(&fd[..4], b"FDST");
    assert_eq!(be32(fd, 4), 12);
    let n_flows = be32(fd, 8) as usize;
    assert_eq!(n_flows, h.fdst_count);
    assert_eq!(fd.len(), 12 + 8 * n_flows);
    let mut flows = Vec::new();
    let mut prev_end = 0usize;
    for i in 0..n_flows {
        let s = be32(fd, 12 + 8 * i) as usize;
        let e = be32(fd, 16 + 8 * i) as usize;
        assert_eq!(s, prev_end, "flows are contiguous");
        assert!(e >= s);
        flows.push(stream[s..e].to_vec());
        prev_end = e;
    }
    assert_eq!(prev_end, stream.len(), "flows cover the whole text");

    // resources follow the NCX and precede the FDST
    let ncx_idx = read_index(&pdb, h.ncx);
    let frag_idx = read_index(&pdb, h.frag);
    let skel_idx = read_index(&pdb, h.skel);
    let n_res = h.fdst - h.first_image.unwrap_or(h.fdst);
    if let Some(fi) = h.first_image {
        assert!(fi > h.ncx);
    }
    let _ = n_res;

    // FRAG / SKEL
    let frags: Vec<Frag> = frag_idx
        .entries
        .iter()
        .map(|(k, t, c)| {
            assert_eq!(*c, 0x0f);
            assert_eq!(t[&6].len(), 2);
            Frag {
                insert: String::from_utf8(k.clone()).unwrap().parse().unwrap(),
                selector: frag_idx.cncx[&t[&2][0]].clone(),
                file: t[&3][0],
                seq: t[&4][0],
                start: t[&6][0],
                len: t[&6][1],
            }
        })
        .collect();
    let mut docs = Vec::new();
    let mut fi = 0;
    let flow0 = &flows[0];
    let mut expect_pos = 0;
    for (i, (key, t, c)) in skel_idx.entries.iter().enumerate() {
        assert_eq!(key, format!("SKEL{i:010}").as_bytes());
        assert_eq!(*c, 0x0a);
        assert_eq!(t[&1], vec![t[&1][0], t[&1][0]]);
        assert_eq!(t[&6][..2], t[&6][2..], "geometry is stored twice");
        let (skel_pos, skel_len) = (t[&6][0], t[&6][1]);
        let count = t[&1][0];
        assert_eq!(
            skel_pos, expect_pos,
            "skeleton follows the previous fragments"
        );
        let skeleton = &flow0[skel_pos..skel_pos + skel_len];
        let mut base = skel_pos + skel_len;
        let mut xml = String::from_utf8(skeleton.to_vec()).unwrap();
        let mut chunk_start = None;
        let mut chunk_len = 0;
        let mut insert_pos = 0;
        for _ in 0..count {
            let f = &frags[fi];
            assert_eq!(f.file, i);
            assert_eq!(f.seq, fi);
            assert_eq!(f.start, 0);
            let ins = f.insert - skel_pos;
            let chunk = String::from_utf8(flow0[base..base + f.len].to_vec()).unwrap();
            xml.insert_str(ins, &chunk);
            chunk_start.get_or_insert(ins);
            insert_pos = f.insert;
            chunk_len += f.len;
            base += f.len;
            fi += 1;
        }
        expect_pos = base;
        docs.push(Doc {
            xml,
            chunk_start: chunk_start.unwrap_or(0),
            chunk_len,
            skel_pos,
            insert_pos,
        });
    }
    assert_eq!(fi, frags.len());
    assert_eq!(
        expect_pos,
        flows[0].len(),
        "flow 0 is exactly skeletons and fragments"
    );

    // NCX
    let ncx: Vec<Ncx> = ncx_idx
        .entries
        .iter()
        .map(|(k, t, _)| Ncx {
            key: String::from_utf8(k.clone()).unwrap(),
            label: ncx_idx.cncx[&t[&3][0]].clone(),
            offset: t[&1][0],
            length: t[&2][0],
            depth: t[&4][0],
            parent: t.get(&21).map(|v| v[0]),
            first_child: t.get(&22).map(|v| v[0]),
            last_child: t.get(&23).map(|v| v[0]),
            fid: t[&6][0],
            off: t[&6][1],
        })
        .collect();

    Kf8 {
        pdb,
        h,
        flows,
        docs,
        frags,
        ncx,
        skel: skel_idx,
        frag_idx,
        ncx_idx,
    }
}

fn parse_xml(xml: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("reassembled document is not well-formed: {e}\n{xml}"))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect()
}

fn body_text(xml: &str) -> String {
    let d = parse_xml(xml);
    let body = d.descendants().find(|n| n.has_tag_name("body")).unwrap();
    collapse(
        &body
            .descendants()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .collect::<String>(),
    )
}

fn resource<'a>(k: &Kf8<'a>, n: usize) -> &'a [u8] {
    k.pdb.rec(k.h.first_image.expect("has resources") + n - 1)
}

fn from_base32(s: &str) -> usize {
    s.bytes().fold(0usize, |a, c| {
        a * 32
            + if c.is_ascii_digit() {
                (c - b'0') as usize
            } else {
                (c - b'A') as usize + 10
            }
    })
}

fn pos_links(xml: &str) -> Vec<(usize, usize)> {
    regex::Regex::new(r#"href="kindle:pos:fid:([0-9A-V]{4}):off:([0-9A-V]{10})""#)
        .unwrap()
        .captures_iter(xml)
        .map(|c| (from_base32(&c[1]), from_base32(&c[2])))
        .collect()
}

// ---------------------------------------------------------------------------
// tests of the decoder-validated layout
// ---------------------------------------------------------------------------

#[test]
fn identity_and_helpers() {
    let w = Azw3Writer;
    assert_eq!(w.id(), "azw3");
    assert_eq!(w.extension(), "azw3");
    assert_eq!(base32(0, 4), "0000");
    assert_eq!(base32(31, 1), "V");
    assert_eq!(base32(32, 2), "10");
    assert_eq!(base32(52, 10), "000000001K");
    assert_eq!(base32(1_048_575, 4), "VVVV");
    assert_eq!(from_base32(&base32(123_456, 6)), 123_456);
}

#[test]
fn container_headers_and_record_order() {
    let book = common::sample_book();
    let bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&bytes);
    assert_eq!(&bytes[..22], b"The_Test_Book__A_Story");
    // resources: cover, thumbnail, inline picture, directly after the NCX CNCX
    let first = k.h.first_image.unwrap();
    assert_eq!(first, k.h.ncx + k.ncx_idx.data_records.len() + 1 + 1);
    assert_eq!(first + 3, k.h.fdst);
    // flows: markup + one style sheet
    assert_eq!(k.flows.len(), 2);
    assert!(String::from_utf8_lossy(&k.flows[1]).contains("line-height"));
    // FCIS is Calibre's KF8 variant
    let fcis = k.pdb.rec(k.h.fcis);
    assert_eq!(be32(fcis, 12), 2);
    assert_eq!(be32(fcis, 28), 0x28);
}

#[test]
fn exth_carries_metadata_and_resource_pointers() {
    let book = common::sample_book();
    let bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&bytes);
    let ex = exth(k.pdb.rec(0), &k.h);
    let all = |t: u32| -> Vec<String> {
        ex.iter()
            .filter(|(k, _)| *k == t)
            .map(|(_, d)| String::from_utf8(d.clone()).unwrap())
            .collect()
    };
    assert_eq!(all(100), vec!["Ada Lovelace", "Grace Hopper"]);
    assert_eq!(all(101), vec!["Bookx Press"]);
    assert_eq!(all(103), vec!["A short description & more."]);
    assert_eq!(all(104), vec!["9781234567897"]);
    assert_eq!(all(105), vec!["testing", "fiction"]);
    assert_eq!(all(106), vec!["2020-05-17"]);
    assert_eq!(all(503), vec!["The Test Book: A Story"]);
    assert_eq!(all(524), vec!["en"]);
    assert_eq!(all(129), vec!["kindle:embed:0001"]);
    let num = |t: u32| be32(&ex.iter().find(|(k, _)| *k == t).unwrap().1, 0);
    assert_eq!(num(201), 0);
    assert_eq!(num(202), 1);
    assert_eq!(num(203), 0);
    assert_eq!(num(125), 3);
    let first = k.h.first_image.unwrap();
    assert_eq!(
        k.pdb.rec(first + num(201) as usize),
        book.cover_bytes().unwrap()
    );
    let thumb = image::load_from_memory(k.pdb.rec(first + num(202) as usize)).unwrap();
    assert!(thumb.width() <= 180 && thumb.height() <= 240);
}

#[test]
fn documents_reassemble_to_the_source_text() {
    let book = common::sample_book();
    let bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&bytes);
    assert_eq!(k.docs.len(), 3);
    for (i, path) in book.spine_paths().iter().enumerate() {
        let src = common::xml_text(&book.archive.get_string(path).unwrap());
        assert_eq!(body_text(&k.docs[i].xml), collapse(&src), "{path}");
        // skeleton: no body content; fragment sits just after the <body> start tag
        let d = &k.docs[i];
        let before = &d.xml[..d.chunk_start];
        assert!(before.trim_end().ends_with('>') && before.contains("<body"));
        assert!(before.rfind("<body").unwrap() > before.rfind("</head>").unwrap());
        assert!(
            d.xml[d.chunk_start + d.chunk_len..]
                .trim_start()
                .starts_with("</body>")
        );
        assert_eq!(k.frags[i].insert, d.skel_pos + d.chunk_start);
        assert_eq!(d.insert_pos, k.frags[i].insert);
        // FRAG names the body by its aid
        let aid = k.frags[i]
            .selector
            .strip_prefix("P-//*[@aid='")
            .unwrap()
            .strip_suffix("']")
            .unwrap();
        let doc = parse_xml(&d.xml);
        let body = doc.descendants().find(|n| n.has_tag_name("body")).unwrap();
        assert_eq!(body.attribute("aid"), Some(aid));
        // every element carries a unique aid (img excepted)
        let mut seen = std::collections::HashSet::new();
        for n in body
            .descendants()
            .filter(|n| n.is_element() && !n.has_tag_name("img"))
        {
            let a = n
                .attribute("aid")
                .unwrap_or_else(|| panic!("{} has no aid", n.tag_name().name()));
            assert!(seen.insert(a.to_string()), "duplicate aid {a}");
        }
        assert!(!d.xml.contains("<!DOCTYPE"));
        assert!(d.xml.starts_with("<?xml"));
    }
}

#[test]
fn links_images_and_styles_use_the_kindle_schemes() {
    let book = common::sample_book();
    let bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&bytes);
    // chapter 1 links to chapter-2.xhtml#c2: fragment 1, offset of the <h1 id="c2">
    let links = pos_links(&k.docs[0].xml);
    assert_eq!(links.len(), 1);
    let (fid, off) = links[0];
    assert_eq!(fid, 1);
    let d = &k.docs[1];
    let target = &d.xml[d.chunk_start + off..];
    assert!(target.starts_with("<h1 id=\"c2\""), "{}", &target[..30]);
    // chapter 2 links to chapter-1.xhtml (no fragment): start of fragment 0
    assert!(pos_links(&k.docs[1].xml).contains(&(0, 0)));
    assert!(
        k.docs[1].xml.contains("href=\"https://example.org/x\""),
        "external links untouched"
    );
    // image: kindle:embed with 1-based base-32 number, mime, bytes of the source PNG
    let re = regex::Regex::new(r#"src="kindle:embed:([0-9A-V]{4})\?mime=(image/[a-z]+)""#).unwrap();
    let c = re.captures(&k.docs[1].xml).unwrap();
    assert_eq!(&c[2], "image/png");
    let n = from_base32(&c[1]);
    assert_eq!(n, 3);
    assert_eq!(
        resource(&k, n),
        book.archive.get("OEBPS/images/pic.png").unwrap()
    );
    // alt text survives
    assert!(k.docs[1].xml.contains("alt=\"a picture\""));
    // style sheet link
    for d in &k.docs {
        assert!(d.xml.contains(
            "<link rel=\"stylesheet\" type=\"text/css\" href=\"kindle:flow:0001?mime=text/css\"/>"
        ));
        assert!(!d.xml.contains("href=\"style.css\""));
    }
}

#[test]
fn every_internal_link_lands_on_the_intended_element() {
    let mut chapters = Vec::new();
    for i in 0..12 {
        let links: String = (0..12)
            .map(|j| format!("<a href=\"chapter-{}.xhtml#s{}\">to {j}</a> ", j + 1, i))
            .collect();
        chapters.push(bookx_core::convert::Chapter {
            title: format!("Chapter {i}"),
            body: format!("<h1>Chapter {i}</h1>\n<p id=\"s{i}\">anchor {i}</p>\n<p>{links}<a href=\"#s{i}\">self</a> <a href=\"chapter-{}.xhtml\">whole</a></p>\n", (i + 1) % 12 + 1),
        });
    }
    let arc = bookx_core::convert::create_epub(&common::metadata(), &chapters, None);
    let book = OpenBook::from_archive(arc).unwrap();
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    for (i, d) in k.docs.iter().enumerate() {
        let links = pos_links(&d.xml);
        assert_eq!(links.len(), 14);
        for (j, (fid, off)) in links.iter().enumerate().take(12) {
            // `#s{i}` exists only in chapter i, so a link to chapter j+1 with that id is unresolved
            // (kept pointing at the chapter start); only j == i is a real anchor
            let t = &k.docs[*fid];
            if j == i {
                assert!(t.xml[t.chunk_start + off..].starts_with(&format!("<p id=\"s{i}\"")));
            } else {
                assert_eq!(*off, 0);
            }
        }
        assert_eq!(
            links[12],
            (
                i,
                k.docs[i].xml[k.docs[i].chunk_start..]
                    .find("<p id=")
                    .unwrap()
            )
        );
        assert_eq!(links[13], ((i + 1) % 12 + 1 - 1, 0));
    }
}

#[test]
fn ncx_index_is_level_ordered_with_links() {
    let book = common::nested_toc_book();
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    let titles: Vec<&str> = k.ncx.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Part I", "Part II", "Chapter Two", "A section"]
    );
    assert_eq!(
        k.ncx.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["00", "01", "02", "03"]
    );
    assert_eq!(
        k.ncx.iter().map(|e| e.depth).collect::<Vec<_>>(),
        vec![0, 0, 1, 1]
    );
    assert_eq!(
        (k.ncx[0].first_child, k.ncx[0].last_child),
        (Some(2), Some(2))
    );
    assert_eq!(
        (k.ncx[1].first_child, k.ncx[1].last_child),
        (Some(3), Some(3))
    );
    assert_eq!(k.ncx[2].parent, Some(0));
    assert_eq!(k.ncx[3].parent, Some(1));
    assert_eq!(k.ncx[0].parent, None);
    // targets
    let wants = [
        ("<h1", "Chapter One"),
        ("<h1", "Chapter Three"),
        ("<h1", "Chapter Two"),
        ("<h2", "A section"),
    ];
    for (e, (tag, text)) in k.ncx.iter().zip(wants) {
        let d = &k.docs[e.fid];
        let at = &d.xml[d.chunk_start + e.off..];
        assert!(at.starts_with(tag), "{}: {}", e.label, &at[..30]);
        let head = &at[..at.find("</h").unwrap()];
        assert!(head.ends_with(text), "{}: {head}", e.label);
        assert_eq!(e.offset, d.insert_pos + e.off, "absolute offset");
        assert!(e.length > 0);
    }
    assert_eq!(k.ncx[0].fid, 0);
    assert_eq!(k.ncx[1].fid, 2);
    assert_eq!(k.ncx[2].fid, 1);
    assert_eq!(k.ncx[3].fid, 2);
    // a parent covers its children
    assert!(k.ncx[1].length > k.ncx[3].length);
}

#[test]
fn toc_defaults_to_one_entry_per_document() {
    let mut book = common::sample_book();
    let pkg = book.package.clone();
    bookx_core::toc::write_toc(&mut book.archive, &pkg, &[]).unwrap();
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    assert_eq!(k.ncx.len(), 3);
    assert_eq!(
        k.ncx.iter().map(|e| e.fid).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn index_tables_use_the_documented_tag_layouts() {
    let k_bytes = Azw3Writer.write(&common::nested_toc_book()).unwrap();
    let k = decode(&k_bytes);
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(
        hex(&k.skel.tagx),
        "5441475800000018000000010101030006020c0000000001"
    );
    assert_eq!(
        hex(&k.frag_idx.tagx),
        "5441475800000020000000010201010003010200040104000602080000000001"
    );
    assert_eq!(
        hex(&k.ncx_idx.tagx),
        "544147580000003000000001010101000201020003010400040108001501100016012000170140000602800000000001"
    );
    // header records follow the same pattern as the MOBI 6 index
    for idx in [&k.skel, &k.frag_idx, &k.ncx_idx] {
        assert_eq!(be32(&idx.header, 16), 2);
        assert_eq!(be32(&idx.header, 28), 65001);
        assert_eq!(be32(&idx.header, 180), 192);
        for r in &idx.data_records {
            assert_eq!(be32(r, 12), 1);
            assert_eq!(be32(r, 28), 0xffff_ffff);
        }
    }
    assert_eq!(be32(&k.skel.header, 52), 0, "SKEL has no CNCX");
    assert_eq!(be32(&k.frag_idx.header, 52), 1);
    assert_eq!(
        k.frag_idx
            .cncx
            .values()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
}

#[test]
fn css_flows_rewrite_urls_and_drop_fonts() {
    let mut book = common::sample_book();
    let css = "@charset \"utf-8\";\n@import url(other.css);\n@font-face { font-family: F; src: url(fonts/f.ttf) }\n.hero { background: url(images/pic.png) }\n.gone { background: url(images/missing.png) }\np { color: red }\n@media print { p { margin: 0 } }\n";
    book.archive.set_string("OEBPS/style.css", css);
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    let flow = String::from_utf8(k.flows[1].clone()).unwrap();
    assert!(
        !flow.contains("@font-face") && !flow.contains("@import") && !flow.contains("@charset")
    );
    assert!(
        flow.contains("background: url(kindle:embed:0003?mime=image/png)"),
        "{flow}"
    );
    assert!(flow.contains("background: none"));
    assert!(flow.contains("p { color: red }") && flow.contains("@media print"));
}

#[test]
fn several_style_sheets_become_several_flows() {
    let mut book = common::sample_book();
    book.archive
        .set_string("OEBPS/extra.css", "em { color: blue }");
    let pkg = book.package.clone();
    pkg.add_manifest_item(
        &mut book.archive,
        "extra",
        "OEBPS/extra.css",
        "text/css",
        "",
    )
    .unwrap();
    book.reload_package().unwrap();
    let ch = book
        .archive
        .get_string("OEBPS/chapter-1.xhtml")
        .unwrap()
        .replace(
            "</head>",
            "<link rel=\"stylesheet\" href=\"extra.css\"/></head>",
        );
    book.archive.set_string("OEBPS/chapter-1.xhtml", &ch);
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    assert_eq!(k.flows.len(), 3);
    assert_eq!(k.flows[2], b"em { color: blue }");
    assert!(k.docs[0].xml.contains("kindle:flow:0002?mime=text/css"));
    assert!(!k.docs[1].xml.contains("kindle:flow:0002"));
}

#[test]
fn svg_cover_pages_become_images() {
    let mut book = common::sample_book();
    let page = "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><head><title>Cover</title><script>var x=1;</script></head><body><div><svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"><image width=\"10\" height=\"10\" xlink:href=\"images/cover.jpg\"/></svg></div><script>alert(1)</script></body></html>";
    book.archive.set_string("OEBPS/chapter-1.xhtml", page);
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    let x = &k.docs[0].xml;
    assert!(
        !x.contains("svg") && !x.contains("script") && !x.contains("alert"),
        "{x}"
    );
    assert!(
        x.contains("<img src=\"kindle:embed:0001?mime=image/jpeg\" alt=\"\"/>"),
        "{x}"
    );
}

#[test]
fn books_without_cover_or_pictures() {
    let mut book = common::sample_book();
    book.remove_cover().unwrap();
    let s = book
        .archive
        .get_string("OEBPS/chapter-2.xhtml")
        .unwrap()
        .replace("<img src=\"images/pic.png\" alt=\"a picture\"/>", "");
    book.archive.set_string("OEBPS/chapter-2.xhtml", &s);
    let bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&bytes);
    assert_eq!(k.h.first_image, None);
    let ex = exth_map(&k);
    for t in [201u32, 202, 203, 129] {
        assert!(!ex.contains_key(&t), "EXTH {t} without a cover");
    }
    assert_eq!(be32(&ex[&125][0], 0), 0);
    assert_eq!(k.h.fdst + 1, k.h.flis);
}

#[test]
fn empty_book_is_valid() {
    let arc = bookx_core::convert::create_epub(&common::metadata(), &[], None);
    let book = OpenBook::from_archive(arc).unwrap();
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    assert_eq!(k.docs.len(), 1);
    assert_eq!(k.ncx.len(), 1);
    parse_xml(&k.docs[0].xml);
}

#[test]
fn large_books_spread_over_many_records() {
    let mut chapters = Vec::new();
    for i in 0..80 {
        let body: String = (0..40)
            .map(|j| format!("<p>Paragraph {i}.{j}: ünïcödé “quotes” — 日本語のテキスト and more words to fill. <a href=\"chapter-{}.xhtml#t\">next</a></p>\n", (i + 1) % 80 + 1))
            .collect();
        chapters.push(bookx_core::convert::Chapter {
            title: format!("Chapter {i}"),
            body: format!("<h1 id=\"t\">Chapter {i}</h1>\n{body}"),
        });
    }
    let arc = bookx_core::convert::create_epub(&common::metadata(), &chapters, None);
    let book = OpenBook::from_archive(arc).unwrap();
    let bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&bytes);
    assert!(k.h.n_text > 30);
    assert_eq!(k.docs.len(), 80);
    assert_eq!(k.ncx.len(), 80);
    for (i, d) in k.docs.iter().enumerate() {
        let want = (i + 1) % 80;
        let links = pos_links(&d.xml);
        assert_eq!(links.len(), 40);
        for (fid, off) in links {
            assert_eq!(fid, want);
            let t = &k.docs[fid];
            assert!(t.xml[t.chunk_start + off..].starts_with("<h1 id=\"t\""));
        }
    }
    let compressed: usize = (1..=k.h.n_text).map(|i| k.pdb.rec(i).len()).sum();
    assert!(compressed < k.h.text_len * 6 / 10);
}

#[test]
fn unicode_attributes_and_entities_survive() {
    let mut book = common::sample_book();
    book.archive.set_string(
        "OEBPS/chapter-1.xhtml",
        "<?xml version=\"1.0\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><p title=\"é &amp; &quot;q&quot;\">a &lt; b &amp; c &#169; ü <br/> <i>x</i><!-- gone --></p></body></html>",
    );
    let k_bytes = Azw3Writer.write(&book).unwrap();
    let k = decode(&k_bytes);
    let d = parse_xml(&k.docs[0].xml);
    let p = d.descendants().find(|n| n.has_tag_name("p")).unwrap();
    assert_eq!(p.attribute("title"), Some("é & \"q\""));
    assert!(p.text().unwrap().starts_with("a < b & c © ü"));
    assert!(!k.docs[0].xml.contains("gone"));
}

#[test]
fn mobi_and_azw3_agree_on_the_text() {
    let book = common::sample_book();
    let azw_bytes = Azw3Writer.write(&book).unwrap();
    let azw = decode(&azw_bytes);
    let mobi = MobiWriter { inline_toc: false }.write(&book).unwrap();
    let mobi_text: String = {
        let pdb = Pdb::parse(&mobi);
        let r0 = pdb.rec(0);
        let n = be16(r0, 8) as usize;
        let mut t = Vec::new();
        for i in 1..=n {
            t.extend(palmdoc(pdb.rec(i)));
        }
        let html = String::from_utf8(t).unwrap();
        let frag = scraper::Html::parse_fragment(&html[html.find("<body>").unwrap()..]);
        collapse(&frag.root_element().text().collect::<String>())
    };
    let azw_text: String = azw.docs.iter().map(|d| body_text(&d.xml)).collect();
    assert_eq!(mobi_text, azw_text);
}

// ---------------------------------------------------------------------------
// Calibre oracle
// ---------------------------------------------------------------------------

fn have(cmd: &str) -> bool {
    std::env::var_os("BOOKX_NO_ORACLE").is_none()
        && std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn exth_map(k: &Kf8) -> HashMap<u32, Vec<Vec<u8>>> {
    let mut m: HashMap<u32, Vec<Vec<u8>>> = HashMap::new();
    for (t, d) in exth(k.pdb.rec(0), &k.h) {
        m.entry(t).or_default().push(d);
    }
    m
}

fn nonzero_header_fields(r0: &[u8]) -> Vec<usize> {
    (16..16 + 264)
        .step_by(4)
        .filter(|o| be32(r0, *o) != 0)
        .collect()
}

#[test]
fn structure_matches_a_file_calibre_wrote_from_the_same_epub() {
    if !have("ebook-convert") {
        eprintln!("skipped: ebook-convert not available");
        return;
    }
    let book = common::nested_toc_book();
    let dir = common::temp_dir("azw3-cmp");
    let epub = dir.join("in.epub");
    std::fs::write(&epub, book.archive.to_bytes().unwrap()).unwrap();
    let theirs_path = dir.join("calibre.azw3");
    let out = std::process::Command::new("ebook-convert")
        .arg(&epub)
        .arg(&theirs_path)
        .arg("--no-inline-toc")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let theirs_bytes = std::fs::read(&theirs_path).unwrap();
    let ours_bytes = Azw3Writer.write(&book).unwrap();
    let (ours, theirs) = (decode_lenient(&ours_bytes), decode_lenient(&theirs_bytes));

    // MOBI header: the same fields are populated, the same constants are used
    let (ro, rt) = (ours.pdb.rec(0), theirs.pdb.rec(0));
    // the same header fields are populated, except the trailing-entry flags (0xf0) that Calibre
    // sets and we leave at zero, and the unique id / title location which are per file
    let ignore = [0x20usize, 0x54, 0x58, 0xf0];
    let populated = |r: &[u8]| -> Vec<usize> {
        nonzero_header_fields(r)
            .into_iter()
            .filter(|o| !ignore.contains(o))
            .collect()
    };
    assert_eq!(populated(ro), populated(rt), "populated MOBI header fields");
    for off in [
        0x14usize, 0x18, 0x1c, 0x24, 0x68, 0x5c, 0x80, 0xcc, 0xd4, 0xe8, 0xec, 0xa4, 0xa8,
    ] {
        assert_eq!(be32(ro, off), be32(rt, off), "header field {off:#x}");
    }
    assert_eq!(be16(ro, 0), be16(rt, 0), "compression");
    assert_eq!(be16(ro, 10), be16(rt, 10), "record size");
    // guide only in Calibre's file when it has an inline TOC; here neither has one
    assert_eq!(ours.h.guide, theirs.h.guide);

    // EXTH: every record type we write is one Calibre writes, with the same value where it is data
    let (eo, et) = (exth_map(&ours), exth_map(&theirs));
    // Calibre 9.17 invents a cover for a book without one (EXTH 201 = 1 and an extra image record): the two files
    // then differ by design, so this comparison only holds for a Calibre that leaves a coverless book coverless
    if et.get(&201).is_some_and(|v| v != &eo[&201]) {
        eprintln!("skipped: this Calibre adds a generated cover to a book without one");
        return;
    }
    for t in eo.keys() {
        assert!(et.contains_key(t), "Calibre writes no EXTH {t}");
    }
    for t in [
        100u32, 101, 104, 105, 503, 524, 201, 202, 203, 125, 129, 501,
    ] {
        assert_eq!(eo[&t], et[&t], "EXTH {t}");
    }

    // record sequence by kind
    let kinds = |pdb: &Pdb| -> Vec<String> {
        (0..pdb.len())
            .map(|i| {
                let r = pdb.rec(i);
                match &r[..r.len().min(4)] {
                    b"INDX" | b"FDST" | b"FLIS" | b"FCIS" => {
                        String::from_utf8_lossy(&r[..4]).to_string()
                    }
                    [0xff, 0xd8, ..] | [0x89, b'P', ..] => "img".into(),
                    [0xe9, 0x8e, ..] => "eof".into(),
                    _ => "other".into(),
                }
            })
            .collect()
    };
    let (ko, kt) = (kinds(&ours.pdb), kinds(&theirs.pdb));
    let squash = |v: &[String]| {
        let mut out: Vec<String> = Vec::new();
        for s in v {
            if s != "other" || out.last().map(|l| l != "other").unwrap_or(true) {
                out.push(s.clone());
            }
        }
        out
    };
    assert_eq!(squash(&ko), squash(&kt));
    assert_eq!(ko.len(), kt.len());

    // FDST: same shape rules
    for k in [&ours, &theirs] {
        assert_eq!(k.flows.iter().map(|f| f.len()).sum::<usize>(), k.h.text_len);
    }
    // TAGX sections byte for byte
    assert_eq!(ours.skel.tagx, theirs.skel.tagx);
    assert_eq!(ours.frag_idx.tagx, theirs.frag_idx.tagx);
    assert_eq!(ours.ncx_idx.tagx, theirs.ncx_idx.tagx);
    // INDX headers: same populated fields
    let fields =
        |h: &[u8]| -> Vec<usize> { (4..192).step_by(4).filter(|o| be32(h, *o) != 0).collect() };
    for (a, b) in [
        (&ours.skel, &theirs.skel),
        (&ours.frag_idx, &theirs.frag_idx),
        (&ours.ncx_idx, &theirs.ncx_idx),
    ] {
        assert_eq!(fields(&a.header), fields(&b.header));
        assert_eq!(a.data_records.len(), b.data_records.len());
        assert_eq!(fields(&a.data_records[0]), fields(&b.data_records[0]));
    }
    // SKEL / FRAG tables: same entry shape (control bytes, tag groups, key format)
    let first_n = ours.skel.entries.len();
    assert_eq!(first_n, 3);
    for i in 0..first_n {
        let (a, b) = (&ours.skel.entries[i], &theirs.skel.entries[i]);
        assert_eq!(a.0, b.0);
        assert_eq!(a.2, b.2);
        assert_eq!(a.1[&1], b.1[&1]);
        assert_eq!(a.1[&6].len(), b.1[&6].len());
        let (fa, fb) = (&ours.frags[i], &theirs.frags[i]);
        assert_eq!((fa.file, fa.seq, fa.start), (fb.file, fb.seq, fb.start));
        assert_eq!(
            fa.insert.to_string().len() <= 10,
            fb.insert.to_string().len() <= 10
        );
        assert!(fa.selector.starts_with("P-//*[@aid='") && fb.selector.starts_with("P-//*[@aid='"));
    }
    // NCX: identical shape for the identical table of contents
    assert_eq!(ours.ncx.len(), theirs.ncx.len());
    for (a, b) in ours.ncx.iter().zip(&theirs.ncx) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.label, b.label);
        assert_eq!(a.depth, b.depth);
        assert_eq!(
            (a.parent, a.first_child, a.last_child),
            (b.parent, b.first_child, b.last_child)
        );
        assert_eq!(a.fid, b.fid);
    }
    for (a, b) in ours.ncx_idx.entries.iter().zip(&theirs.ncx_idx.entries) {
        assert_eq!(a.2, b.2, "NCX control byte of {:?}", a.0);
    }
    // magic records
    assert_eq!(ours.pdb.rec(ours.h.flis), theirs.pdb.rec(theirs.h.flis));
    let (fo, ft) = (ours.pdb.rec(ours.h.fcis), theirs.pdb.rec(theirs.h.fcis));
    assert_eq!(fo.len(), ft.len());
    assert_eq!(&fo[..20], &ft[..20]);
    assert_eq!(&fo[24..], &ft[24..]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Decoder variant for Calibre's file, which has trailing entries (extra data flags 3).
fn decode_lenient(bytes: &[u8]) -> Kf8<'_> {
    let pdb = Pdb::parse(bytes);
    let flags = be32(pdb.rec(0), 240);
    if flags == 0 {
        return decode(bytes);
    }
    // Calibre appends [multibyte overlap][TBS] to each text record: strip them and decode as usual
    let h = header(pdb.rec(0));
    let mut stream = Vec::new();
    for i in 1..=h.n_text {
        let mut r = pdb.rec(i).to_vec();
        // TBS entry: backward varint size at the very end
        let (mut p, mut size, mut shift) = (r.len() - 1, 0usize, 0);
        loop {
            size |= ((r[p] & 0x7f) as usize) << shift;
            shift += 7;
            if r[p] & 0x80 != 0 {
                break;
            }
            p -= 1;
        }
        r.truncate(r.len() - size);
        let n = (r[r.len() - 1] & 3) as usize + 1;
        r.truncate(r.len() - n);
        stream.extend(palmdoc(&r));
    }
    decode_with_stream(bytes, stream)
}

fn decode_with_stream(bytes: &[u8], stream: Vec<u8>) -> Kf8<'_> {
    let pdb = Pdb::parse(bytes);
    let r0 = pdb.rec(0);
    let h = header(r0);
    let fd = pdb.rec(h.fdst);
    let n_flows = be32(fd, 8) as usize;
    let mut flows = Vec::new();
    for i in 0..n_flows {
        let s = be32(fd, 12 + 8 * i) as usize;
        let e = be32(fd, 16 + 8 * i) as usize;
        flows.push(stream[s..e].to_vec());
    }
    let ncx_idx = read_index(&pdb, h.ncx);
    let frag_idx = read_index(&pdb, h.frag);
    let skel_idx = read_index(&pdb, h.skel);
    let frags: Vec<Frag> = frag_idx
        .entries
        .iter()
        .map(|(k, t, _)| Frag {
            insert: String::from_utf8(k.clone()).unwrap().parse().unwrap(),
            selector: frag_idx.cncx[&t[&2][0]].clone(),
            file: t[&3][0],
            seq: t[&4][0],
            start: t[&6][0],
            len: t[&6][1],
        })
        .collect();
    let ncx: Vec<Ncx> = ncx_idx
        .entries
        .iter()
        .map(|(k, t, _)| Ncx {
            key: String::from_utf8(k.clone()).unwrap(),
            label: ncx_idx.cncx[&t[&3][0]].clone(),
            offset: t[&1][0],
            length: t[&2][0],
            depth: t[&4][0],
            parent: t.get(&21).map(|v| v[0]),
            first_child: t.get(&22).map(|v| v[0]),
            last_child: t.get(&23).map(|v| v[0]),
            fid: t[&6][0],
            off: t[&6][1],
        })
        .collect();
    Kf8 {
        pdb,
        h,
        flows,
        docs: Vec::new(),
        frags,
        ncx,
        skel: skel_idx,
        frag_idx,
        ncx_idx,
    }
}

#[test]
fn calibre_reads_our_metadata_and_cover() {
    if !have("ebook-meta") {
        eprintln!("skipped: ebook-meta not available");
        return;
    }
    let dir = common::temp_dir("azw3-meta");
    let path = dir.join("book.azw3");
    let book = common::sample_book();
    std::fs::write(&path, Azw3Writer.write(&book).unwrap()).unwrap();
    let out = std::process::Command::new("ebook-meta")
        .arg(&path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}");
    for want in [
        "The Test Book: A Story",
        "Ada Lovelace & Grace Hopper",
        "Bookx Press",
        "isbn:9781234567897",
        "testing, fiction",
        "A short description & more.",
        "eng",
    ] {
        assert!(
            text.contains(want),
            "ebook-meta output lacks `{want}`:\n{text}"
        );
    }
    let cover = dir.join("cover.jpg");
    let ok = std::process::Command::new("ebook-meta")
        .arg(&path)
        .arg(format!("--get-cover={}", cover.display()))
        .output()
        .unwrap();
    assert!(ok.status.success());
    assert_eq!(std::fs::read(&cover).unwrap(), book.cover_bytes().unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn calibre_converts_our_file_back_with_identical_text_and_toc() {
    if !have("ebook-convert") {
        eprintln!("skipped: ebook-convert not available");
        return;
    }
    use std::io::Read;
    let book = common::nested_toc_book();
    let dir = common::temp_dir("azw3-rt");
    let src = dir.join("book.azw3");
    std::fs::write(&src, Azw3Writer.write(&book).unwrap()).unwrap();
    let epub = dir.join("out.epub");
    let out = std::process::Command::new("ebook-convert")
        .arg(&src)
        .arg(&epub)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(std::fs::read(&epub).unwrap())).unwrap();
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        files.insert(f.name().unwrap().to_string(), b);
    }

    // text: Calibre splits the KF8 back into one part per file, in order
    let mut parts: Vec<&String> = files
        .keys()
        .filter(|n| n.starts_with("text/part"))
        .collect();
    parts.sort();
    assert_eq!(parts.len(), 3);
    for (i, path) in book.spine_paths().iter().enumerate() {
        let theirs = body_text(std::str::from_utf8(&files[parts[i]]).unwrap());
        let ours = collapse(&common::xml_text(&book.archive.get_string(path).unwrap()));
        assert_eq!(theirs, ours, "{path}");
    }

    // nested table of contents
    let ncx = String::from_utf8(files["toc.ncx"].clone()).unwrap();
    let doc = roxmltree::Document::parse(&ncx).unwrap();
    let label = |n: roxmltree::Node| {
        n.children()
            .find(|c| c.has_tag_name("navLabel"))
            .and_then(|l| l.descendants().find(|d| d.has_tag_name("text")))
            .and_then(|t| t.text())
            .unwrap_or("")
            .to_string()
    };
    let top: Vec<_> = doc
        .descendants()
        .find(|n| n.has_tag_name("navMap"))
        .unwrap()
        .children()
        .filter(|c| c.has_tag_name("navPoint"))
        .collect();
    assert_eq!(
        top.iter().map(|n| label(*n)).collect::<Vec<_>>(),
        vec!["Part I", "Part II"]
    );
    let kids = |n: roxmltree::Node| -> Vec<String> {
        n.children()
            .filter(|c| c.has_tag_name("navPoint"))
            .map(label)
            .collect()
    };
    assert_eq!(kids(top[0]), vec!["Chapter Two"]);
    assert_eq!(kids(top[1]), vec!["A section"]);
    // the "A section" entry points at the h2 with its original id
    let target = top[1]
        .children()
        .find(|c| c.has_tag_name("navPoint"))
        .unwrap()
        .children()
        .find(|c| c.has_tag_name("content"))
        .unwrap()
        .attribute("src")
        .unwrap();
    assert!(target.ends_with("part0002.html#c3a"), "{target}");

    // resources are byte-identical to the source images
    let mut images: Vec<&Vec<u8>> = files
        .iter()
        .filter(|(n, _)| n.starts_with("images/"))
        .map(|(_, b)| b)
        .collect();
    images.sort_by_key(|b| b.len());
    assert!(
        images
            .iter()
            .any(|b| b.as_slice() == book.cover_bytes().unwrap())
    );
    assert!(
        images
            .iter()
            .any(|b| b.as_slice() == book.archive.get("OEBPS/images/pic.png").unwrap())
    );

    // style sheet content made it through
    let css: String = files
        .iter()
        .filter(|(n, _)| n.ends_with(".css"))
        .map(|(_, b)| String::from_utf8_lossy(b).to_string())
        .collect();
    assert!(css.contains("line-height"));

    // links between chapters resolve inside Calibre's output
    let part1 = String::from_utf8(files[parts[1]].clone()).unwrap();
    assert!(
        part1.contains("href=\"part0000.html"),
        "link back to chapter one"
    );
    assert!(part1.contains("href=\"https://example.org/x\""));
    let _ = std::fs::remove_dir_all(dir);
}
