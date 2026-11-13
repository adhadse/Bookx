//! Independent test-only *writers* for the container formats of the second reader wave (PDB /
//! eReader / zTXT, RB, SNB, TCR, CHM, LIT) and for hand assembled LZX streams. Each is written
//! from the public format description, byte by byte, and shares no code with the readers.
#![allow(dead_code)]

use std::io::Write;

use super::mobi_writer::palmdoc_compress;

// ---------------------------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------------------------

pub fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn cp1252(s: &str) -> Vec<u8> {
    encoding_rs::WINDOWS_1252.encode(s).0.into_owned()
}

// ---------------------------------------------------------------------------------------------
// LZX
// ---------------------------------------------------------------------------------------------

/// Bits are collected MSB first and emitted as 16 bit little endian words (LZX convention).
#[derive(Default)]
pub struct LzxBits {
    bits: Vec<bool>,
}

impl LzxBits {
    pub fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            self.bits.push((value >> i) & 1 == 1);
        }
    }
    pub fn align16(&mut self) {
        while self.bits.len() % 16 != 0 {
            self.bits.push(false);
        }
    }
    pub fn bytes(mut self) -> Vec<u8> {
        self.align16();
        self.bits
            .chunks(16)
            .flat_map(|w| {
                let v = w.iter().fold(0u16, |a, b| (a << 1) | *b as u16);
                v.to_le_bytes()
            })
            .collect()
    }
}

/// One frame holding a single uncompressed block (type 3) with the given bytes.
pub fn lzx_uncompressed_stream(data: &[u8]) -> Vec<u8> {
    let mut w = LzxBits::default();
    w.put(0, 1); // no E8 translation header
    w.put(3, 3); // block type: uncompressed
    w.put(data.len() as u32, 24);
    w.align16();
    let mut out = w.bytes();
    for _ in 0..3 {
        out.extend(1u32.to_le_bytes()); // R0, R1, R2
    }
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// One frame holding a verbatim block (type 1) of the symbols `a`(0) / `b`(1): the main tree
/// gives exactly those two literals the code length 1.
pub fn lzx_verbatim_ab_stream(symbols: &[u8]) -> Vec<u8> {
    let mut w = LzxBits::default();
    w.put(0, 1);
    w.put(1, 3); // verbatim
    w.put(symbols.len() as u32, 24);
    // Pretree: 20 four bit lengths, symbols 0 and 16 have length 1 (codes 0 and 1).
    let pretree = |w: &mut LzxBits| {
        for s in 0..20 {
            w.put(if s == 0 || s == 16 { 1 } else { 0 }, 4);
        }
    };
    // Main tree, first 256 elements: delta symbol 16 turns length 0 into 1, symbol 0 keeps 0.
    pretree(&mut w);
    for e in 0..256 {
        w.put(
            if e == b'a' as u32 || e == b'b' as u32 {
                1
            } else {
                0
            },
            1,
        );
    }
    // Main tree, match elements: 8 * 30 position slots for a 32 KiB window, all zero.
    pretree(&mut w);
    for _ in 0..240 {
        w.put(0, 1);
    }
    // Length tree: 249 zero elements.
    pretree(&mut w);
    for _ in 0..249 {
        w.put(0, 1);
    }
    for s in symbols {
        w.put(*s as u32, 1); // 'a' = code 0, 'b' = code 1
    }
    w.bytes()
}

/// Real compressed frames from the independent `lzxc` encoder (window 64 KiB, one stream).
pub fn lzxc_encode(data: &[u8]) -> Vec<Vec<u8>> {
    lzxc::Encoder::new(lzxc::WindowSize::KB64).encode_stream(data)
}

// ---------------------------------------------------------------------------------------------
// PalmDB, eReader, zTXT
// ---------------------------------------------------------------------------------------------

/// Assembles a Palm database from raw records.
pub fn pdb_assemble(
    name: &str,
    db_type: &[u8; 4],
    creator: &[u8; 4],
    records: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = vec![0u8; 78];
    let n = name.as_bytes();
    out[..n.len().min(31)].copy_from_slice(&n[..n.len().min(31)]);
    out[60..64].copy_from_slice(db_type);
    out[64..68].copy_from_slice(creator);
    out[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
    let mut offset = 78 + records.len() * 8 + 2;
    for (i, r) in records.iter().enumerate() {
        out.extend((offset as u32).to_be_bytes());
        out.push(0);
        out.extend([0, 0, i as u8]);
        offset += r.len();
    }
    out.extend([0, 0]);
    for r in records {
        out.extend_from_slice(r);
    }
    out
}

#[derive(Default, Clone)]
pub struct EReaderSpec {
    /// Header version: 2 = PalmDOC, 10 = zlib; other values only change the header.
    pub version: u16,
    pub title: String,
    pub author: String,
    pub text: String,
    pub images: Vec<(String, Vec<u8>)>,
    pub footnotes: Vec<(String, String)>,
    pub sidebars: Vec<(String, String)>,
    /// Uncompressed bytes per text record (0 = 8192).
    pub record_size: usize,
    /// Footnote ids as `00 01 len id 00` instead of NUL separated.
    pub new_style_ids: bool,
    pub no_metadata: bool,
}

fn ereader_compress(version: u16, data: &[u8]) -> Vec<u8> {
    if version == 10 {
        zlib_compress(data)
    } else {
        palmdoc_compress(data)
    }
}

fn note_records(version: u16, notes: &[(String, String)], new_style: bool) -> Vec<Vec<u8>> {
    if notes.is_empty() {
        return vec![];
    }
    let mut ids = Vec::new();
    for (id, _) in notes {
        if new_style {
            ids.extend([0, 1, id.len() as u8]);
            ids.extend(id.as_bytes());
            ids.push(0);
        } else {
            ids.extend(id.as_bytes());
            ids.push(0);
        }
    }
    let mut recs = vec![ids];
    for (_, text) in notes {
        recs.push(ereader_compress(version, &cp1252(text)));
    }
    recs
}

pub fn write_ereader(spec: &EReaderSpec) -> Vec<u8> {
    let rs = if spec.record_size == 0 {
        8192
    } else {
        spec.record_size
    };
    let text = cp1252(&spec.text);
    let mut records: Vec<Vec<u8>> = vec![vec![]];
    for chunk in text.chunks(rs) {
        records.push(ereader_compress(spec.version, chunk));
    }
    let non_text = records.len();
    let mut h = vec![0u8; 132];
    let put = |h: &mut Vec<u8>, off: usize, v: usize| {
        h[off..off + 2].copy_from_slice(&(v as u16).to_be_bytes())
    };
    put(&mut h, 0, spec.version as usize);
    put(&mut h, 6, 25152);
    put(&mut h, 12, non_text);

    let image_start = records.len();
    for (name, data) in &spec.images {
        let mut r = b"PNG ".to_vec();
        let mut nm = name.as_bytes().to_vec();
        nm.resize(32, 0);
        r.extend(nm);
        r.extend([0u8; 22]);
        r.extend([0, 1, 0, 1]);
        r.extend(data);
        records.push(r);
    }
    put(&mut h, 20, spec.images.len());
    let meta_start = records.len();
    if !spec.no_metadata {
        let mut m = Vec::new();
        for f in [
            spec.title.as_str(),
            &spec.author,
            "(c) test",
            "Pub Co",
            "1234567890",
        ] {
            m.extend(cp1252(f));
            m.push(0);
        }
        records.push(m);
        put(&mut h, 24, 1);
    }
    let foot_start = records.len();
    records.extend(note_records(
        spec.version,
        &spec.footnotes,
        spec.new_style_ids,
    ));
    let side_start = records.len();
    records.extend(note_records(
        spec.version,
        &spec.sidebars,
        spec.new_style_ids,
    ));
    records.push(b"MeTaInFo\0".to_vec());
    let last = records.len() - 1;
    put(&mut h, 28, spec.footnotes.len());
    put(&mut h, 30, spec.sidebars.len());
    put(&mut h, 34, 2560);
    put(
        &mut h,
        40,
        if spec.images.is_empty() {
            last
        } else {
            image_start
        },
    );
    put(&mut h, 44, if spec.no_metadata { last } else { meta_start });
    put(
        &mut h,
        48,
        if spec.footnotes.is_empty() {
            last
        } else {
            foot_start
        },
    );
    put(
        &mut h,
        50,
        if spec.sidebars.is_empty() {
            last
        } else {
            side_start
        },
    );
    put(&mut h, 52, last);
    records[0] = h;
    pdb_assemble("ereader_book", b"PNRd", b"PPrs", &records)
}

/// zTXT: one zlib stream over all text records (or just the random-access flag set).
pub fn write_ztxt(name: &str, text: &str, random_access: bool) -> Vec<u8> {
    let z = zlib_compress(text.as_bytes());
    let chunks: Vec<Vec<u8>> = z.chunks(64).map(|c| c.to_vec()).collect();
    let mut h = vec![0u8; 24];
    h[0..2].copy_from_slice(&0x0128u16.to_be_bytes());
    h[2..4].copy_from_slice(&(chunks.len() as u16).to_be_bytes());
    h[4..8].copy_from_slice(&(text.len() as u32).to_be_bytes());
    h[8..10].copy_from_slice(&8192u16.to_be_bytes());
    h[18] = random_access as u8;
    let mut records = vec![h];
    records.extend(chunks);
    pdb_assemble(name, b"zTXT", b"GPlm", &records)
}

// ---------------------------------------------------------------------------------------------
// RB (Rocket eBook)
// ---------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct RbSpec {
    pub info: String,
    /// `(name, data, deflated)`
    pub pages: Vec<(String, Vec<u8>, bool)>,
    pub no_info_page: bool,
}

fn rb_deflate(data: &[u8]) -> Vec<u8> {
    let chunks: Vec<Vec<u8>> = data.chunks(4096).map(zlib_compress).collect();
    let mut out = Vec::new();
    out.extend((chunks.len() as u32).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    for c in &chunks {
        out.extend((c.len() as u32).to_le_bytes());
    }
    for c in &chunks {
        out.extend(c);
    }
    out
}

pub fn write_rb(spec: &RbSpec) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>, u32)> = Vec::new();
    if !spec.no_info_page {
        entries.push(("000000.info".into(), spec.info.as_bytes().to_vec(), 2));
    }
    for (n, d, z) in &spec.pages {
        if *z {
            entries.push((n.clone(), rb_deflate(d), 8));
        } else {
            entries.push((n.clone(), d.clone(), 0));
        }
    }
    let toc_off = 0x128usize;
    let mut data_off = toc_off + 4 + entries.len() * 44;
    let mut out = vec![0u8; toc_off];
    out[..4].copy_from_slice(&[0xB0, 0x0C, 0xB0, 0x0C]);
    out[4..6].copy_from_slice(&[2, 0]);
    out[6..10].copy_from_slice(b"NUVO");
    out[0x0E..0x10].copy_from_slice(&100u16.to_le_bytes());
    out[0x10] = 5;
    out[0x11] = 17;
    out[0x18..0x1C].copy_from_slice(&(toc_off as u32).to_le_bytes());
    out.extend((entries.len() as u32).to_le_bytes());
    for (n, d, f) in &entries {
        let mut name = n.as_bytes().to_vec();
        name.resize(32, 0);
        out.extend(name);
        out.extend((d.len() as u32).to_le_bytes());
        out.extend((data_off as u32).to_le_bytes());
        out.extend(f.to_le_bytes());
        data_off += d.len();
    }
    for (_, d, _) in &entries {
        out.extend(d);
    }
    out.extend([1u8; 20]);
    let len = out.len() as u32;
    out[0x1C..0x20].copy_from_slice(&len.to_le_bytes());
    out
}

// ---------------------------------------------------------------------------------------------
// SNB
// ---------------------------------------------------------------------------------------------

fn bz2_compress(data: &[u8]) -> Vec<u8> {
    let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// The text `hello hello hello\n` compressed by the `bzip2 -9` command line tool, and the text.
pub fn bzip2_cli_fixture() -> (Vec<u8>, Vec<u8>) {
    (
        vec![
            0x42, 0x5a, 0x68, 0x39, 0x31, 0x41, 0x59, 0x26, 0x53, 0x59, 0xe5, 0xb5, 0xf3, 0x09,
            0x00, 0x00, 0x04, 0x51, 0x00, 0x00, 0x10, 0x40, 0x00, 0x02, 0x44, 0xa0, 0x00, 0x21,
            0xb5, 0x18, 0x0c, 0x02, 0x90, 0x69, 0xc2, 0xa3, 0x0b, 0xb9, 0x22, 0x9c, 0x28, 0x48,
            0x72, 0xda, 0xf9, 0x84, 0x80,
        ],
        b"hello hello hello\n".to_vec(),
    )
}

/// `(name, data, plain)`: plain files (XML) go to the bzip2 stream, the others are stored.
pub fn write_snb(files: &[(&str, Vec<u8>, bool)]) -> Vec<u8> {
    let mut vfat = Vec::new();
    let mut names = Vec::new();
    let mut bin = Vec::new();
    let mut plain = Vec::new();
    for (name, data, is_plain) in files {
        let attr: u32 = if *is_plain { 0x4100_0000 } else { 0x0100_0000 };
        vfat.extend(attr.to_be_bytes());
        vfat.extend((names.len() as u32).to_be_bytes());
        vfat.extend((data.len() as u32).to_be_bytes());
        names.extend(name.as_bytes());
        names.push(0);
        if *is_plain {
            plain.extend(data);
        } else {
            bin.extend(data);
        }
    }
    vfat.extend(&names);
    let vfat_c = zlib_compress(&vfat);

    let mut out = Vec::new();
    out.extend(b"SNBP000B");
    for v in [
        0x8000u32,
        0x00A3_A3A3,
        0,
        files.len() as u32,
        vfat.len() as u32,
        vfat_c.len() as u32,
        bin.len() as u32,
        plain.len() as u32,
        0,
    ] {
        out.extend(v.to_be_bytes());
    }
    out.extend(&vfat_c);
    let bin_start = out.len();
    out.extend(&bin);
    let mut tail = Vec::new();
    for i in 0..bin.len().div_ceil(0x8000) {
        tail.extend(((bin_start + i * 0x8000) as u32).to_be_bytes());
    }
    for block in plain.chunks(0x8000) {
        tail.extend((out.len() as u32).to_be_bytes());
        out.extend(bz2_compress(block));
    }
    let bin_blocks = bin.len().div_ceil(0x8000);
    let (mut bpos, mut ppos) = (0usize, 0usize);
    for (_, data, is_plain) in files {
        if *is_plain {
            tail.extend(((bin_blocks + ppos / 0x8000) as u32).to_be_bytes());
            tail.extend(((ppos % 0x8000) as u32).to_be_bytes());
            ppos += data.len();
        } else {
            tail.extend(((bpos / 0x8000) as u32).to_be_bytes());
            tail.extend(((bpos % 0x8000) as u32).to_be_bytes());
            bpos += data.len();
        }
    }
    let tail_c = zlib_compress(&tail);
    let tail_off = out.len();
    out.extend(&tail_c);
    out.extend((tail_c.len() as u32).to_be_bytes());
    out.extend((tail_off as u32).to_be_bytes());
    out.extend(b"SNBP000B");
    out
}

// ---------------------------------------------------------------------------------------------
// TCR
// ---------------------------------------------------------------------------------------------

/// A TCR file from explicit dictionary entries `(index, bytes)`; unlisted indices map to the
/// single byte equal to the index (except they are never used when the payload avoids them).
pub fn tcr_file(entries: &[(u8, &[u8])], payload: &[u8]) -> Vec<u8> {
    let mut out = b"!!8-Bit!!".to_vec();
    for i in 0..=255u8 {
        match entries.iter().find(|(k, _)| *k == i) {
            Some((_, e)) => {
                out.push(e.len() as u8);
                out.extend(*e);
            }
            None => {
                out.push(1);
                out.push(i);
            }
        }
    }
    out.extend(payload);
    out
}

/// A simple TCR compressor: the 255 most useful byte pairs become dictionary entries.
pub fn tcr_compress(text: &[u8]) -> Vec<u8> {
    use std::collections::HashMap;
    // Entry i (0..=255) is a single byte unless replaced by a frequent pair.
    let mut pairs: HashMap<(u8, u8), usize> = HashMap::new();
    for w in text.windows(2) {
        *pairs.entry((w[0], w[1])).or_default() += 1;
    }
    let mut ranked: Vec<_> = pairs.into_iter().filter(|(_, n)| *n > 1).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    // Free slots: byte values that do not occur in the text.
    let used: std::collections::HashSet<u8> = text.iter().copied().collect();
    let free: Vec<u8> = (0..=255u8).filter(|b| !used.contains(b)).collect();
    let mut dict: HashMap<(u8, u8), u8> = HashMap::new();
    for ((pair, _), slot) in ranked.into_iter().zip(free.iter()) {
        dict.insert(pair, *slot);
    }
    let mut payload = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if i + 1 < text.len() {
            if let Some(s) = dict.get(&(text[i], text[i + 1])) {
                payload.push(*s);
                i += 2;
                continue;
            }
        }
        payload.push(text[i]);
        i += 1;
    }
    let entries: Vec<(u8, Vec<u8>)> = dict.iter().map(|((a, b), s)| (*s, vec![*a, *b])).collect();
    let refs: Vec<(u8, &[u8])> = entries.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    tcr_file(&refs, &payload)
}

// ---------------------------------------------------------------------------------------------
// CHM
// ---------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct ChmSpec {
    pub title: String,
    pub toc_file: Option<String>,
    pub default_topic: Option<String>,
    /// `(absolute path such as /a/b.htm, data)`
    pub files: Vec<(String, Vec<u8>)>,
    /// Put the files in the LZX compressed section 1.
    pub compressed: bool,
    /// LZX reset interval in frames (0 = 1).
    pub reset_frames: usize,
    /// ITSF version 2 header (no explicit content offset).
    pub version2: bool,
}

fn encint_bytes(mut v: u64) -> Vec<u8> {
    let mut groups = vec![(v & 0x7F) as u8];
    v >>= 7;
    while v > 0 {
        groups.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    groups.reverse();
    groups
}

fn chm_entry(path: &str, section: u64, offset: u64, length: u64) -> Vec<u8> {
    let mut e = encint_bytes(path.len() as u64);
    e.extend(path.as_bytes());
    e.extend(encint_bytes(section));
    e.extend(encint_bytes(offset));
    e.extend(encint_bytes(length));
    e
}

pub fn write_chm(spec: &ChmSpec) -> Vec<u8> {
    let reset_frames = spec.reset_frames.max(1);
    let mut system = 3u32.to_le_bytes().to_vec();
    for (code, s) in [
        (0u16, spec.toc_file.clone()),
        (2, spec.default_topic.clone()),
        (
            3,
            if spec.title.is_empty() {
                None
            } else {
                Some(spec.title.clone())
            },
        ),
    ] {
        if let Some(s) = s {
            system.extend(code.to_le_bytes());
            system.extend(((s.len() + 1) as u16).to_le_bytes());
            system.extend(s.as_bytes());
            system.push(0);
        }
    }

    // (path, section, offset, length) and the section 0 data region.
    let mut dir: Vec<(String, u64, u64, u64)> = Vec::new();
    let mut sec0: Vec<u8> = Vec::new();
    let mut add0 = |path: &str, data: &[u8], dir: &mut Vec<(String, u64, u64, u64)>| {
        dir.push((path.to_string(), 0, sec0.len() as u64, data.len() as u64));
        sec0.extend_from_slice(data);
    };
    add0("/#SYSTEM", &system, &mut dir);
    if spec.compressed {
        let mut stream = Vec::new();
        for (p, d) in &spec.files {
            dir.push((p.clone(), 1, stream.len() as u64, d.len() as u64));
            stream.extend(d);
        }
        // Real compressors always fill the last frame: pad with zeros to a whole frame (chmlib
        // and 7-Zip decode full frames). The reset table still records the true length.
        let mut padded = stream.clone();
        padded.resize(stream.len().div_ceil(0x8000) * 0x8000, 0);
        // Encode each reset interval with a fresh encoder so the decoder can reset there.
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for group in padded.chunks(reset_frames * 0x8000) {
            frames.extend(lzxc::Encoder::new(lzxc::WindowSize::KB64).encode_stream(group));
        }
        let mut comp = Vec::new();
        let mut offsets = Vec::new();
        for f in &frames {
            offsets.push(comp.len() as u64);
            comp.extend(f);
        }
        let mut control = Vec::new();
        control.extend(6u32.to_le_bytes());
        control.extend(b"LZXC");
        for v in [2u32, reset_frames as u32, 2, 1, 0] {
            control.extend(v.to_le_bytes());
        }
        let mut reset = Vec::new();
        for v in [2u32, frames.len() as u32, 8, 0x28] {
            reset.extend(v.to_le_bytes());
        }
        for v in [stream.len() as u64, comp.len() as u64, 0x8000u64] {
            reset.extend(v.to_le_bytes());
        }
        for o in &offsets {
            reset.extend(o.to_le_bytes());
        }
        let utf16 =
            |t: &str| -> Vec<u8> { t.encode_utf16().flat_map(|c| c.to_le_bytes()).collect() };
        let mut name_list = Vec::new();
        for n in ["Uncompressed", "MSCompressed"] {
            name_list.extend((n.len() as u16).to_le_bytes());
            name_list.extend(utf16(n));
            name_list.extend(0u16.to_le_bytes());
        }
        let mut nl = ((name_list.len() / 2 + 2) as u16).to_le_bytes().to_vec();
        nl.extend(2u16.to_le_bytes());
        nl.extend(name_list);
        add0("::DataSpace/NameList", &nl, &mut dir);
        let mut transforms = utf16("{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}");
        transforms.extend([0, 0]);
        add0(
            "::DataSpace/Storage/MSCompressed/Transform/List",
            &transforms,
            &mut dir,
        );
        add0("::DataSpace/Storage/MSCompressed/Content", &comp, &mut dir);
        add0(
            "::DataSpace/Storage/MSCompressed/ControlData",
            &control,
            &mut dir,
        );
        add0(
            "::DataSpace/Storage/MSCompressed/Transform/{7FC28940-9D31-11D0-9B27-00A0C91E9C7C}/InstanceData/ResetTable",
            &reset,
            &mut dir,
        );
    } else {
        for (p, d) in &spec.files {
            add0(p, d, &mut dir);
        }
    }
    dir.sort_by_key(|e| e.0.to_ascii_lowercase());

    // Directory chunks of 1 KiB: PMGL listing chunks, then one (ignored) PMGI index chunk.
    const CHUNK: usize = 0x400;
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    for (p, s, o, l) in &dir {
        let e = chm_entry(p, *s, *o, *l);
        if 20 + cur.len() + e.len() > CHUNK {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.extend(e);
    }
    chunks.push(cur);
    let n_pmgl = chunks.len();
    let mut dir_data = Vec::new();
    for (i, body) in chunks.iter().enumerate() {
        let mut c = b"PMGL".to_vec();
        c.extend(((CHUNK - 20 - body.len()) as u32).to_le_bytes());
        c.extend(0u32.to_le_bytes());
        c.extend((if i == 0 { -1i32 } else { i as i32 - 1 }).to_le_bytes());
        c.extend((if i + 1 == n_pmgl { -1i32 } else { i as i32 + 1 }).to_le_bytes());
        c.extend(body);
        c.resize(CHUNK, 0);
        dir_data.extend(c);
    }
    let mut pmgi = b"PMGI".to_vec();
    pmgi.extend(((CHUNK - 8) as u32).to_le_bytes());
    pmgi.resize(CHUNK, 0);
    dir_data.extend(pmgi);
    let n_chunks = n_pmgl + 1;

    let mut itsp = b"ITSP".to_vec();
    for v in [
        1u32,
        0x54,
        10,
        CHUNK as u32,
        2,
        1,
        0xFFFF_FFFF,
        0,
        (n_pmgl - 1) as u32,
        0xFFFF_FFFF,
        n_chunks as u32,
        0x409,
    ] {
        itsp.extend(v.to_le_bytes());
    }
    itsp.extend([0u8; 16]);
    itsp.extend(0x54u32.to_le_bytes());
    for _ in 0..3 {
        itsp.extend(0xFFFF_FFFFu32.to_le_bytes());
    }
    assert_eq!(itsp.len(), 0x54);
    let mut directory = itsp;
    directory.extend(dir_data);

    let header_len = if spec.version2 { 0x58 } else { 0x60 };
    let sec0_off = header_len;
    let dir_off = sec0_off + 0x18;
    let content_off = dir_off + directory.len();
    let mut out = b"ITSF".to_vec();
    out.extend((if spec.version2 { 2u32 } else { 3 }).to_le_bytes());
    out.extend((header_len as u32).to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0x409u32.to_le_bytes());
    out.extend([0u8; 32]);
    assert_eq!(out.len(), 0x38);
    for v in [
        sec0_off as u64,
        0x18,
        dir_off as u64,
        directory.len() as u64,
    ] {
        out.extend(v.to_le_bytes());
    }
    if !spec.version2 {
        out.extend((content_off as u64).to_le_bytes());
    }
    assert_eq!(out.len(), header_len);
    out.extend(0x1FEu32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(directory);
    out.extend(sec0);
    out
}

// ---------------------------------------------------------------------------------------------
// LIT
// ---------------------------------------------------------------------------------------------

/// A LIT container skeleton: primary header, a 5 entry piece table and some body bytes. With
/// `drm` the body names the DRM storage.
pub fn write_lit(drm: bool) -> Vec<u8> {
    let mut out = b"ITOLITLS".to_vec();
    for v in [1u32, 40, 5, 0x58] {
        out.extend(v.to_le_bytes());
    }
    out.extend([0xAAu8; 16]);
    assert_eq!(out.len(), 40);
    let body_start = 40 + 5 * 16;
    for i in 0..5u64 {
        out.extend((body_start as u64 + i * 20).to_le_bytes());
        out.extend(20u64.to_le_bytes());
    }
    let mut body = vec![0u8; 100];
    if drm {
        let name = b"/DRMStorage/DRMSealed";
        body[..name.len()].copy_from_slice(name);
    } else {
        let name = b"/manifest";
        body[..name.len()].copy_from_slice(name);
    }
    out.extend(body);
    out
}
