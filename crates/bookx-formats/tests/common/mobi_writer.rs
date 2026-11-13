//! A small independent MOBI / KF8 *writer* used only to build test fixtures. It shares no code
//! with the reader: headers are assembled byte by byte from the public format description.
#![allow(dead_code)]

/// PalmDoc LZ77 compressor (greedy, window 2047, matches of 3..=10 bytes).
pub fn palmdoc_compress(data: &[u8]) -> Vec<u8> {
    let plain = |c: u8| c == 0 || (0x09..0x80).contains(&c);
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if i + 3 <= data.len() {
            for j in i.saturating_sub(2047)..i {
                let mut l = 0;
                while l < 10 && i + l < data.len() && data[j + l] == data[i + l] {
                    l += 1;
                }
                if l >= 3 && l > best.1 {
                    best = (i - j, l);
                }
            }
        }
        if best.1 >= 3 {
            let v = 0x8000u16 | ((best.0 as u16) << 3) | ((best.1 - 3) as u16);
            out.extend(v.to_be_bytes());
            i += best.1;
            continue;
        }
        let c = data[i];
        if c == b' ' && i + 1 < data.len() && (0x40..0x80).contains(&data[i + 1]) {
            out.push(data[i + 1] | 0x80);
            i += 2;
            continue;
        }
        if plain(c) {
            out.push(c);
            i += 1;
            continue;
        }
        let mut n = 1;
        while n < 8 && i + n < data.len() && !plain(data[i + n]) {
            n += 1;
        }
        out.push(n as u8);
        out.extend(&data[i..i + n]);
        i += n;
    }
    out
}

#[derive(Clone)]
pub struct Kf8Spec {
    /// MOBI 6 + KF8 in one file (`true`) or KF8 only.
    pub joint: bool,
    /// `(skeleton, fragments)` per file.
    pub files: Vec<(String, Vec<String>)>,
    pub css: String,
}

#[derive(Clone)]
pub struct MobiSpec {
    pub title: String,
    pub authors: Vec<String>,
    pub publisher: String,
    pub description: String,
    pub isbn: Option<String>,
    pub subjects: Vec<String>,
    pub language: Option<String>,
    /// Text of the book; the placeholder `XXXXXXXXXX` after `filepos=` is replaced by the byte
    /// offset of the first `<p>Third chapter`.
    pub html: String,
    pub images: Vec<Vec<u8>>,
    pub cover_index: Option<u32>,
    pub compress: bool,
    pub encoding: u32,
    pub extra_flags: u32,
    pub exth_updated_title: Option<String>,
    pub compression_override: Option<u16>,
    pub encryption: u16,
    pub drm_offset: Option<u32>,
    pub kf8: Option<Kf8Spec>,
    pub no_fdst: bool,
}

impl Default for MobiSpec {
    fn default() -> Self {
        MobiSpec {
            title: String::new(),
            authors: vec![],
            publisher: String::new(),
            description: String::new(),
            isbn: None,
            subjects: vec![],
            language: None,
            html: String::new(),
            images: vec![],
            cover_index: None,
            compress: true,
            encoding: 1252,
            extra_flags: 0,
            exth_updated_title: None,
            compression_override: None,
            encryption: 0,
            drm_offset: None,
            kf8: None,
            no_fdst: false,
        }
    }
}

fn put32(v: &mut [u8], off: usize, x: u32) {
    v[off..off + 4].copy_from_slice(&x.to_be_bytes());
}

fn put16(v: &mut [u8], off: usize, x: u16) {
    v[off..off + 2].copy_from_slice(&x.to_be_bytes());
}

fn encode(s: &str, enc: u32) -> Vec<u8> {
    if enc == 65001 {
        s.as_bytes().to_vec()
    } else {
        encoding_rs::WINDOWS_1252.encode(s).0.into_owned()
    }
}

fn text_records(text: &[u8], compress: bool, extra_flags: u32) -> Vec<Vec<u8>> {
    let mut recs = Vec::new();
    for chunk in text.chunks(4096) {
        let mut r = if compress {
            palmdoc_compress(chunk)
        } else {
            chunk.to_vec()
        };
        if extra_flags & 1 != 0 {
            r.extend([0x5A, 0x01]); // multibyte overlap: (0x01 & 3) + 1 = 2 bytes
        }
        if extra_flags & 2 != 0 {
            r.extend([b'T', b'B', 0x83]); // one backward varint entry of size 3
        }
        recs.push(r);
    }
    if recs.is_empty() {
        recs.push(Vec::new());
    }
    recs
}

fn exth(spec: &MobiSpec, boundary: Option<u32>) -> Vec<u8> {
    let enc = spec.encoding;
    let mut recs: Vec<(u32, Vec<u8>)> = Vec::new();
    for a in &spec.authors {
        recs.push((100, encode(a, enc)));
    }
    if !spec.publisher.is_empty() {
        recs.push((101, encode(&spec.publisher, enc)));
    }
    if !spec.description.is_empty() {
        recs.push((103, encode(&spec.description, enc)));
    }
    if let Some(i) = &spec.isbn {
        recs.push((104, encode(i, enc)));
    }
    for s in &spec.subjects {
        recs.push((105, encode(s, enc)));
    }
    if let Some(c) = spec.cover_index {
        recs.push((201, c.to_be_bytes().to_vec()));
    }
    recs.push((204, 201u32.to_be_bytes().to_vec()));
    if let Some(b) = boundary {
        recs.push((121, b.to_be_bytes().to_vec()));
    }
    if let Some(t) = &spec.exth_updated_title {
        recs.push((503, encode(t, enc)));
    }
    if let Some(l) = &spec.language {
        recs.push((524, encode(l, enc)));
    }
    let mut body = Vec::new();
    for (t, d) in &recs {
        body.extend(t.to_be_bytes());
        body.extend(((d.len() + 8) as u32).to_be_bytes());
        body.extend(d);
    }
    let mut out = b"EXTH".to_vec();
    let total = 12 + body.len();
    let padded = total.div_ceil(4) * 4;
    out.extend((total as u32).to_be_bytes());
    out.extend((recs.len() as u32).to_be_bytes());
    out.extend(body);
    out.resize(padded, 0);
    out
}

struct Part {
    text_len: usize,
    text_recs: usize,
}

#[allow(clippy::too_many_arguments)]
fn record0(
    spec: &MobiSpec,
    part: &Part,
    version: u32,
    first_image: u32,
    fdst: Option<u32>,
    boundary: Option<u32>,
) -> Vec<u8> {
    let mut r = vec![0u8; 16 + 232];
    let comp = spec
        .compression_override
        .unwrap_or(if spec.compress { 2 } else { 1 });
    put16(&mut r, 0, comp);
    put32(&mut r, 4, part.text_len as u32);
    put16(&mut r, 8, part.text_recs as u16);
    put16(&mut r, 10, 4096);
    put16(&mut r, 12, spec.encryption);
    let m = 16;
    r[m..m + 4].copy_from_slice(b"MOBI");
    put32(&mut r, m + 4, 232);
    put32(&mut r, m + 8, 2);
    put32(&mut r, m + 12, spec.encoding);
    put32(&mut r, m + 16, 0x1234);
    put32(&mut r, m + 20, version);
    for off in (0x18..0x44).step_by(4) {
        put32(&mut r, m + off, 0xFFFF_FFFF);
    }
    let ex = exth(spec, boundary);
    let name = encode(
        if spec.title.is_empty() {
            "Untitled"
        } else {
            &spec.title
        },
        spec.encoding,
    );
    put32(&mut r, m + 0x44, (16 + 232 + ex.len()) as u32);
    put32(&mut r, m + 0x48, name.len() as u32);
    put32(&mut r, m + 0x4C, 9);
    put32(&mut r, m + 0x58, version);
    put32(&mut r, m + 0x5C, first_image);
    put32(&mut r, m + 0x70, 0x40);
    put32(&mut r, m + 0x98, spec.drm_offset.unwrap_or(0xFFFF_FFFF));
    if version >= 8 {
        // KF8 reuses the "first / last content record" field for the FDST record number
        put32(&mut r, 0xC0, fdst.unwrap_or(0xFFFF_FFFF));
    } else {
        put16(&mut r, m + 0xB0, 1);
        put16(&mut r, m + 0xB2, part.text_recs as u16);
    }
    put32(&mut r, m + 0xE0, spec.extra_flags);
    put32(&mut r, m + 0xE4, 0xFFFF_FFFF);
    r.extend(ex);
    r.extend(name);
    r.extend([0, 0]);
    r
}

fn assemble(db_name: &str, db_type: &[u8; 8], records: Vec<Vec<u8>>) -> Vec<u8> {
    let n = records.len();
    let mut out = vec![0u8; 78];
    let name = db_name.as_bytes();
    out[..name.len().min(31)].copy_from_slice(&name[..name.len().min(31)]);
    out[60..68].copy_from_slice(db_type);
    put32(&mut out, 68, 2 * n as u32 - 1);
    put16(&mut out, 76, n as u16);
    let mut offset = 78 + 8 * n + 2;
    for (i, r) in records.iter().enumerate() {
        out.extend((offset as u32).to_be_bytes());
        out.extend([0, 0, 0, i as u8]);
        offset += r.len();
    }
    out.extend([0, 0]);
    for r in records {
        out.extend(r);
    }
    out
}

pub fn write_mobi(spec: &MobiSpec) -> Vec<u8> {
    let mut html = encode(&spec.html, spec.encoding);
    if let Some(pos) = find(&html, b"<p>Third chapter") {
        if let Some(ph) = find(&html, b"XXXXXXXXXX") {
            html[ph..ph + 10].copy_from_slice(format!("{pos:010}").as_bytes());
        }
    }
    let mut records: Vec<Vec<u8>> = Vec::new();
    let kf8_only = spec.kf8.as_ref().is_some_and(|k| !k.joint);
    let mut first_image = 0u32;

    let mobi6_recs = text_records(&html, spec.compress, spec.extra_flags);
    let kf8_stream = spec.kf8.as_ref().map(|k| {
        let mut flow0 = Vec::new();
        for (skel, frags) in &k.files {
            flow0.extend(skel.as_bytes());
            for f in frags {
                flow0.extend(f.as_bytes());
            }
        }
        let flow0_len = flow0.len();
        flow0.extend(k.css.as_bytes());
        (flow0, flow0_len)
    });

    if !kf8_only {
        first_image = 1 + mobi6_recs.len() as u32;
    } else if let Some((s, _)) = &kf8_stream {
        first_image = 1 + text_records(s, spec.compress, spec.extra_flags).len() as u32;
    }

    // Record numbers of the KF8 half are only known after the MOBI 6 half has been laid out.
    let boundary_index = if spec.kf8.as_ref().is_some_and(|k| k.joint) {
        Some(1 + mobi6_recs.len() + spec.images.len() + 1)
    } else {
        None
    };

    if !kf8_only {
        let part = Part {
            text_len: html.len(),
            text_recs: mobi6_recs.len(),
        };
        records.push(record0(
            spec,
            &part,
            6,
            first_image,
            None,
            boundary_index.map(|b| b as u32),
        ));
        records.extend(mobi6_recs);
        records.extend(spec.images.iter().cloned());
    }
    if let (Some(k), Some((stream, flow0_len))) = (&spec.kf8, &kf8_stream) {
        if k.joint {
            records.push(b"BOUNDARY".to_vec());
        }
        let recs = text_records(stream, spec.compress, spec.extra_flags);
        let part = Part {
            text_len: stream.len(),
            text_recs: recs.len(),
        };
        let kf8_rec0_index = records.len();
        let fdst_index =
            kf8_rec0_index + 1 + recs.len() + if kf8_only { spec.images.len() } else { 0 };
        let fdst = (!spec.no_fdst).then_some(fdst_index as u32);
        records.push(record0(spec, &part, 8, first_image, fdst, None));
        records.extend(recs);
        if kf8_only {
            records.extend(spec.images.iter().cloned());
        }
        if !spec.no_fdst {
            let mut f = b"FDST".to_vec();
            f.extend(12u32.to_be_bytes());
            f.extend(2u32.to_be_bytes());
            f.extend(0u32.to_be_bytes());
            f.extend((*flow0_len as u32).to_be_bytes());
            f.extend((*flow0_len as u32).to_be_bytes());
            f.extend((stream.len() as u32).to_be_bytes());
            records.push(f);
        }
        if let Some(b) = boundary_index {
            assert_eq!(kf8_rec0_index, b, "writer layout");
        }
    }
    assemble(&spec.title.replace(' ', "_"), b"BOOKMOBI", records)
}

pub fn write_palmdoc(name: &str, text: &str) -> Vec<u8> {
    let recs = text_records(text.as_bytes(), true, 0);
    let mut r0 = vec![0u8; 16];
    put16(&mut r0, 0, 2);
    put32(&mut r0, 4, text.len() as u32);
    put16(&mut r0, 8, recs.len() as u16);
    put16(&mut r0, 10, 4096);
    let mut records = vec![r0];
    records.extend(recs);
    assemble(name, b"TEXtREAd", records)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
