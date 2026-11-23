//! eReader PDB (`PNRdPPrs`) writer.
//!
//! Layout (as written by Calibre and understood by readers): the 78 byte Palm database header, the
//! record list, a 132 byte header record, the PML text in records of 8192 bytes (zlib or PalmDOC
//! compressed), one chapter-index record per heading (`u32` offset of the chapter code, title,
//! NUL), one link-index record per anchor (`u32` offset of the `\Q` code, name, NUL), the images
//! (`"PNG "` + 32 byte name + padding + width/height, 62 bytes, then the PNG), the metadata record
//! (`title NUL author NUL rights NUL publisher NUL isbn NUL`), a 4 byte record (the visible text
//! length twice, as `u16`) that the unused index fields point at, and the `MeTaInFo\0` end marker. No footnotes or sidebars are written.

use std::io::Write;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};

use crate::charset::to_cp1252_lossy;
use crate::mobi::palmdoc;
use crate::model::Model;

use super::pml::{PmlText, render_pml};

/// Text record compression of eReader files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdbCompression {
    /// zlib, header version 10 (what Calibre writes).
    #[default]
    Deflate,
    /// PalmDOC LZ77, header version 2.
    PalmDoc,
}

impl PdbCompression {
    /// The version number stored in the header record.
    pub fn version(self) -> u16 {
        match self {
            PdbCompression::Deflate => 10,
            PdbCompression::PalmDoc => 2,
        }
    }
}

/// Uncompressed size of one text record.
pub const TEXT_RECORD: usize = 8192;
const HEADER_LEN: usize = 132;
const IMAGE_HEADER_LEN: usize = 62;
/// Seconds between the Palm epoch (1904) and the Unix epoch.
const PALM_EPOCH_OFFSET: u64 = 2_082_844_800;

#[derive(Debug, Clone, Copy, Default)]
pub struct EReaderWriter {
    pub compression: PdbCompression,
}

/// A PalmDB file: the 32 byte name, type, creator and the records.
pub fn palm_database(
    name: &[u8],
    db_type: &[u8; 4],
    creator: &[u8; 4],
    records: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut n = [0u8; 32];
    let take = name.len().min(31);
    n[..take].copy_from_slice(&name[..take]);
    out.extend_from_slice(&n);
    out.extend_from_slice(&[0, 0, 0, 0]); // attributes, version
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + PALM_EPOCH_OFFSET;
    let now = (now.min(u32::MAX as u64)) as u32;
    out.extend_from_slice(&now.to_be_bytes()); // created
    out.extend_from_slice(&now.to_be_bytes()); // modified
    out.extend_from_slice(&[0u8; 16]); // backup, modification number, app info, sort info
    out.extend_from_slice(db_type);
    out.extend_from_slice(creator);
    out.extend_from_slice(&((records.len() as u32 * 2) + 1).to_be_bytes()); // unique id seed
    out.extend_from_slice(&[0, 0, 0, 0]); // next record list
    out.extend_from_slice(&(records.len().min(u16::MAX as usize) as u16).to_be_bytes());
    debug_assert_eq!(out.len(), 78);
    let mut offset = 78 + records.len() * 8 + 2;
    for (i, r) in records.iter().enumerate() {
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.push(0);
        out.extend_from_slice(&((i as u32 * 2) & 0x00ff_ffff).to_be_bytes()[1..]);
        offset += r.len();
    }
    out.extend_from_slice(&[0, 0]);
    for r in records {
        out.extend_from_slice(r);
    }
    out
}

fn compress(kind: PdbCompression, chunk: &[u8]) -> Result<Vec<u8>> {
    Ok(match kind {
        PdbCompression::PalmDoc => palmdoc::compress(chunk),
        PdbCompression::Deflate => {
            let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
            e.write_all(chunk)?;
            e.finish()?
        }
    })
}

fn image_record(name: &str, png: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut r = Vec::with_capacity(IMAGE_HEADER_LEN + png.len());
    r.extend_from_slice(b"PNG ");
    let mut n = [0u8; 32];
    let bytes = to_cp1252_lossy(name);
    let take = bytes.len().min(31);
    n[..take].copy_from_slice(&bytes[..take]);
    r.extend_from_slice(&n);
    r.extend_from_slice(&[0u8; 22]);
    r.extend_from_slice(&(width.min(u16::MAX as u32) as u16).to_be_bytes());
    r.extend_from_slice(&(height.min(u16::MAX as u32) as u16).to_be_bytes());
    debug_assert_eq!(r.len(), IMAGE_HEADER_LEN);
    r.extend_from_slice(png);
    r
}

fn field(s: &str) -> Vec<u8> {
    let mut v = to_cp1252_lossy(&s.replace('\0', " "));
    v.push(0);
    v
}

/// Builds the eReader file from rendered PML.
pub fn build_ereader(
    book: &OpenBook,
    pml: &PmlText,
    compression: PdbCompression,
) -> Result<Vec<u8>> {
    let md = book.metadata();
    let text = pml.text.as_bytes();
    let mut records: Vec<Vec<u8>> = vec![Vec::new()];
    for chunk in text.chunks(TEXT_RECORD) {
        records.push(compress(compression, chunk)?);
    }
    if text.is_empty() {
        records.push(compress(compression, b"")?);
    }
    let text_records = records.len() - 1;
    let chapter_start = records.len();
    for ch in &pml.chapters {
        let mut r = (ch.offset as u32).to_be_bytes().to_vec();
        r.extend(field(&ch.title));
        records.push(r);
    }
    let link_start = records.len();
    for (offset, name) in &pml.links {
        let mut r = (*offset as u32).to_be_bytes().to_vec();
        r.extend(field(name));
        records.push(r);
    }
    let image_start = records.len();
    for img in &pml.images {
        records.push(image_record(&img.name, &img.png, img.width, img.height));
    }
    let meta_start = records.len();
    let mut meta = Vec::new();
    meta.extend(field(&md.title));
    meta.extend(field(&md.authors.join(" & ")));
    meta.extend(field(&md.rights));
    meta.extend(field(&md.publisher));
    meta.extend(field(md.identifier("isbn").unwrap_or("")));
    records.push(meta);
    let dummy = records.len();
    let visible = (pml.visible_chars.min(u16::MAX as usize) as u16).to_be_bytes();
    records.push(vec![visible[0], visible[1], visible[0], visible[1]]);
    records.push(b"MeTaInFo\0".to_vec());

    let mut h = [0u16; HEADER_LEN / 2];
    let u = |n: usize| n.min(u16::MAX as usize) as u16;
    h[0] = compression.version();
    h[3] = 0x6240; // constant seen in Calibre's files
    h[6] = u(text_records + 1);
    h[7] = u(pml.chapters.len());
    h[10] = u(pml.images.len());
    h[11] = u(pml.links.len());
    h[12] = 1; // has a metadata record
    h[16] = u(chapter_start);
    h[17] = 0x0a00; // constant seen in Calibre's files
    h[18] = u(dummy);
    h[19] = u(dummy);
    h[20] = u(if pml.images.is_empty() {
        dummy
    } else {
        image_start
    });
    h[21] = u(if pml.links.is_empty() {
        dummy
    } else {
        link_start
    });
    h[22] = u(meta_start);
    h[24] = u(dummy);
    h[25] = u(dummy);
    h[26] = u(dummy);
    records[0] = h.iter().flat_map(|v| v.to_be_bytes()).collect();

    let name: Vec<u8> = to_cp1252_lossy(&md.title)
        .into_iter()
        .map(|b| if b == 0 { b' ' } else { b })
        .collect();
    Ok(palm_database(&name, b"PNRd", b"PPrs", &records))
}

impl FormatWriter for EReaderWriter {
    fn id(&self) -> &'static str {
        "pdb"
    }
    fn extension(&self) -> &'static str {
        "pdb"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        let model = Model::build(book);
        let pml = render_pml(book, &model);
        build_ereader(book, &pml, self.compression)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palm_header_and_record_list() {
        let db = palm_database(b"Name", b"PNRd", b"PPrs", &[vec![1, 2, 3], vec![4], vec![]]);
        assert_eq!(&db[..4], b"Name");
        assert!(db[4..32].iter().all(|b| *b == 0));
        assert_eq!(&db[60..64], b"PNRd");
        assert_eq!(&db[64..68], b"PPrs");
        assert_eq!(u16::from_be_bytes([db[76], db[77]]), 3);
        let off = |i: usize| u32::from_be_bytes(db[78 + i * 8..82 + i * 8].try_into().unwrap());
        assert_eq!(off(0), 78 + 24 + 2);
        assert_eq!(off(1), off(0) + 3);
        assert_eq!(off(2), off(1) + 1);
        assert_eq!(&db[off(0) as usize..], &[1, 2, 3, 4]);
        // two byte gap before the data
        assert_eq!(&db[78 + 24..78 + 26], &[0, 0]);
    }

    #[test]
    fn long_names_are_truncated_and_terminated() {
        let db = palm_database(&[b'x'; 80], b"TEXt", b"REAd", &[vec![]]);
        assert_eq!(db[31], 0);
        assert_eq!(db[30], b'x');
    }

    #[test]
    fn image_record_layout() {
        let r = image_record("a.png", &[9, 9], 40, 30);
        assert_eq!(&r[..4], b"PNG ");
        assert_eq!(&r[4..9], b"a.png");
        assert_eq!(r[9], 0);
        assert_eq!(u16::from_be_bytes([r[58], r[59]]), 40);
        assert_eq!(u16::from_be_bytes([r[60], r[61]]), 30);
        assert_eq!(&r[62..], &[9, 9]);
    }

    #[test]
    fn fields_are_nul_terminated_cp1252() {
        assert_eq!(field("aé"), vec![b'a', 0xe9, 0]);
        assert_eq!(field("a\0b"), vec![b'a', b' ', b'b', 0]);
        assert_eq!(field(""), vec![0]);
    }

    #[test]
    fn compression_versions() {
        assert_eq!(PdbCompression::Deflate.version(), 10);
        assert_eq!(PdbCompression::PalmDoc.version(), 2);
        assert_eq!(PdbCompression::default(), PdbCompression::Deflate);
    }

    #[test]
    fn both_compressions_round_trip() {
        let data = b"hello hello hello hello world ".repeat(50);
        let z = compress(PdbCompression::Deflate, &data).unwrap();
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&z[..]), &mut out).unwrap();
        assert_eq!(out, data);
        let p = compress(PdbCompression::PalmDoc, &data).unwrap();
        assert!(p.len() < data.len());
    }

    use crate::testutil::book_with;

    #[test]
    fn writer_identity_and_a_small_file() {
        assert_eq!(EReaderWriter::default().id(), "pdb");
        assert_eq!(EReaderWriter::default().extension(), "pdb");
        let book = book_with(&["<h1>T</h1><p>Hello <img src=\"pic.png\"/></p>"]);
        let bytes = EReaderWriter::default().write(&book).unwrap();
        assert_eq!(&bytes[60..68], b"PNRdPPrs");
        let n = u16::from_be_bytes([bytes[76], bytes[77]]) as usize;
        // header, text, chapter, links(0 or more), image, metadata, dummy, end
        assert!(n >= 7, "{n} records");
        assert!(bytes.ends_with(b"MeTaInFo\0"));
        let empty = EReaderWriter {
            compression: PdbCompression::PalmDoc,
        }
        .write(&book_with(&["<div></div>"]))
        .unwrap();
        assert_eq!(u16::from_be_bytes([empty[76], empty[77]]), 5);
    }
}
