# What real multi-device library sync would take (design note, not implemented)

Bookx **backs up**; it does not **sync a library**. A backup is a one-way snapshot of the catalogue (`docs/backup-notes.md`):
restoring replaces everything. Two computers writing snapshots to one place would overwrite each other's work. This note
lists what would have to exist before "my library on two computers" can be promised.

Status: living | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## The problem

Three kinds of data change on any device, and each needs its own rule:

1. **Catalogue rows** (title, authors, tags, series, rating, collections): small, structured, edited on both sides.
2. **Reading state** (position, finished, highlights, notes, statistics): written constantly, usually on one device
   at a time, but offline on several.
3. **Book files and covers**: large, immutable in practice (an edit makes a new file), content-addressable.

The current catalogue is one SQLite file with `INTEGER` row ids and one-writer locking (`guard.rs`); a network or synced
folder even triggers a warning because two writers corrupt it. File-level sync (Syncthing, Nextcloud client) of that
database is exactly what must not be recommended.

## What it would require

1. **Stable identity.** Row ids are local. Every book needs a UUID (content hash of the file is not enough: an edit changes
   it), every note/highlight its own id, and the catalogue schema a migration for it. (Provenance already records where
   fields came from; that is a start.)
2. **A change log instead of snapshots.** Each device appends operations ("set title of book B to X at hybrid-logical-clock
   T by device D") to a local log; sync exchanges the log entries the other side lacks. Alternatives: state-based CRDTs per
   field (last-writer-wins registers for scalar fields, add-wins sets for tags/collections, an append-only set for
   highlights); or a library like cr-sqlite/Automerge. Per-field last-writer-wins with a hybrid logical clock is enough
   for catalogue rows; deletions need tombstones kept long enough for the slowest device.
3. **Conflict policy that a person can understand.** Same field edited on two devices while offline: the later wins,
   but the loser is kept in history and offered in the Undo list. Reading position: take the furthest-read unless a
   device explicitly says otherwise (the KOSync prompt already does "continue from your other device?").
4. **A transport.** The simplest honest one is a dumb folder or WebDAV that both devices can read and write (the
   `Target` trait already abstracts it): each device writes its own log file and reads the others', never writing a
   file another device writes, so no locking is needed. A server is the alternative (the built-in content server could
   be extended), with auth and TLS already available.
5. **Book file transfer by hash.** A manifest of `(book uuid, file hash, size)`; devices fetch missing files on demand
   (the library on a phone does not need all of them). Layout (`tree`/`flat`/`template`) stays a local choice.
6. **Safety nets that already exist and must stay**: a snapshot before applying remote changes (the "Before a restore"
   mechanism), the history/undo, the integrity check after a merge, and read-only mode when the schema of the other side
   is newer.
7. **Encryption** of the log and files for untrusted places, reusing the `.bookxbackup` construction (Argon2id +
   ChaCha20-Poly1305), with one shared passphrase or per-device keys.
8. **Testing**: a property test that applies the same operations on two replicas in random orders and requires equal
   catalogues; fault injection (cut-off log, replayed log, clock skew); the existing parallel UI-test harness extended
   to two app instances sharing a folder.

## Cheaper things that work today

- Reading position between devices: *Preferences → Sync* (KOSync) with KOReader or another Bookx.
- A library on a NAS opened by **one** computer at a time, with backups to a second place.
- Catalogue to a new computer: restore the latest backup (`wiki/docs/backup-and-restore.md`) and copy the book folder.
- Read-only access from phones: the Share Library (OPDS) server.

## Rough effort

Identity migration (M), operation log + merge for catalogue fields (L), reading state/highlights (M), file manifest and
transfer (M), folder/WebDAV transport (S, existing), conflict UI (M), tests (L). It is a project of its own, not an
extension of the backup window.
