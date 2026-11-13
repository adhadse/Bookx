//! LIT (Microsoft Reader): recognised, not supported.
//!
//! Microsoft discontinued the format, so Bookx does not convert it. The reader exists so that a
//! `.lit` file (or any file starting with the `ITOLITLS` magic) is identified and rejected with
//! a clear message instead of being opened by the plain text reader.

use anyhow::{bail, Result};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;

/// The magic at the start of a LIT file.
pub const MAGIC: &[u8; 8] = b"ITOLITLS";

/// The error shown for every LIT file.
pub const UNSUPPORTED: &str = "LIT (Microsoft Reader) is a discontinued format and is not supported. Convert the book to EPUB with another tool first.";

/// Reader for `.lit` files; always fails with [`UNSUPPORTED`].
pub struct LitReader;

impl FormatReader for LitReader {
    fn id(&self) -> &'static str {
        "lit"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["lit"]
    }
    fn sniff(&self, head: &[u8]) -> bool {
        head.starts_with(MAGIC)
    }
    fn read(&self, _bytes: &[u8]) -> Result<Archive> {
        bail!(UNSUPPORTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fmt_writers::*;
    use crate::testutil::*;

    #[test]
    fn every_lit_file_gets_the_discontinued_error() {
        for f in [write_lit(false), write_lit(true), MAGIC.to_vec(), vec![]] {
            let e = LitReader.read(&f).unwrap_err();
            assert_eq!(format!("{e}"), UNSUPPORTED);
        }
        assert!(UNSUPPORTED.contains("discontinued format"));
    }

    #[test]
    fn sniff_accepts_the_header_and_rejects_other_formats() {
        assert_eq!(LitReader.id(), "lit");
        assert_eq!(LitReader.extensions(), &["lit"]);
        assert!(LitReader.sniff(&write_lit(false)));
        assert!(LitReader.sniff(MAGIC));
        for other in [&b"ITSF"[..], b"PK\x03\x04", b"%PDF-1.4", b"", b"ITOLIT"] {
            assert!(!LitReader.sniff(other));
        }
    }

    #[test]
    fn fuzz_does_not_panic() {
        fuzz_no_panic(&LitReader, &write_lit(false));
        assert_rejects_garbage(&LitReader);
    }
}
