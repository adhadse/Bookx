//! MOBI 6 writer (PalmDB container, PalmDOC compression, MOBI header, EXTH, NCX index).
//!
//! Layout of the produced file, record by record:
//! `0` header | `1..=n` compressed text | images (cover first, then its thumbnail last) |
//! INDX header, INDX data, CNCX | FLIS | FCIS | EOF (`E9 8E 0D 0A`).

use anyhow::{bail, Result};
use bookx_core::book::OpenBook;
use bookx_core::contracts::FormatWriter;

pub(crate) mod html;
pub(crate) mod indx;
pub mod kf8;
pub mod palmdoc;

pub use html::to_mobi_image;
pub use indx::vwi;
pub use kf8::Azw3Writer;

pub(crate) const TEXT_RECORD: usize = 4096;
pub(crate) const NO_VALUE: u32 = 0xffff_ffff;
const MOBI_HEADER_LEN: usize = 232;

#[derive(Debug, Clone, Copy)]
pub struct MobiWriter {
    /// Append a "Table of Contents" page and point the guide at it (what Calibre does, and what
    /// Calibre's own MOBI reader needs to recover the table of contents).
    pub inline_toc: bool,
}

impl Default for MobiWriter {
    fn default() -> Self {
        MobiWriter { inline_toc: true }
    }
}

impl FormatWriter for MobiWriter {
    fn id(&self) -> &'static str {
        "mobi"
    }
    fn extension(&self) -> &'static str {
        "mobi"
    }
    fn write(&self, book: &OpenBook) -> Result<Vec<u8>> {
        write_mobi(book, self.inline_toc)
    }
}

pub(crate) fn put_u32(v: &mut [u8], at: usize, val: u32) {
    v[at..at + 4].copy_from_slice(&val.to_be_bytes());
}

pub(crate) fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// Splits at 4096 bytes without cutting a UTF-8 sequence.
pub(crate) fn split_text(text: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + TEXT_RECORD).min(text.len());
        while end < text.len() && end > start + 1 && (text[end] & 0xc0) == 0x80 {
            end -= 1;
        }
        out.push(&text[start..end]);
        start = end;
    }
    out
}

pub(crate) fn locale_for(lang: &str) -> u32 {
    match lang
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "ar" => 1,
        "bg" => 2,
        "cs" => 5,
        "da" => 6,
        "de" => 7,
        "el" => 8,
        "en" => 9,
        "es" => 10,
        "fi" => 11,
        "fr" => 12,
        "he" => 13,
        "hu" => 14,
        "it" => 16,
        "ja" => 17,
        "ko" => 18,
        "nl" => 19,
        "no" | "nb" | "nn" => 20,
        "pl" => 21,
        "pt" => 22,
        "ro" => 24,
        "ru" => 25,
        "sv" => 29,
        "tr" => 31,
        "uk" => 34,
        "zh" => 4,
        _ => 0,
    }
}

pub(crate) fn exth_record(out: &mut Vec<u8>, count: &mut u32, kind: u32, data: &[u8]) {
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(data);
    *count += 1;
}

pub(crate) fn pdb_name(title: &str) -> Vec<u8> {
    let mut name: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.truncate(31);
    if name.is_empty() {
        name.push_str("Book");
    }
    let mut v = name.into_bytes();
    v.resize(32, 0);
    v
}

/// Wraps `records` in a PalmDB container named after `title`.
pub(crate) fn assemble_pdb(title: &str, records: &[Vec<u8>]) -> Vec<u8> {
    let now = chrono::Utc::now().timestamp().max(0) as u32;
    let mut out = pdb_name(title);
    out.extend_from_slice(&0u16.to_be_bytes()); // attributes
    out.extend_from_slice(&0u16.to_be_bytes()); // version
    out.extend_from_slice(&now.to_be_bytes());
    out.extend_from_slice(&now.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // backup
    out.extend_from_slice(&0u32.to_be_bytes()); // modification number
    out.extend_from_slice(&0u32.to_be_bytes()); // app info
    out.extend_from_slice(&0u32.to_be_bytes()); // sort info
    out.extend_from_slice(b"BOOKMOBI");
    out.extend_from_slice(&((2 * records.len() - 1) as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // next record list
    out.extend_from_slice(&(records.len() as u16).to_be_bytes());
    let mut offset = out.len() + records.len() * 8 + 2;
    for (i, r) in records.iter().enumerate() {
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.push(0);
        out.extend_from_slice(&((2 * i) as u32).to_be_bytes()[1..]);
        offset += r.len();
    }
    out.extend_from_slice(&[0, 0]);
    for r in records {
        out.extend_from_slice(r);
    }
    out
}

fn write_mobi(book: &OpenBook, inline_toc: bool) -> Result<Vec<u8>> {
    let md = book.metadata();
    let conv = html::convert(book, inline_toc)?;
    if conv.text.len() > u32::MAX as usize / 2 {
        bail!("the book is too large for the MOBI format");
    }

    // --- text records ---------------------------------------------------------------------
    let text_records: Vec<Vec<u8>> = split_text(&conv.text)
        .into_iter()
        .map(palmdoc::compress)
        .collect();
    if text_records.len() > 0xfff0 {
        bail!("the book has too many text records for the MOBI format");
    }
    let n_text = text_records.len();

    // --- record numbering -------------------------------------------------------------------------
    let first_image = 1 + n_text;
    let n_images = conv.images.len();
    let indx_start = first_image + n_images;
    let ncx_items: Vec<(String, usize, usize)> = conv
        .toc
        .iter()
        .map(|t| (t.title.clone(), t.offset, t.depth))
        .collect();
    let index_records = indx::build_ncx_index(&indx::link_entries(&ncx_items, conv.text.len()));
    let after_index = indx_start + index_records.len();
    let flis = after_index;
    let fcis = after_index + 1;

    // --- record 0 ------------------------------------------------------------------------------------
    let mut exth = Vec::new();
    let mut n_exth = 0u32;
    for a in &md.authors {
        exth_record(&mut exth, &mut n_exth, 100, a.as_bytes());
    }
    if !md.publisher.is_empty() {
        exth_record(&mut exth, &mut n_exth, 101, md.publisher.as_bytes());
    }
    let descr = crate::markup::plain_text(&md.description);
    if !descr.trim().is_empty() {
        exth_record(&mut exth, &mut n_exth, 103, descr.trim().as_bytes());
    }
    if let Some(isbn) = md.identifier("isbn") {
        exth_record(&mut exth, &mut n_exth, 104, isbn.as_bytes());
    }
    for t in &md.tags {
        exth_record(&mut exth, &mut n_exth, 105, t.as_bytes());
    }
    if !md.published.is_empty() {
        exth_record(&mut exth, &mut n_exth, 106, md.published.as_bytes());
    }
    exth_record(&mut exth, &mut n_exth, 108, b"Bookx");
    exth_record(
        &mut exth,
        &mut n_exth,
        113,
        uuid::Uuid::new_v4().to_string().as_bytes(),
    );
    exth_record(
        &mut exth,
        &mut n_exth,
        116,
        &(conv.start_offset as u32).to_be_bytes(),
    );
    if let Some(c) = conv.cover_index {
        exth_record(&mut exth, &mut n_exth, 201, &(c as u32).to_be_bytes());
    }
    if let Some(t) = conv.thumb_index {
        exth_record(&mut exth, &mut n_exth, 202, &(t as u32).to_be_bytes());
    }
    exth_record(&mut exth, &mut n_exth, 501, b"EBOK");
    exth_record(&mut exth, &mut n_exth, 503, md.title.as_bytes());
    if !md.language.is_empty() {
        exth_record(&mut exth, &mut n_exth, 524, md.language.as_bytes());
    }

    let mut r0 = vec![0u8; 16 + MOBI_HEADER_LEN];
    r0[0..2].copy_from_slice(&2u16.to_be_bytes()); // PalmDOC compression
    put_u32(&mut r0, 4, conv.text.len() as u32);
    r0[8..10].copy_from_slice(&(n_text as u16).to_be_bytes());
    r0[10..12].copy_from_slice(&(TEXT_RECORD as u16).to_be_bytes());
    r0[16..20].copy_from_slice(b"MOBI");
    put_u32(&mut r0, 20, MOBI_HEADER_LEN as u32);
    put_u32(&mut r0, 24, 2); // MOBI book
    put_u32(&mut r0, 28, 65001);
    put_u32(&mut r0, 32, uuid::Uuid::new_v4().as_u128() as u32);
    put_u32(&mut r0, 36, 6);
    for off in (40..=76).step_by(4) {
        put_u32(&mut r0, off, NO_VALUE);
    }
    put_u32(&mut r0, 80, (1 + n_text) as u32); // first non-text record
    let locale = locale_for(&md.language);
    put_u32(&mut r0, 92, locale);
    put_u32(&mut r0, 104, 6); // minimum version
    put_u32(
        &mut r0,
        108,
        if n_images > 0 {
            first_image as u32
        } else {
            NO_VALUE
        },
    );
    put_u32(&mut r0, 128, 0x40); // EXTH present
    put_u32(&mut r0, 164, NO_VALUE);
    put_u32(&mut r0, 168, NO_VALUE); // no DRM
    r0[192..194].copy_from_slice(&1u16.to_be_bytes());
    r0[194..196].copy_from_slice(&((after_index - 1) as u16).to_be_bytes());
    put_u32(&mut r0, 196, 1);
    put_u32(&mut r0, 200, fcis as u32);
    put_u32(&mut r0, 204, 1);
    put_u32(&mut r0, 208, flis as u32);
    put_u32(&mut r0, 212, 1);
    put_u32(&mut r0, 224, NO_VALUE);
    put_u32(&mut r0, 232, NO_VALUE);
    put_u32(&mut r0, 236, NO_VALUE);
    put_u32(&mut r0, 240, 0); // no trailing entries after text records
    put_u32(&mut r0, 244, indx_start as u32);

    let mut exth_block = b"EXTH".to_vec();
    exth_block.extend_from_slice(&((exth.len() + 12) as u32).to_be_bytes());
    exth_block.extend_from_slice(&n_exth.to_be_bytes());
    exth_block.extend_from_slice(&exth);
    pad4(&mut exth_block);
    r0.extend_from_slice(&exth_block);
    let name_offset = r0.len();
    let title = if md.title.is_empty() {
        "Untitled"
    } else {
        &md.title
    };
    r0.extend_from_slice(title.as_bytes());
    r0.extend_from_slice(&[0, 0]);
    pad4(&mut r0);
    r0.extend_from_slice(&[0u8; 8]);
    put_u32(&mut r0, 84, name_offset as u32);
    put_u32(&mut r0, 88, title.len() as u32);

    // --- magic records ------------------------------------------------------------------------------------
    let mut flis_rec = b"FLIS".to_vec();
    flis_rec.extend_from_slice(&8u32.to_be_bytes());
    flis_rec.extend_from_slice(&0x41u16.to_be_bytes());
    flis_rec.extend_from_slice(&[0; 2]);
    flis_rec.extend_from_slice(&0u32.to_be_bytes());
    flis_rec.extend_from_slice(&NO_VALUE.to_be_bytes());
    flis_rec.extend_from_slice(&1u16.to_be_bytes());
    flis_rec.extend_from_slice(&3u16.to_be_bytes());
    flis_rec.extend_from_slice(&3u32.to_be_bytes());
    flis_rec.extend_from_slice(&1u32.to_be_bytes());
    flis_rec.extend_from_slice(&NO_VALUE.to_be_bytes());
    let mut fcis_rec = b"FCIS".to_vec();
    for v in [0x14u32, 0x10, 1, 0, conv.text.len() as u32, 0, 0x20, 8] {
        fcis_rec.extend_from_slice(&v.to_be_bytes());
    }
    fcis_rec.extend_from_slice(&1u16.to_be_bytes());
    fcis_rec.extend_from_slice(&1u16.to_be_bytes());
    fcis_rec.extend_from_slice(&0u32.to_be_bytes());
    let eof = vec![0xe9, 0x8e, 0x0d, 0x0a];

    // --- assemble ------------------------------------------------------------------------------------------------
    let mut records: Vec<Vec<u8>> = vec![r0];
    records.extend(text_records);
    records.extend(conv.images);
    records.extend(index_records);
    records.push(flis_rec);
    records.push(fcis_rec);
    records.push(eof);
    if records.len() > 0xffff {
        bail!("the book has too many records for the MOBI format");
    }

    Ok(assemble_pdb(title, &records))
}
