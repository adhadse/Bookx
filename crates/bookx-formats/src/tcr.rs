//! TCR (Psion "Text Compression for Reader") input.
//!
//! Layout: the 9 byte tag `!!8-Bit!!`, then 256 dictionary entries (one length byte followed by
//! that many bytes), then the payload in which every byte is an index into the dictionary. The
//! text is Windows-1252 / UTF-8 (auto detected) and goes through the plain text importer.

use anyhow::{Result, bail};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;

use crate::textfmt::convert_plain;

/// The magic tag at the start of an 8-bit TCR file.
pub const MAGIC: &[u8] = b"!!8-Bit!!";
/// Largest decompressed text.
const MAX_TEXT: usize = 128 << 20;

/// Reader for `.tcr` files.
pub struct TcrReader;

impl FormatReader for TcrReader {
    fn id(&self) -> &'static str {
        "tcr"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["tcr"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(MAGIC)
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        convert_tcr(bytes, "")
    }
}

/// Expands the TCR payload to raw text bytes.
pub fn tcr_decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    let Some(rest) = bytes.strip_prefix(MAGIC) else {
        bail!("not a TCR file: the `!!8-Bit!!` tag is missing");
    };
    let mut dict: Vec<&[u8]> = Vec::with_capacity(256);
    let mut pos = 0usize;
    for i in 0..256 {
        let Some(&len) = rest.get(pos) else {
            bail!("the TCR dictionary is truncated at entry {i}");
        };
        let start = pos + 1;
        let Some(entry) = rest.get(start..start + len as usize) else {
            bail!("the TCR dictionary is truncated at entry {i}");
        };
        dict.push(entry);
        pos = start + len as usize;
    }
    let payload = &rest[pos..];
    let mut out = Vec::with_capacity(payload.len().saturating_mul(2).min(MAX_TEXT));
    for &b in payload {
        out.extend_from_slice(dict[b as usize]);
        if out.len() > MAX_TEXT {
            bail!("the TCR text expands to more than {MAX_TEXT} bytes");
        }
    }
    Ok(out)
}

/// Converts a TCR file; `file_name` (may be empty) supplies the title.
pub fn convert_tcr(bytes: &[u8], file_name: &str) -> Result<Archive> {
    let text = tcr_decompress(bytes)?;
    if text.iter().all(|b| b.is_ascii_whitespace()) {
        bail!("the TCR file contains no text");
    }
    convert_plain("txt", &text, file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fmt_writers::*;
    use crate::testutil::*;

    #[test]
    fn decompress_hand_computed() {
        // Dictionary: 0 -> "ab", 1 -> "c", 2 -> "" (empty), everything else single bytes
        // equal to the index; payload 0,1,0,2,0x41 -> "ab" "c" "ab" "" "A".
        let mut f = MAGIC.to_vec();
        for i in 0..256u32 {
            match i {
                0 => f.extend([2, b'a', b'b']),
                1 => f.extend([1, b'c']),
                2 => f.push(0),
                _ => f.extend([1, i as u8]),
            }
        }
        f.extend([0, 1, 0, 2, 0x41]);
        assert_eq!(tcr_decompress(&f).unwrap(), b"abcabA");
    }

    #[test]
    fn decompress_edge_cases() {
        let empty = tcr_file(&[], b"");
        assert_eq!(tcr_decompress(&empty).unwrap(), b"");
        let e = tcr_decompress(b"nope").unwrap_err();
        assert!(format!("{e}").contains("tag is missing"));
        let mut cut = tcr_file(&[], b"x");
        cut.truncate(MAGIC.len() + 10);
        assert!(format!("{}", tcr_decompress(&cut).unwrap_err()).contains("truncated"));
        // Entry claims 200 bytes but the file ends.
        let mut bad = MAGIC.to_vec();
        bad.extend([200, 1, 2, 3]);
        assert!(format!("{}", tcr_decompress(&bad).unwrap_err()).contains("truncated at entry 0"));
    }

    #[test]
    fn expansion_is_capped() {
        // 255 byte entries, 600k payload bytes would be 150 MB > cap.
        let big = vec![b'x'; 255];
        let f = tcr_file(&[(0, &big)], &vec![0u8; 600_000]);
        let e = tcr_decompress(&f).unwrap_err();
        assert!(format!("{e}").contains("expands to more than"));
    }

    #[test]
    fn writer_roundtrip_unicode_and_binary_text() {
        let text = "Grüße aus Köln.\nZweite Zeile — mit Strich.\n".to_string();
        let packed = tcr_compress(text.as_bytes());
        assert_eq!(tcr_decompress(&packed).unwrap(), text.as_bytes());
    }

    #[test]
    fn converts_to_a_book() {
        let text = "Chapter One\n\nIt was a dark night. It was a dark night.\n\nThe end.\n";
        let arc = TcrReader.read(&tcr_compress(text.as_bytes())).unwrap();
        let book = validate(&arc);
        let joined = chapter_texts(&book).join(" ");
        assert!(joined.contains("It was a dark night."));
        assert!(joined.contains("The end."));
    }

    #[test]
    fn file_name_becomes_the_title() {
        let arc = convert_tcr(&tcr_compress(b"Hello there.\n"), "dir/My Story.tcr").unwrap();
        assert_eq!(validate(&arc).metadata().title, "My Story");
    }

    #[test]
    fn empty_text_is_rejected() {
        let e = TcrReader.read(&tcr_compress(b"  \n")).unwrap_err();
        assert!(format!("{e}").contains("no text"));
    }

    #[test]
    fn sniff_and_extensions() {
        assert!(TcrReader.sniff(b"!!8-Bit!!\x00"));
        assert!(!TcrReader.sniff(b"!!8-Bit"));
        assert!(!TcrReader.sniff(b"PK\x03\x04"));
        assert_eq!(TcrReader.extensions(), &["tcr"]);
        assert_eq!(TcrReader.id(), "tcr");
    }

    #[test]
    fn fuzz() {
        let s = tcr_compress("Chapter 1\n\nSome text for the fuzzer to chew on.\n".as_bytes());
        fuzz_no_panic(&TcrReader, &s);
        assert_rejects_garbage(&TcrReader);
    }
}
