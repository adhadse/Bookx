//! PDB (Palm Database) books.
//!
//! The container is read here; the content is dispatched on the database type and creator at
//! offset 60:
//!
//! * `PNRdPPrs` (eReader / Peanut Press): header record version 2 (PalmDOC compression) and 10
//!   (zlib); text, embedded PNG images, the metadata record, footnotes and sidebars. The PML
//!   markup is rendered by [`crate::pml`]. Versions 260 and 272 are DRM protected and rejected.
//!   Skipped: chapter/link/page index records (chapters are rebuilt from the markup), the
//!   202 byte Makebook header variant is read with the same field offsets.
//! * `TEXtREAd` (PalmDOC) and `BOOKMOBI`: handed to [`crate::mobi`].
//! * `zTXTGPlm` (zTXT): non random-access files (one zlib stream over all text records).
//!   Random-access zTXT and bookmarks are not supported.
//!
//! Every other type or creator gives a clear error naming it.

use std::collections::HashMap;
use std::io::Read;

use anyhow::{Context, Result, bail};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::opf::{Identifier, Metadata};
use flate2::read::ZlibDecoder;

use crate::common::*;
use crate::mobi::{convert_mobi, palmdoc_decompress};
use crate::pml::{ImageStore, pml_to_book};
use crate::textfmt::convert_plain;

/// Largest decompressed size of one eReader text record.
const MAX_RECORD_OUT: usize = 1 << 20;
/// Largest total text.
const MAX_TEXT: usize = 128 << 20;

/// Reader for `.pdb` files (eReader, zTXT, PalmDOC, MOBI).
pub struct PdbReader;

impl FormatReader for PdbReader {
    fn id(&self) -> &'static str {
        "pdb"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["pdb"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        matches!(
            type_creator(head),
            Some((b"PNRd", b"PPrs")) | Some((b"zTXT", b"GPlm"))
        )
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_pdb(bytes)
    }
}

/// The type and creator codes at offset 60, if the head is long enough.
pub fn type_creator(head: &[u8]) -> Option<(&[u8; 4], &[u8; 4])> {
    let t: &[u8; 4] = head.get(60..64)?.try_into().ok()?;
    let c: &[u8; 4] = head.get(64..68)?.try_into().ok()?;
    Some((t, c))
}

/// A parsed Palm database: name, type, creator and the record boundaries.
pub struct PalmDb<'a> {
    data: &'a [u8],
    pub name: String,
    pub db_type: [u8; 4],
    pub creator: [u8; 4],
    offsets: Vec<usize>,
}

fn be16(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
}

fn be32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

impl<'a> PalmDb<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 78 {
            bail!("file is too small to be a Palm database");
        }
        let n = be16(data, 76).unwrap_or(0) as usize;
        if n == 0 {
            bail!("the Palm database has no records");
        }
        let mut offsets = Vec::with_capacity(n);
        for i in 0..n {
            let off = be32(data, 78 + i * 8).context("the record list is truncated")? as usize;
            offsets.push(off);
        }
        if offsets.iter().any(|o| *o > data.len()) {
            bail!("a record points beyond the end of the file (truncated file)");
        }
        if offsets.windows(2).any(|w| w[0] > w[1]) {
            bail!("the record offsets are not ascending");
        }
        let raw_name = &data[..32];
        let end = raw_name.iter().position(|b| *b == 0).unwrap_or(32);
        let name = decode_text(&raw_name[..end]).trim().to_string();
        let mut db_type = [0u8; 4];
        db_type.copy_from_slice(&data[60..64]);
        let mut creator = [0u8; 4];
        creator.copy_from_slice(&data[64..68]);
        Ok(PalmDb {
            data,
            name,
            db_type,
            creator,
            offsets,
        })
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn record(&self, i: usize) -> Result<&'a [u8]> {
        let start = *self
            .offsets
            .get(i)
            .with_context(|| format!("record {i} does not exist"))?;
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.data.len());
        Ok(&self.data[start..end])
    }
}

/// Converts any supported PDB file.
pub fn convert_pdb(bytes: &[u8]) -> Result<Archive> {
    let db = PalmDb::parse(bytes)?;
    match (&db.db_type, &db.creator) {
        (b"PNRd", b"PPrs") => convert_ereader(&db),
        (b"zTXT", b"GPlm") => convert_ztxt(&db),
        (b"TEXt", b"REAd") | (b"BOOK", b"MOBI") => convert_mobi(bytes),
        (t, c) => bail!(
            "unsupported Palm database type `{}` / creator `{}`",
            String::from_utf8_lossy(t),
            String::from_utf8_lossy(c)
        ),
    }
}

fn inflate_zlib(data: &[u8], max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .context("corrupt zlib data")?;
    if out.len() > max {
        bail!("zlib data expands to more than {max} bytes");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// eReader
// ---------------------------------------------------------------------------

struct EReaderHeader {
    version: u16,
    non_text: usize,
    images: usize,
    image_start: usize,
    has_meta: bool,
    meta_start: usize,
    footnotes: usize,
    footnote_start: usize,
    sidebars: usize,
    sidebar_start: usize,
}

fn parse_ereader_header(rec0: &[u8]) -> Result<EReaderHeader> {
    if rec0.len() < 54 {
        bail!(
            "the eReader header record is too short ({} bytes)",
            rec0.len()
        );
    }
    let g = |o: usize| be16(rec0, o).unwrap_or(0) as usize;
    Ok(EReaderHeader {
        version: g(0) as u16,
        non_text: g(12),
        images: g(20),
        image_start: g(40),
        has_meta: g(24) != 0,
        meta_start: g(44),
        footnotes: g(28),
        footnote_start: g(48),
        sidebars: g(30),
        sidebar_start: g(50),
    })
}

fn ereader_record_text(version: u16, rec: &[u8]) -> Result<Vec<u8>> {
    match version {
        2 => palmdoc_decompress(rec, MAX_RECORD_OUT),
        10 => inflate_zlib(rec, MAX_RECORD_OUT),
        v => bail!("unsupported eReader compression version {v}"),
    }
}

fn cp1252(b: &[u8]) -> String {
    encoding_rs::WINDOWS_1252
        .decode_without_bom_handling(b)
        .0
        .into_owned()
}

/// The ids of the first footnote / sidebar record: either `\0`-separated, or (newer files) a
/// sequence of `00 01 <len> <id> 00`.
fn parse_note_ids(rec: &[u8]) -> Vec<String> {
    let mut ids = Vec::new();
    if rec.starts_with(&[0, 1]) {
        let mut i = 0;
        while i + 3 <= rec.len() && rec[i] == 0 && rec[i + 1] == 1 {
            let len = rec[i + 2] as usize;
            let Some(id) = rec.get(i + 3..i + 3 + len) else {
                break;
            };
            ids.push(cp1252(id));
            i += 3 + len + 1;
        }
        return ids;
    }
    for part in rec.split(|b| *b == 0) {
        if !part.is_empty() {
            ids.push(cp1252(part));
        }
    }
    ids
}

fn read_notes(
    db: &PalmDb,
    version: u16,
    start: usize,
    count: usize,
) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    if count == 0 || start == 0 || start >= db.len() {
        return Ok(map);
    }
    let ids = parse_note_ids(db.record(start)?);
    for (k, id) in ids.iter().enumerate().take(count) {
        let Ok(rec) = db.record(start + 1 + k) else {
            break;
        };
        let text = ereader_record_text(version, rec)?;
        map.insert(id.clone(), cp1252(&text));
    }
    Ok(map)
}

fn convert_ereader(db: &PalmDb) -> Result<Archive> {
    let h = parse_ereader_header(db.record(0)?)?;
    if matches!(h.version, 260 | 272) {
        bail!(
            "this eReader book is DRM protected (compression version {}) and cannot be read",
            h.version
        );
    }
    if !matches!(h.version, 2 | 10) {
        bail!("unsupported eReader compression version {}", h.version);
    }
    if h.non_text < 2 || h.non_text > db.len() {
        bail!(
            "the eReader header lists {} as the first non-text record, the file has {} records",
            h.non_text,
            db.len()
        );
    }
    let mut text = Vec::new();
    for i in 1..h.non_text {
        let rec = db.record(i)?;
        let part =
            ereader_record_text(h.version, rec).with_context(|| format!("text record {i}"))?;
        text.extend_from_slice(&part);
        if text.len() > MAX_TEXT {
            bail!("the eReader text is larger than {MAX_TEXT} bytes");
        }
    }
    let pml = cp1252(&text);

    let mut images = ImageStore::default();
    if h.images > 0 && h.image_start > 0 {
        for i in h.image_start..(h.image_start + h.images).min(db.len()) {
            let rec = db.record(i)?;
            if rec.len() < 62 || !rec.starts_with(b"PNG ") {
                continue;
            }
            let raw_name = &rec[4..36];
            let end = raw_name.iter().position(|b| *b == 0).unwrap_or(32);
            let name = cp1252(&raw_name[..end]);
            let data = &rec[62..];
            if image_ext(data).is_some() && !name.is_empty() {
                images.insert(&name, data.to_vec());
            }
        }
    }

    let mut md = Metadata::default();
    if h.has_meta && h.meta_start < db.len() {
        let rec = db.record(h.meta_start)?;
        let fields: Vec<String> = rec
            .split(|b| *b == 0)
            .map(|f| cp1252(f).trim().to_string())
            .collect();
        let f = |i: usize| fields.get(i).cloned().unwrap_or_default();
        md.title = f(0);
        if !f(1).is_empty() {
            md.authors = vec![f(1)];
        }
        md.rights = f(2);
        md.publisher = f(3);
        if !f(4).is_empty() {
            md.identifiers.push(Identifier {
                scheme: "isbn".into(),
                value: f(4),
                xml_id: None,
            });
        }
    }
    if md.title.is_empty() {
        md.title = db.name.clone();
    }

    let footnotes = read_notes(db, h.version, h.footnote_start, h.footnotes)?;
    let sidebars = read_notes(db, h.version, h.sidebar_start, h.sidebars)?;
    let book = pml_to_book(&pml, &images, footnotes, sidebars, md);
    build(Parts {
        md: book.md,
        chapters: book.chapters,
        cover: book.cover,
        media: book.media,
        css: crate::pml::CSS.into(),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// zTXT
// ---------------------------------------------------------------------------

fn convert_ztxt(db: &PalmDb) -> Result<Archive> {
    let rec0 = db.record(0)?;
    if rec0.len() < 20 {
        bail!("the zTXT header record is too short");
    }
    let n = be16(rec0, 2).unwrap_or(0) as usize;
    let size = be32(rec0, 4).unwrap_or(0) as usize;
    let flags = rec0[18];
    if flags & 1 != 0 {
        bail!("zTXT files in random-access mode are not supported");
    }
    if n == 0 || n >= db.len() {
        bail!(
            "the zTXT header lists {n} text records, the file has {} records",
            db.len()
        );
    }
    if size > MAX_TEXT {
        bail!("the zTXT text is larger than {MAX_TEXT} bytes");
    }
    let mut stream = Vec::new();
    for i in 1..=n {
        stream.extend_from_slice(db.record(i)?);
    }
    let text = inflate_zlib(&stream, MAX_TEXT)?;
    if text.iter().all(|b| b.is_ascii_whitespace()) {
        bail!("the zTXT file contains no text");
    }
    convert_plain("txt", &text, &db.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fmt_writers::*;
    use crate::testutil::mobi_writer::*;
    use crate::testutil::*;

    #[test]
    fn palmdb_parse_fields_and_records() {
        let f = pdb_assemble(
            "My Name",
            b"TEXt",
            b"REAd",
            &[b"abc".to_vec(), b"de".to_vec(), vec![]],
        );
        let db = PalmDb::parse(&f).unwrap();
        assert_eq!(db.name, "My Name");
        assert_eq!(&db.db_type, b"TEXt");
        assert_eq!(&db.creator, b"REAd");
        assert_eq!(db.len(), 3);
        assert!(!db.is_empty());
        assert_eq!(db.record(0).unwrap(), b"abc");
        assert_eq!(db.record(1).unwrap(), b"de");
        assert_eq!(db.record(2).unwrap(), b"");
        assert!(db.record(3).is_err());
        assert_eq!(type_creator(&f), Some((b"TEXt", b"REAd")));
        assert_eq!(type_creator(&f[..66]), None);
    }

    #[test]
    fn palmdb_rejects_bad_containers() {
        assert!(format!("{}", PalmDb::parse(b"short").err().unwrap()).contains("too small"));
        let mut f = pdb_assemble("n", b"TEXt", b"REAd", &[b"x".to_vec()]);
        f[76] = 0;
        f[77] = 0;
        assert!(format!("{}", PalmDb::parse(&f).err().unwrap()).contains("no records"));
        let mut f = pdb_assemble("n", b"TEXt", b"REAd", &[b"x".to_vec()]);
        f[76] = 0;
        f[77] = 200;
        assert!(PalmDb::parse(&f).is_err(), "record list beyond the file");
        let mut f = pdb_assemble("n", b"TEXt", b"REAd", &[b"x".to_vec(), b"y".to_vec()]);
        f[78..82].copy_from_slice(&0xFFFF_0000u32.to_be_bytes());
        assert!(format!("{}", PalmDb::parse(&f).err().unwrap()).contains("beyond the end"));
        let mut f = pdb_assemble("n", b"TEXt", b"REAd", &[b"x".to_vec(), b"y".to_vec()]);
        let (a, b) = (be32(&f, 78).unwrap(), be32(&f, 86).unwrap());
        f[78..82].copy_from_slice(&b.to_be_bytes());
        f[86..90].copy_from_slice(&a.to_be_bytes());
        assert!(format!("{}", PalmDb::parse(&f).err().unwrap()).contains("not ascending"));
    }

    fn sample_ereader(version: u16) -> Vec<u8> {
        write_ereader(&EReaderSpec {
            version,
            title: "Peanut Book".into(),
            author: "Pea Nut".into(),
            text: "\\x\\cChapter One\\c\\x\nHello \\iworld\\i, caf\\a233 \\m=\"pic.png\"\nSee\\Fn=\"n1\"1\\Fn and \\Sd=\"s1\"box\\Sd.\n\\xChapter Two\\x\nBye.".into(),
            images: vec![("pic.png".into(), PNG.to_vec())],
            footnotes: vec![("n1".into(), "The footnote.".into())],
            sidebars: vec![("s1".into(), "The sidebar.".into())],
            record_size: 40,
            ..Default::default()
        })
    }

    #[test]
    fn ereader_palmdoc_end_to_end() {
        let arc = PdbReader.read(&sample_ereader(2)).unwrap();
        let book = validate(&arc);
        assert_eq!(book.metadata().title, "Peanut Book");
        assert_eq!(book.metadata().authors, ["Pea Nut"]);
        assert_eq!(book.metadata().publisher, "Pub Co");
        assert_eq!(book.metadata().identifier("isbn"), Some("1234567890"));
        assert_eq!(toc_titles(&book), ["Chapter One", "Chapter Two"]);
        let t = chapter_texts(&book);
        assert!(t[0].contains("Hello"));
        assert!(t[0].contains("café"));
        assert!(chapter_xml(&book, 0).contains("<i>world</i>"));
        assert!(chapter_xml(&book, 0).contains("<img src=\"images/"));
        let last = chapter_xml(&book, 1);
        assert!(
            last.contains("The footnote."),
            "footnote aside at the end: {last}"
        );
        assert!(last.contains("The sidebar."));
    }

    #[test]
    fn ereader_zlib_version_matches_palmdoc_version() {
        let a = validate(&PdbReader.read(&sample_ereader(2)).unwrap());
        let b = validate(&PdbReader.read(&sample_ereader(10)).unwrap());
        assert_eq!(chapter_texts(&a), chapter_texts(&b));
        assert_eq!(toc_titles(&a), toc_titles(&b));
    }

    #[test]
    fn ereader_old_style_note_ids_and_missing_metadata() {
        let f = write_ereader(&EReaderSpec {
            version: 2,
            title: String::new(),
            text: "Hi\\Fn=\"note10\"1\\Fn\n".into(),
            footnotes: vec![("note10".into(), "Long id note".into())],
            new_style_ids: true,
            no_metadata: true,
            ..Default::default()
        });
        let book = validate(&PdbReader.read(&f).unwrap());
        assert_eq!(
            book.metadata().title,
            "ereader_book",
            "falls back to the database name"
        );
        assert!(chapter_texts(&book).join(" ").contains("Long id note"));
    }

    #[test]
    fn parse_note_ids_both_layouts() {
        assert_eq!(parse_note_ids(b"a\0bb\0"), ["a", "bb"]);
        assert_eq!(
            parse_note_ids(b"\x00\x01\x03abc\x00\x00\x01\x01z\x00"),
            ["abc", "z"]
        );
        assert!(parse_note_ids(b"").is_empty());
        // truncated id: stops instead of panicking
        assert!(parse_note_ids(b"\x00\x01\x09ab").is_empty());
    }

    #[test]
    fn ereader_drm_and_bad_versions_have_clear_errors() {
        for v in [260u16, 272] {
            let e = PdbReader.read(&sample_ereader(v)).unwrap_err();
            assert!(format!("{e}").contains("DRM protected"), "{v}: {e}");
        }
        let e = PdbReader.read(&sample_ereader(7)).unwrap_err();
        assert!(format!("{e}").contains("unsupported eReader compression version 7"));
    }

    #[test]
    fn ereader_structural_errors() {
        let mut f = sample_ereader(2);
        // first non-text record beyond the record count
        let rec0 = be32(&f, 78).unwrap() as usize;
        f[rec0 + 12..rec0 + 14].copy_from_slice(&900u16.to_be_bytes());
        assert!(format!("{}", PdbReader.read(&f).unwrap_err()).contains("first non-text record"));
        let short = pdb_assemble("x", b"PNRd", b"PPrs", &[vec![0, 2, 0, 0], vec![1]]);
        assert!(format!("{}", PdbReader.read(&short).unwrap_err()).contains("too short"));
    }

    #[test]
    fn ereader_corrupt_text_record_names_the_record() {
        let mut f = sample_ereader(10);
        let r1 = be32(&f, 86).unwrap() as usize;
        for b in &mut f[r1..r1 + 6] {
            *b = 0xFF;
        }
        let e = PdbReader.read(&f).unwrap_err();
        assert!(format!("{e:#}").contains("text record 1"), "{e:#}");
    }

    #[test]
    fn ereader_bomb_is_capped() {
        let big = vec![b'a'; MAX_RECORD_OUT + 10];
        let z = zlib_compress(&big);
        let e = ereader_record_text(10, &z).unwrap_err();
        assert!(format!("{e}").contains("expands to more than"));
        assert!(ereader_record_text(3, b"").is_err());
    }

    #[test]
    fn palmdoc_type_goes_through_the_mobi_reader() {
        let f = write_palmdoc("Palm Text", "Hello palmdoc world.\n");
        let book = validate(&PdbReader.read(&f).unwrap());
        assert!(
            chapter_texts(&book)
                .join(" ")
                .contains("Hello palmdoc world.")
        );
        assert!(!PdbReader.sniff(&f), "TEXtREAd stays with the MOBI reader");
    }

    #[test]
    fn ztxt_stream_mode() {
        let text = "Line one of the zTXT book.\n\nLine two, a bit longer than the first.\n";
        let f = write_ztxt("zTXT Book", text, false);
        assert!(PdbReader.sniff(&f));
        let book = validate(&PdbReader.read(&f).unwrap());
        assert_eq!(book.metadata().title, "zTXT Book");
        assert!(chapter_texts(&book).join(" ").contains("Line two"));
    }

    #[test]
    fn ztxt_random_access_and_bad_headers_are_rejected() {
        let f = write_ztxt("t", "text text text\n", true);
        assert!(format!("{}", PdbReader.read(&f).unwrap_err()).contains("random-access"));
        let f = pdb_assemble("t", b"zTXT", b"GPlm", &[vec![0; 8]]);
        assert!(format!("{}", PdbReader.read(&f).unwrap_err()).contains("too short"));
        let f = pdb_assemble("t", b"zTXT", b"GPlm", &[vec![0; 24], vec![1]]);
        assert!(format!("{}", PdbReader.read(&f).unwrap_err()).contains("lists 0 text records"));
        let f = write_ztxt("t", "   \n", false);
        assert!(format!("{}", PdbReader.read(&f).unwrap_err()).contains("no text"));
    }

    #[test]
    fn unknown_type_names_the_codes() {
        let f = pdb_assemble("t", b"Plkr", b"Data", &[vec![1, 2, 3]]);
        let e = PdbReader.read(&f).unwrap_err();
        assert!(format!("{e}").contains("unsupported Palm database type `Plkr` / creator `Data`"));
    }

    #[test]
    fn sniff_extension_and_id() {
        assert_eq!(PdbReader.extensions(), &["pdb"]);
        assert_eq!(PdbReader.id(), "pdb");
        assert!(!PdbReader.sniff(b"short"));
        assert!(PdbReader.sniff(&sample_ereader(2)));
        let m = write_mobi(&MobiSpec::default());
        assert!(!PdbReader.sniff(&m));
    }

    #[test]
    fn fuzz() {
        let f = sample_ereader(2);
        fuzz_no_panic(&PdbReader, &f);
        fuzz_no_panic(&PdbReader, &sample_ereader(10));
        fuzz_no_panic(
            &PdbReader,
            &write_ztxt("z", "Some text for the fuzzer.\n", false),
        );
        assert_rejects_garbage(&PdbReader);
    }
}
