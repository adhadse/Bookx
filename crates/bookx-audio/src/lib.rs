#![forbid(unsafe_code)]
//! Audiobooks for Bookx (roadmap PP-G-25). GTK-free.
//!
//! * [`format`]: what a file is (by its first bytes, never by its name alone)
//! * [`id3`], [`mp4`], [`vorbis`], [`ogg`]: tag, chapter and cover readers with hard limits (the
//!   files come from anywhere, so nothing in them decides how much memory is used)
//! * [`probe`]: duration and tags of one file; [`scan`]: a folder or a zip of parts into an
//!   [`model::AudioBook`]
//! * [`model`]: parts, chapters and the position arithmetic (chapter math, resume, time left)
//! * [`decode`], [`stretch`], [`player`]: decoding with Symphonia, pitch preserving speed change,
//!   and the playback engine that writes to a [`player::AudioSink`]
//! * [`testgen`]: tiny silent files for tests

pub mod decode;
pub mod format;
pub mod id3;
pub mod limits;
pub mod model;
pub mod mp4;
pub mod ogg;
pub mod player;
pub mod probe;
pub mod scan;
pub mod state;
pub mod stretch;
pub mod testgen;
pub mod text;
pub mod vorbis;

pub use format::Format;
pub use model::{AudioBook, Chapter, Part};
pub use probe::{probe_file, FileInfo};
