#![forbid(unsafe_code)]
//! Comic reading without GTK. The reader (`src/ui/reader_comic.rs`) draws and handles input;
//! every decision lives here so it can be tested:
//!
//! * [`decode`]: page decoding with size limits (image bombs), downscaling and grey-scale,
//! * [`panels`]: finding the panels of a page by their gutters, for smart zoom and guided view,
//! * [`layout`]: single pages, two-page spreads, the webtoon strip and the reading direction,
//! * [`smart`]: smart zoom targets and the stops of guided view,
//! * [`view`]: the camera (fit, zoom, pan, animation maths),
//! * [`cache`]: a byte-budgeted cache of decoded pages,
//! * [`meta`]: what the converter stores about a comic in the book (page types, double pages),
//! * [`worker`]: decoding on threads of their own, with a queue that follows the reader,
//! * [`settings`]: reading modes and what is remembered per book.

pub mod cache;
pub mod decode;
pub mod encode;
pub mod layout;
pub mod meta;
pub mod panels;
pub mod settings;
pub mod smart;
#[doc(hidden)]
pub mod testgen;
pub mod view;
pub mod volumes;
pub mod worker;

pub use panels::Rect;
pub use settings::{Fit, Mode};
