#![forbid(unsafe_code)]
//! Pure logic behind Bookx dictionaries, vocabulary, reading statistics,
//! in-book search, RSVP reading and annotation export. No GTK, no network.

pub mod anki;
pub mod bookmap;
pub mod dictset;
pub mod dwell;
pub mod export;
pub mod ids;
pub mod import;
pub mod mapview;
pub mod notes_query;
pub mod rsvp;
pub mod search;
pub mod stardict;
pub mod stats;
pub mod thumbs;
pub mod vocab;

#[cfg(test)]
pub(crate) mod testutil;
