//! Output format writers.

pub mod docx;
pub mod epub;
pub mod fb2;
pub mod flat;
pub mod kepub;
pub mod oeb;
pub mod pdb;
pub mod pml;
pub mod rb;
pub mod rtf;
pub mod snb;

use bookx_core::contracts::{FormatWriter, Registry};

pub use docx::{DocxOptions, DocxWriter, PageSize};
pub use epub::{EpubVersion, EpubWriter};
pub use fb2::Fb2Writer;
pub use flat::{HtmlWriter, HtmlzWriter, MarkdownWriter, TxtWriter, TxtzWriter};
pub use kepub::KepubWriter;
pub use oeb::{OebWriter, ZipWriter};
pub use pdb::{EReaderWriter, PdbCompression};
pub use pml::PmlzWriter;
pub use rb::RbWriter;
pub use rtf::RtfWriter;
pub use snb::SnbWriter;

use crate::mobi::{Azw3Writer, MobiWriter};

/// Every output format Bookx can write.
pub fn default_writers() -> Registry<dyn FormatWriter> {
    let mut r: Registry<dyn FormatWriter> = Registry::new();
    r.register(Box::new(EpubWriter::default()));
    r.register(Box::new(KepubWriter::default()));
    r.register(Box::new(Fb2Writer));
    r.register(Box::new(HtmlzWriter));
    r.register(Box::new(TxtzWriter));
    r.register(Box::new(TxtWriter));
    r.register(Box::new(MarkdownWriter));
    r.register(Box::new(HtmlWriter));
    r.register(Box::new(MobiWriter::default()));
    r.register(Box::new(Azw3Writer));
    r.register(Box::new(DocxWriter::default()));
    r.register(Box::new(RtfWriter));
    r.register(Box::new(EReaderWriter::default()));
    r.register(Box::new(PmlzWriter));
    r.register(Box::new(RbWriter));
    r.register(Box::new(SnbWriter));
    r.register(Box::new(OebWriter));
    r.register(Box::new(ZipWriter));
    r
}
