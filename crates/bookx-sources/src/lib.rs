#![forbid(unsafe_code)]
//! Online and "around the library" tools of Bookx (roadmap track T8).
//!
//! Every online piece is split into a request builder, a pure response parser and a [`fetch::Fetcher`]
//! so that tests never touch the network; [`fetch::UreqFetcher`] is the only network code.

pub mod aliases;
pub mod audiobooks;
pub mod authors;
pub mod cache;
pub mod cover;
pub mod dedup;
pub mod feed;
pub mod fetch;
pub mod filename;
pub mod health;
pub mod idclean;
pub mod identifiers;
pub mod isbn;
pub mod readability;
pub mod recipe;
pub mod sources;
pub mod template;
pub mod text;
pub mod tidy;
pub mod trackers;
