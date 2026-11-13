#![forbid(unsafe_code)]
//! Read aloud for Bookx (roadmap R10, enhancement 9). GTK-free.
//!
//! * [`text`]: sentences, chunks, language of a book
//! * [`session`]: the [`session::Engine`] interface, the reading [`session::Session`], the mock
//! * [`pipeline`]: gapless playback with look-ahead synthesis for any [`synth::Synthesizer`]
//! * engines: [`server`] (OpenAI-compatible and XTTS/Fish
//!   servers), [`kokoro`] and [`piper`] (ONNX Runtime, run in the process)
//! * [`models`] / [`download`]: the packs the user downloads on demand, hash-verified
//! * [`cache`], [`sink`], [`prefs`]

pub mod cache;
#[rustfmt::skip]
pub mod catalog_data;
pub mod download;
pub mod export;
pub mod factory;
pub mod items;
pub mod kokoro;
pub mod models;
pub mod onnx;
pub mod phonemize;
pub mod pipeline;
pub mod piper;
pub mod prefs;
pub mod server;
pub mod service;
pub mod session;
pub mod setup;
pub mod sink;
pub mod synth;
pub mod text;

pub use prefs::{ServerPrefs, TtsPrefs};
pub use session::{
    voices_for, Engine, Event, EventKind, MockEngine, Session, SleepTimer, State, Update, Voice,
};
pub use text::{split_sentences, Sentence};
