#![forbid(unsafe_code)]
//! Off-site backups of the Bookx catalogue.
//!
//! A backup is a **one-way snapshot**: the catalogue (titles, tags, reading progress, notes) is
//! copied to one or more *destinations* and can be restored from any of them. It is not a sync:
//! nothing is merged, nothing flows back on its own. See `docs/library-sync-design.md` for what
//! real multi-device sync would need.
//!
//! * [`target`]: the places (a folder, which also covers drives, network shares and GNOME Online
//!   Accounts mounts; a WebDAV folder) behind one small trait, plus an in-memory one for tests.
//! * [`crypto`]: the `.bookxbackup` file, a passphrase-encrypted snapshot.
//! * [`places`]: finding and describing mounted drives, shares and cloud drives.
//! * [`push`]: writing a snapshot to a place with verify-after-write and per-place retention,
//!   restoring from a place, and what is due when.
//! * [`settings`]: the destinations the user configured.
//!
//! * [`repo`]: the incremental repository (book files, covers, catalogue, notes journal; decision D9).
//!
//! Snapshots themselves (SQLite online backup, local retention) stay in `bookx_store::health`.

pub mod crypto;
pub mod places;
pub mod push;
pub mod repo;
pub mod settings;
pub mod target;

pub use push::{
    Cancelled, Progress, PushOptions, PushReport, PushStage, Summary, fetch_to_file, list_backups,
    open_file, prune, push, summarize,
};
pub use settings::{BackupSettings, DestKind, DestStatus, Destination, Trigger};
pub use target::{
    FolderTarget, MemTarget, RemoteFile, RemoteName, Target, WebDavTarget, parse_remote_name,
    remote_name,
};
