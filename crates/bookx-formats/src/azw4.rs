//! AZW4 (Kindle Print Replica): a PDF document wrapped in a MOBI container.
//!
//! PDF input is deferred (owner decision D3), so a replica cannot be converted. The reader
//! still opens the MOBI wrapper to report the title and authors in its error, and
//! [`azw4_metadata`] exposes them. An `.azw4` file without an embedded PDF is an ordinary MOBI
//! book and is converted as such.

use anyhow::{Result, bail};
use bookx_core::archive::Archive;
use bookx_core::contracts::FormatReader;
use bookx_core::opf::Metadata;

use crate::mobi::{convert_mobi, peek_metadata};

/// Reader for `.azw4` files.
pub struct Azw4Reader;

impl FormatReader for Azw4Reader {
    fn id(&self) -> &'static str {
        "azw4"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["azw4"]
    }
    // Print replicas look exactly like MOBI files; only the extension identifies them.
    fn sniff(&self, _head: &[u8]) -> bool {
        false
    }
    fn read(&self, bytes: &[u8]) -> Result<Archive> {
        if !contains_pdf(bytes) {
            return convert_mobi(bytes);
        }
        let md = azw4_metadata(bytes).unwrap_or_default();
        let mut what = String::new();
        if !md.title.is_empty() {
            what.push_str(&format!(" \u{201c}{}\u{201d}", md.title));
        }
        if !md.authors.is_empty() {
            what.push_str(&format!(" by {}", md.authors.join(", ")));
        }
        bail!(
            "AZW4 is a PDF wrapper (Kindle Print Replica){what}; PDF support is deferred, so it cannot be converted"
        )
    }
}

/// Whether the file embeds a PDF document.
pub fn contains_pdf(bytes: &[u8]) -> bool {
    bytes.windows(5).any(|w| w == b"%PDF-")
}

/// Title and authors of the MOBI wrapper.
pub fn azw4_metadata(bytes: &[u8]) -> Result<Metadata> {
    peek_metadata(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::mobi_writer::*;
    use crate::testutil::*;

    fn replica() -> Vec<u8> {
        write_mobi(&MobiSpec {
            title: "Print Book".into(),
            authors: vec!["Pat Print".into(), "Quin".into()],
            html: "<html><body><p>%PDF-1.4 fake replica payload</p></body></html>".into(),
            ..Default::default()
        })
    }

    #[test]
    fn replica_gives_a_clear_error_with_metadata() {
        let e = Azw4Reader.read(&replica()).unwrap_err();
        let m = format!("{e}");
        assert!(m.contains("AZW4 is a PDF wrapper"), "{m}");
        assert!(m.contains("PDF support is deferred"), "{m}");
        assert!(
            m.contains("Print Book") && m.contains("Pat Print, Quin"),
            "{m}"
        );
    }

    #[test]
    fn metadata_is_extracted() {
        let md = azw4_metadata(&replica()).unwrap();
        assert_eq!(md.title, "Print Book");
        assert_eq!(md.authors, ["Pat Print", "Quin"]);
        assert!(azw4_metadata(b"garbage").is_err());
    }

    #[test]
    fn broken_wrapper_still_reports_the_pdf_error() {
        let mut data = b"%PDF-1.7 not a mobi at all".to_vec();
        data.extend([0u8; 100]);
        let e = Azw4Reader.read(&data).unwrap_err();
        let m = format!("{e}");
        assert!(
            m.contains("AZW4 is a PDF wrapper") && !m.contains("by "),
            "{m}"
        );
    }

    #[test]
    fn plain_mobi_with_azw4_extension_is_converted() {
        let f = write_mobi(&MobiSpec {
            title: "Normal".into(),
            html: "<html><body><h1>One</h1><p>Plain mobi text.</p></body></html>".into(),
            ..Default::default()
        });
        let book = validate(&Azw4Reader.read(&f).unwrap());
        assert!(chapter_texts(&book).join(" ").contains("Plain mobi text."));
    }

    #[test]
    fn contains_pdf_and_registration() {
        assert!(contains_pdf(b"xx%PDF-1.4"));
        assert!(!contains_pdf(b"%PDF"));
        assert!(!contains_pdf(b""));
        assert_eq!(Azw4Reader.id(), "azw4");
        assert_eq!(Azw4Reader.extensions(), &["azw4"]);
        assert!(!Azw4Reader.sniff(&replica()));
    }

    #[test]
    fn fuzz() {
        fuzz_no_panic(&Azw4Reader, &replica());
        assert_rejects_garbage(&Azw4Reader);
    }
}
