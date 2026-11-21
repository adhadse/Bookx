#![forbid(unsafe_code)]
//! SQLite catalogue store of Bookx.
//!
//! * [`SqliteStore`] implements [`bookx_core::contracts::Store`] over a versioned SQLite schema (WAL),
//!   with FTS5 indexes for metadata and for book content text.
//! * [`open_library`] opens a library whose catalogue lives in `bookx-library.db` and moves an old
//!   `bookx-library.json` into it once ([`SqliteStore::import_json_catalog`]).
//! * [`SqliteStore::import_calibre`] reads a Calibre `metadata.db`; [`export_calibre`] writes one.
//! * Collections, reading status, online backup / restore and integrity checks.
//! * [`LibraryRegistry`] lists several named libraries.
//! * [`guard`] (single-writer lock, network/synced folder detection), [`health`] (rolling snapshots,
//!   restore, library check with drift) and [`provenance`] (where each field came from).

mod calibre;
mod calibre_custom;
mod calibre_export;
mod catalog;
mod custom;
pub mod formats;
pub mod guard;
pub mod health;
pub mod notes;
pub mod provenance;
mod registry;
mod store;
mod works;

pub use calibre::{import_calibre_into, CalibreImport, ImportReport, Stage};
pub use calibre_export::{
    check_target as check_calibre_export_target, export_calibre, export_calibre_with_columns,
    language_to_calibre, CalibreExport, CALIBRE_USER_VERSION,
};
pub use catalog::{
    has_library, library_db_path, open_library, CatalogWatcher, OpenedLibrary, SqliteCatalog,
    LEGACY_JSON, LIBRARY_DB,
};
pub use registry::{LibraryEntry, LibraryRegistry};
pub use store::{
    is_fast_query, Collection, JsonImportReport, Order, ReadingStatus, SqliteStore, TextHit,
    SCHEMA_VERSION,
};
pub use works::WorkLink;
