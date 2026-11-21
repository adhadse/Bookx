#![forbid(unsafe_code)]
//! Input format readers for Bookx.
//!
//! Every reader implements [`bookx_core::contracts::FormatReader`] and converts its format into
//! the internal EPUB [`Archive`]. [`default_readers`] registers them all and [`read_any`] picks
//! the right one by extension, then by content.
//!
//! Formats: FB2/FBZ, RTF, DOCX, ODT, HTMLZ, TXTZ, TXT, MD, HTML, CBZ/CBT/CBR (RAR through `unrar`/7-Zip), DJVU (through DjVuLibre), CBC, MOBI/AZW/AZW3/PRC,
//! PDB (eReader, zTXT, PalmDOC), PML/PMLZ, RB, SNB, TCR, CHM, LRF (best effort), plus the explicit errors
//! for AZW4 (PDF wrapper) and LIT (discontinued format, recognised but not supported). PDF is
//! reflowed to chapters (experimental, see `pdf`).

pub mod azw4;
pub mod cbc;
pub mod chm;
pub mod comic;
pub mod common;
pub mod djvu;
pub mod docx;
pub mod fb2;
pub(crate) mod helper;
pub mod lit;
pub mod lrf;
pub mod lzx;
pub mod mobi;
pub mod mobi_index;
pub mod mobi_kf8;
pub mod odt;
pub mod pdb;
pub mod pdf;
pub mod pml;
pub mod rar;
pub mod rb;
pub mod rtf;
pub mod snb;
pub mod tcr;
pub mod textfmt;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod testutil;

use anyhow::Result;
use bookx_core::archive::Archive;
use bookx_core::contracts::{pick_reader, FormatReader, Registry};

/// Registry with every reader of this crate.
pub fn default_readers() -> Registry<dyn FormatReader> {
    let mut r: Registry<dyn FormatReader> = Registry::new();
    // Order matters for content sniffing: the specific zip based formats come before CBZ
    // (which accepts any zip made of images) and the text formats come last.
    r.register(Box::new(fb2::Fb2Reader));
    r.register(Box::new(fb2::FbzReader));
    r.register(Box::new(rtf::RtfReader));
    r.register(Box::new(docx::DocxReader));
    r.register(Box::new(odt::OdtReader));
    r.register(Box::new(textfmt::HtmlzReader));
    r.register(Box::new(textfmt::TxtzReader));
    r.register(Box::new(mobi::MobiReader));
    // Container formats with a unique magic come next. Their zip based relatives (PMLZ, CBC)
    // must be tried before CBZ, which accepts any zip made of images.
    r.register(Box::new(pdb::PdbReader));
    r.register(Box::new(azw4::Azw4Reader));
    r.register(Box::new(rb::RbReader));
    r.register(Box::new(snb::SnbReader));
    r.register(Box::new(tcr::TcrReader));
    r.register(Box::new(chm::ChmReader));
    r.register(Box::new(lit::LitReader));
    r.register(Box::new(lrf::LrfReader));
    r.register(Box::new(djvu::DjvuReader));
    r.register(Box::new(pdf::PdfReader));
    r.register(Box::new(pml::PmlzReader));
    r.register(Box::new(cbc::CbcReader));
    r.register(Box::new(comic::CbzReader));
    r.register(Box::new(comic::CbtReader));
    r.register(Box::new(comic::CbrReader));
    r.register(Box::new(pml::PmlReader));
    r.register(Box::new(textfmt::HtmlReader));
    r.register(Box::new(textfmt::MarkdownReader));
    r.register(Box::new(textfmt::TxtReader));
    r
}

/// Converts any supported file to an EPUB archive, choosing the reader from the file name and,
/// when the extension is unknown, from the content.
pub fn read_any(file_name: &str, bytes: &[u8]) -> Result<Archive> {
    let readers = default_readers();
    let reader = pick_reader(&readers, file_name, bytes)?;
    let result = match reader.id() {
        // Plain text formats take the book title from the file name.
        "txt" => textfmt::convert_plain("txt", bytes, file_name),
        "md" => textfmt::convert_plain("md", bytes, file_name),
        "html" => textfmt::convert_plain("html", bytes, file_name),
        "tcr" => tcr::convert_tcr(bytes, file_name),
        "pml" => pml::convert_pml(bytes, file_name),
        "pdf" => pdf::convert_pdf(bytes, file_name),
        _ => reader.read(bytes),
    };
    result.map_err(|e| e.context(format!("reading `{file_name}` as {}", reader.id())))
}

/// All file extensions (lower case, without the dot) the readers claim, sorted.
pub fn supported_extensions() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = default_readers()
        .iter()
        .flat_map(|r| r.extensions().iter().copied())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}
