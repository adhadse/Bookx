#![forbid(unsafe_code)]
//! Conversion pipeline of Bookx: output writers, transforms and presets.

pub mod charset;
pub mod images;
pub mod inspect;
pub mod markup;
pub mod mobi;
pub mod model;
pub mod options;
pub mod pipeline;
pub mod queue;
pub mod report;
#[cfg(test)]
pub(crate) mod testutil;
pub mod transforms;
pub mod writers;

pub use options::{builtin_presets, ConvertOptions, Preset, PresetStore};
pub use pipeline::{convert, convert_file, read_input, Cancelled, Converted, Progress};
pub use transforms::default_transforms;
pub use writers::default_writers;
