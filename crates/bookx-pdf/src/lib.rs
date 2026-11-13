#![forbid(unsafe_code)]
//! PDF for Bookx: pure-Rust rendering (hayro), text with positions, outline, links, search and a
//! reflow to chapters. See `docs/pdf-research.md` for why these libraries.
//!
//! [`Document`] is the synchronous engine (one thread); [`Worker`] owns a document on a thread of
//! its own for the UI.

pub mod cache;
pub mod doc;
pub mod reflow;
pub mod render;
#[doc(hidden)]
pub mod testpdf;
pub mod text;
pub mod worker;

pub use cache::PageCache;
pub use doc::{Dest, Document, Info, Link, LinkTarget, OpenError, OutlineItem};
pub use render::{Bitmap, Renderer};
pub use text::{PageText, TextChar, TextLine};
pub use worker::{Cancel, Event, Hit, Summary, Worker};
