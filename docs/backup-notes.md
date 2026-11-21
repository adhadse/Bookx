# Backup destinations and secret storage: internal notes

Status of enhancement 7 (backup and restore UI, destinations, encryption) and of the secret storage work.
**Status: D9 is implemented end to end (engine, CLI, app UI, scheduling, wiki).**
User-facing text is in `wiki/docs/backup-and-restore.md`; this file has the decisions, caveats and what is untested.

**Read D9 first** (the incremental repository: book files, covers, catalogue and a notes journal). It supersedes D8
("catalogue only") and the "whole-file, not incremental" decision below; the old whole-catalogue `.db` /
`.bookxbackup` files described in the rest of this file still exist, are still listed and restorable, and coexist with
the repository in the same place.

## What exists

| Part | Where | Tests |
|---|---|---|
| Destinations (`Target` trait: folder, WebDAV, in-memory), `.bookxbackup` encryption, push with verify-after-write and retention, fetch/decrypt, places (GVfs, `/proc/mounts`, iCloud folder), destination settings and "what is due" | `crates/bookx-backup` | 29 unit tests (crypto round trips and tamper cases, verify mismatch, cancel, retention, names, due rules, gvfs names) |
| WebDAV `put_replace`, `get_file`, `delete_file`, `httpc::request_limited` (answers over 8 MB) | `crates/bookx-serve/src/webdav.rs`, `httpc.rs` | one test with a 9 MB file against the in-process WebDAV server |
| `Vault` (SecretStore trait, `MemoryStore`, `Unavailable`, `Oo7Store`), load/persist/migrate | `crates/bookx-secrets` | 6 unit tests |
| Config integration: passwords out of `config.json`, migration of plain text, fallback | `src/config.rs` (`secret_fields`, `from_text`, `file_text`), `src/secrets.rs` | 5 + 2 unit tests |
| App glue: push to all places, outcomes, schedule, words | `src/backups.rs` | 6 unit tests |
| Window: places list with estimates and statistics, add/edit dialog (book/cover/size switches), progress and cancel (also in the Jobs panel), restore dialog with preview, check and clean-up, timers (daily copy, notes every 5 minutes), exit copy | `src/ui/backup.rs`, `src/ui/health.rs` | UI scenarios (below) |
| CLI `library backup --to`, `snapshots --from`, `restore --from` and restore of a `.bookxbackup` path | `crates/bookx-cli/src/libtools.rs` | `crates/bookx-cli/tests/backup.rs` (4 tests) |

UI scenarios in `build-aux/ui-tests.sh`: `backup_destinations`, `backup_restore_from_destination`,
`backup_restore_encrypted`, `backup_remove`, `backup_webdav` (a Python WebDAV server, `build-aux/fake-webdav.py`, with
basic auth), `backup_webdav_wrong_password`, `backup_on_exit`, `secrets_migrate`, `secrets_in_keyring`,
`secrets_no_keyring`. `BOOKX_SECRETS=memory` is set for every scenario (and the session tests); `none` pretends there
is no keyring.

Dev hooks (`library:health=…`; the newer ones are listed in `_developer/docs.md`): `backup-add=NAME|PATH[|encrypt][|keep=N][|auto][|nobooks][|nocovers][|maxmb=N]`, `backup-add-dav=NAME|URL|USER|PASSWORD|FOLDER[|encrypt]`,
`backup-passphrase=…`, `backup-remove1`, `backup-reload`, `backup-restore-remote1`, `backup-pick=N`, `backup-cancel`, `backup-scope`
(prints the `backup_scope:` line used by `backup_scope_explanation`), `backup-dump`. The scope hook takes the same pure strings
the Backups panel shows before a run; it does not take a backup.

## Decisions

- **(Superseded by D9.) Wave 7 backup scope (§7 D8): catalogue only, stated before a run.** The Backups panel and the public wiki now say up
  front that a backup contains catalogue metadata, tags, collections, reading progress, bookmarks, highlights, notes and
  the FTS index. It does **not** contain book files or `cover.jpg`, configuration, plugins/secrets, `.bookx/` state, the
  trash, device copies or caches. Book files and covers need the user's normal document backup; a restore needs the
  library folder pointed at those files. Do not add book files to `.bookxbackup` in this wave.
- **(Superseded by D9 for the repository; still true for the old `.bookxbackup`/`.db` files.) Wave 7 copy is whole-file, not incremental.** A Bookx backup is a snapshot of the catalogue file. Bookx skips a place
  when the snapshot has not changed, skips a copy a place already has, prunes by retention and reads every copy back for
  a SHA-256 check. The UI must not claim deltas, deduplication or incremental transfer. `backup_scope_explanation`
  checks the exact copy and `a11y-audit:` checks the scope row is named.
- **One crate for destinations, one for secrets.** `bookx-backup` depends on `bookx-store` (snapshots, retention)
  and `bookx-serve` (WebDAV client); the CLI and the app share it. `bookx-secrets` is separate so it can be used
  without the backup code and keeps `oo7` (zbus) out of the other crates.
- **Cloud drives are folders.** GNOME Online Accounts mounts Google Drive and OneDrive (and Nextcloud) through GVfs;
  `$XDG_RUNTIME_DIR/gvfs/<scheme>:…` is a FUSE folder. A backup is an ordinary file copy into it, so nothing Google or
  Microsoft specific is in Bookx, and Bookx never sees a token. Detection is by directory name
  (`google-drive:`, `onedrive:`, `dav:`, `smb-share:`, `sftp:`, `nfs:`…), see `places::gvfs_mount`.
- **iCloud**: no public API, no GVfs backend. Only an iCloud Drive folder that a desktop client keeps in sync
  (`~/iCloud Drive`, `~/iCloudDrive`, `~/iCloud` are looked for). Documented as unsupported otherwise.
- **SMB/NFS**: mounted through `gio::File::mount_enclosing_volume` with a `gtk::MountOperation` (the password dialog is
  GNOME's), then the FUSE path is used. If gvfs-fuse is not running the share can be mounted but not used, and the
  window says so. Mounts are not kept across sessions; reconnecting asks again (GNOME's own credential saving applies).
- **Crypto**: Argon2id (19 MiB, 2 passes, 1 lane, RFC 9106 interactive profile; parameters live in the header) +
  ChaCha20-Poly1305 STREAM (`EncryptorBE32`, 64 KiB chunks, header as associated data, last chunk flagged). Chosen over
  `age` because the file must carry readable metadata of its own and age's passphrase mode (scrypt) cannot; chosen over
  AES-GCM because it is constant time without hardware support and has no nonce-misuse cliff in this construction
  (random 56-bit stream nonce per file, a fresh salt per file, so a key is never used twice). Crates: `chacha20poly1305`
  0.10, `argon2` 0.5, `sha2` 0.10, `getrandom` 0.2 (all RustCrypto, pure Rust). A file asking for more than 1 GiB of
  Argon2 memory is refused (a hostile file cannot exhaust memory). Not audited by us.
- **Verify-after-write** is a read-back and SHA-256 compare of the file as stored (ciphertext for encrypted copies). It
  does not decrypt/open the database again. A mismatch deletes the copy and fails the push.
- **Retention per place** reuses `health::retention` (manual never removed, newest N automatic + one per day for 7
  days); only files with Bookx's own names are listed or deleted, so a shared folder is safe.
- **Scheduling**: on exit (every enabled place whose last good copy is older than 10 minutes and older than the newest
  snapshot, with a 20 s ceiling for the whole thing) and a 10 minute timer that copies once a day (a failed place is
  retried at most hourly). The pushed file is the newest non-"before a restore" snapshot; the timer first takes an
  automatic snapshot if something changed. Back up now pushes a fresh manual snapshot.
- **Secrets**: `oo7` 0.5 (0.6 and later need a newer rustc than the pinned 1.85 floor allows). Features
  `async-std` (+ `async-io::block_on`, no tokio) and `native_crypto`. It handles the Flatpak portal itself
  (`ashpd::is_sandboxed`). The vault remembers what it stored, so saving the settings only talks to the keyring about
  changed values; clearing a password deletes the keyring item; removing a destination calls `Vault::forget`. Config keeps
  plain strings in memory (all existing UI code unchanged) and strips them only in the copy that is written to disk.
  On a store failure the value stays in the file (never lost) and the dialogs' caption says so (`secrets::caption`).
  Keys: `share-password`, `email-password`, `kosync-password`, `device-webdav-password`, `backup-passphrase`,
  `backup-<id>-password`.

## D9: the incremental repository (supersedes D8)

Owner decision: *"a user expects his books, with notes/highlights, to be kept in a backup; he/she won't manually copy
the library folder to an online drive every time he adds a book. Also maintain incremental backups of the
highlights/notes/bookmarks managed by Bookx."* Implemented in `crates/bookx-backup/src/repo/`; CLI
`bookx-cli backup {plan,run,journal,list,restore,verify,gc}` (`crates/bookx-cli/src/backup.rs`); app glue in
`src/backups.rs` (`backups::repo`).

### What a backup contains now

Book files, covers, the catalogue (a consistent SQLite online-backup copy) and a readable "notes state" (highlights,
notes, bookmarks, position, progress, status of every book, plus collections). Not included: configuration, plugins and
passwords, `.bookx*` folders (state, trash, old snapshots), device copies, caches, partial/temporary files
(`*.part`, `*.tmp`, `*.crdownload`, `*~`, lock files), the catalogue files themselves (they go in as the consistent
copy), symbolic links (never followed), any folder that holds a `bookx-repo.json` (a repository chosen inside the
library does not back itself up). Per place: `include_books` (default on, also for old configs), `include_covers`
(default on), `max_file_mb` (0 = none; bigger files are listed as "too big", never silently dropped).
Names a snapshot could not carry safely (reserved Windows device names such as `con.epub`, control characters,
non-UTF-8, over-long) are *reported* as left out, never dropped silently.

### Repository layout (format 1)

A folder `Bookx Backup/` inside the place (folder/drive/share/cloud-drive mount; for WebDAV below the configured
folder). Old `.db`/`.bookxbackup` files stay in the place root; their listing ignores the repository folder.

```
Bookx Backup/
  README.txt                        what this is; how to get a book out by hand (plain places)
  bookx-repo.json                   {format, repo id (32 hex), created, app, encrypted, kdf{argon2id,mem,passes,lanes,salt}, check}
  blobs/<aa>/<id>                   content-addressed file contents (id = 64 hex)
  snapshots/s<seq:8>-<UTC time>-<auto|manual>.snap   manifests, written last
  journal/j<seq:10>-<UTC time>.jrn  small records of changed notes since the previous record
  gc/pending.json                   unused blobs and when they were first seen unused (two-phase gc)
```

* **Blob id.** Plain places: `SHA-256(content)` and the blob *is* the original file, so a user can recover a book by
  hand: open the newest `snapshots/*.snap` (JSON), take `files[i].blob`, copy `blobs/<aa>/<blob>` to `files[i].path`.
  Encrypted places: `HMAC-SHA256(id key, content)`; the id key is derived from the passphrase, so deduplication works
  inside the repository but the server cannot confirm that it holds a known file (no SHA-256 of a famous e-book matches
  an id), and two repositories with different passphrases share no ids. (Residual leak, documented: equal ids mean equal
  contents and sizes are visible to the server, as in every dedupe store; file names, titles and counts are not.)
* **Manifest** (`Manifest`): `format, repo, seq, created, kind, app, books, catalogue{blob,size}, state{blob,size},
  journal_seq, include_books, include_covers, too_big[], files[{path, blob, size, mtime, role}]`. Written last, after
  every blob it names exists and was read back; a crashed or cancelled run leaves only unused blobs. Plain places store
  it as JSON, encrypted ones sealed.
* **Journal record**: `{format, repo, seq, at, books[BookNotes…]}` where each `BookNotes` is the *complete* notes state
  of one book that changed (so records are idempotent and replay is "latest record per book wins"). Derived by diffing
  the catalogue's notes against fingerprints kept locally: the catalogue has no change log for annotations (the oplog is
  an undo log), so the diff is the source. `journal_append` costs one catalogue read and, when something changed, one
  small put (a few hundred bytes per changed book). A snapshot records `journal_seq` = the last record its catalogue
  already contains; records at or below the oldest kept snapshot's number are compacted away by retention.
* **Encryption** (`seal.rs`, `crypto.rs`): Argon2id (RFC 9106 interactive profile, parameters in `bookx-repo.json`, a
  file asking for more than 1 GiB or silly passes/lanes is refused before any work) gives a master key; HMAC-SHA256
  derives an object key and an id key. Every object (blob, manifest, journal record, `gc/pending.json`, the passphrase
  check) is `BOOKXOBJ` · version · salt(16) · nonce(7) · ChaCha20-Poly1305 STREAM chunks of 64 KiB. Each object has its
  own key `HMAC(enc key, "obj" ‖ salt)`, so random nonces can never collide across objects under one key (the existing
  `.bookxbackup` format used a fresh Argon2 salt per file; per-object subkeys give the same guarantee without paying
  Argon2 per blob). The header and the object's repository path are the associated data of every chunk: an object copied
  under another name, a swapped header, reordered, truncated or flipped chunks fail authentication. No compression, so
  no decompression bomb. The key never leaves memory and is zeroized on drop.
* **Verify-after-write.** After each object is written it is read back and its SHA-256 compared with the bytes that were
  sent (streaming both ways, never whole-file in memory); a mismatch deletes the object and fails the run. The content
  hash computed while sending must equal the blob id, otherwise the file changed while being copied: the blob is
  removed and that file is left out and reported (the run goes on). A source that cannot be read is also left out and
  reported; a *place* error (full, refused, unreachable) stops the run.
* **Atomic puts.** Folder: write `.<name>.part-<rand>`, fsync, rename (a FUSE mount that refuses rename-over gets
  remove + rename). WebDAV: PUT to `.<name>.part-<rand>` then `MOVE` (probed once per run with a tiny file); a server
  without `MOVE` gets a direct PUT (read back and checked; a cancelled direct PUT is deleted best-effort, so
  *mid-file cancel is only partly guaranteed there*). Leftover `.part-` files are removed by `gc`.
* **Efficiency.** Streaming SHA-256/HMAC (256 KiB reads); per-file cache `(path, size, mtime_ns) → id` in
  `<library>/.bookx/backup/<repo id>.json` (0600), not trusted for files modified in the last 2 s (racy timestamps) and
  dropped when the key fingerprint differs; 1-8 worker threads (default 3) for hashing and for uploading; the place's
  blob folders are listed lazily, one `PROPFIND`/`read_dir` per prefix used. New files are read twice (hash, then
  upload) because the object name needs the hash first; that doubles local reads of *new* data, nothing else.
  Progress callback: phase, files and bytes done/total, current file, ETA. Cancel is checked between 64 KiB blocks
  everywhere (hashing, copying, restoring), not only between files.
* **Skip-if-up-to-date.** A run whose snapshot would equal the newest one (same files, blobs, catalogue and notes
  blobs, same scope) writes nothing. The catalogue copy is byte-identical when nothing changed (tested), so the
  comparison is exact.
* **Space guard.** `plan()` and `run()` compare what would be sent (including encryption overhead) plus a margin with the
  place's free space (folder: `statvfs`; WebDAV: quota properties when the server reports them) and fail with
  "not enough space" before sending anything.
* **Retention + GC.** `prune(keep, now)` applies the existing `health::retention` (manual never removed; newest N
  automatic plus one per day for 7 days) to snapshots and compacts the journal. `gc(grace)` is two-phase: a blob no
  kept snapshot uses is first only *noted* in `gc/pending.json`; a later call removes it when it has been unused for
  `grace` (default 24 h), so a blob that a snapshot still being written is about to use is never taken. `gc` refuses to
  delete anything when any snapshot cannot be read (the references would be unknown). Writers: **one at a time per
  repository**; two computers backing up into the same folder is not supported (use one folder each).
* **Restore.** `Repository::restore(snapshot|latest, target, RestoreOptions, progress, cancel)`; modes `MissingOnly`
  (default; never touches an existing file or catalogue) and `ReplaceAll` (replaces differing files, lists those whose
  local copy was newer in `replaced_newer`, snapshots the old catalogue as "before a restore" first); `dry_run`
  reports everything and changes nothing; `until` picks the newest snapshot at or before a time and replays the journal
  up to it; `min_seq` refuses a snapshot older than the newest this library saw. Each file is written to a temporary
  name in the destination folder, checked against its id and size, given its recorded modification time, then renamed
  (so a failed file leaves nothing behind). One bad file does not stop the others (`failed` lists them); a damaged
  catalogue stops the restore before the target gets one.
* **Rollback.** The library remembers the newest snapshot number it has seen per repository
  (`LocalState.last_seen_seq`). A run that finds the place *older* (snapshots deleted, an old copy restored over the
  place) fails with `RolledBack` unless `accept_rollback`; numbers are never reused. A snapshot copied over a newer name
  is refused by content (encrypted: authentication fails because the name is part of the associated data; plain: the
  manifest's number and time must equal the name's). Not detectable: a brand-new computer with no local state, and
  unencrypted places are not tamper-evident beyond hash checks (anyone who can write the place can write a consistent
  repository).
* **Backward compatibility.** Nothing is migrated: the repository is created next to the old files on the first run,
  which uploads everything (`plan()` shows the size first; for an empty place it counts file sizes without hashing and
  says `estimated`). `list_backups`/`push`/`fetch_to_file` and the `library backup/snapshots/restore` CLI are unchanged.
  Existing `config.json` destinations load unchanged and get `include_books = include_covers = true`, `max_file_mb = 0`.

### Public API (`bookx_backup::repo`)

`Repository::{open, create, open_or_create, exists, is_encrypted_at}`; `plan(lib, scope, jobs, cancel, progress) -> Plan`;
`estimate_new(lib, scope, encrypted, free) -> Plan` (no repository needed); `run(lib, &RunOptions, progress, cancel) ->
Report`; `journal_append(lib, now, cancel) -> JournalAppend`; `journal_replay(store, after_seq, until) -> ReplayReport`;
`list_snapshots()`, `manifest(&SnapName)`, `find_snapshot(name|latest, until)`, `list_journal()`;
`restore(snapshot, target, &RestoreOptions, progress, cancel) -> RestoreReport`; `verify(deep, jobs, cancel, progress)
-> VerifyReport`; `remove_blobs(ids)`; `prune(keep, now)`; `compact_journal()`; `gc(grace, now, cancel) -> GcReport`.
Places: `RepoStore` trait with `FolderStore`, `DavStore`, `MemStore` (tests; can go down, corrupt, run out of space,
fail after N puts) and `Destination::repo_store()` / `Destination::scope()`. `LibrarySource::{open, with_store}`.
`bookx-serve` gained streaming `WebDav::{put_stream, get_to, move_file}` and `httpc::request_stream` (any body from a
reader, any `200` answer into a writer, incremental chunked decoding).

Config keys per destination (all optional, defaults keep old files loading): `include_books` (true), `include_covers`
(true), `max_file_mb` (0). Environment for the CLI: `BOOKX_BACKUP_PASSPHRASE`, `BOOKX_WEBDAV_PASSWORD`;
`BOOKX_BACKUP_KDF=test` (cheap Argon2, tests only).

### Security checklist (docs/security.md §8) for this change

* **Untrusted bytes?** Yes: the whole repository (config, manifests, journal records, blobs, `gc/pending.json`) comes from
  a place that may be shared, damaged or hostile, and WebDAV answers come from a server. Budgets: config 64 KiB,
  manifest 128 MiB, 1 000 000 files, path 1 024 bytes / 255 per component / 32 deep, file 1 PiB, total 4 PiB, journal
  record 64 MiB, state 256 MiB, catalogue 64 GiB, every blob read is bounded by the size the (validated) manifest names
  (plus encryption overhead), restore limits `max_files`/`max_bytes` and free-space check, Argon2 memory ≤ 1 GiB; JSON
  nesting is bounded by serde_json's recursion limit; no decompression anywhere. Bad-input tests: `a_hostile_manifest_cannot_write_outside_the_target`,
  `hostile_paths_are_refused`, `a_manifest_must_agree_with_its_name_and_repository`,
  `tampered_truncated_and_missing_blobs_are_caught_for_both_kinds_of_place`,
  `a_blob_bigger_than_the_snapshot_says_is_not_read_to_the_end`, `hostile_repository_settings_are_refused_without_doing_the_work`,
  `a_damaged_catalogue_stops_the_restore_and_leaves_the_target_without_one`, `restore_limits_are_enforced_before_writing`,
  seal tests (flip/cut/rename at every region and chunk boundary). No fuzz target yet (the parsers are serde + the
  AEAD framing; a `cargo-fuzz` target for `Crypto::open_bytes` and `Manifest` is a follow-up).
* **Path from a file or the network?** Yes. Every path in a manifest goes through `validate_rel` (relative, `/` only, no
  empty/`.`/`..`, no hidden component so never `.bookx*`/`.ssh`/`.git`, no control characters, no backslash or drive
  letter, no reserved device names, no `bookx-library.db*`, length/depth limits, no duplicates) when the manifest is
  loaded and again before use. `prepare_dest` creates folders one component at a time with `symlink_metadata`: an existing
  link or file in the way is an error, nothing is written through a link; files are written to a temporary name in the
  destination folder and renamed (rename replaces a link instead of following it). Repository paths (`blobs/ab/<id>` etc.)
  are checked by `valid_path` (≤ 3 segments of `[A-Za-z0-9._-]`) before they become a file name or a URL, blob ids must be
  64 lower-case hex, and `FolderStore` refuses links in the repository folder (get, put parent, list). Tests:
  `a_link_in_the_target_is_never_followed_on_restore`, `links_in_a_hostile_repository_folder_are_not_followed`,
  `only_repository_paths_are_accepted`. The path is joined, not canonicalised, then opened by the OS; the check is
  component by component so a link planted between check and use by another local process is the one remaining race
  (same trust level as the user's own account).
* **Executes something?** No. No shell, no `PATH` search, no process.
* **Listener or user-supplied URL?** No listener. The WebDAV address is user-typed in the existing destination dialog
  (same SSRF position as before: the user chooses the server; credentials in their own fields, `http://` allowed only
  because the existing dialog allows it; `Authorization` is only sent to that host; redirects are not followed).
* **New dependency?** `hmac 0.12`, `subtle 2`, `zeroize 1` (RustCrypto/dalek-crypto ecosystem, MIT/Apache-2.0, already
  in the lock file through `chacha20poly1305`/`argon2`/`rustls`). Justification: HMAC for keyed ids and sub-keys,
  constant-time id comparison, key wiping. `NOTICE.md` lists them with the other RustCrypto crates (check at release).
* **Secret?** The passphrase comes from the keyring (`Secrets`, never `Debug`-printed, never written to config, logs,
  errors, manifests or the local state; the state holds only a short public fingerprint of the id key). Keys are zeroized
  on drop. The cached ids in `.bookx/backup/*.json` of an encrypted place are keyed hashes, not secrets, and the file
  is `0600`. WebDAV passwords as before (keyring).
* **Widens a capability?** Yes: Bookx now copies the user's book files (not only the catalogue) to places the user chose,
  including network places; and a restore can *write book files into a folder*. The write is contained to the chosen
  target by the checks above, never executes anything and never overwrites without `ReplaceAll`. The capability
  statement in `docs/security.md` should be updated by the reviewer (not done here). Needs a second reviewer.
* **Regression tests** listed above; each exercises the check it is named for (not run against a version without the
  check: that would need the checks removed; the replay, traversal and link tests assert on the specific refusal).

### Tests

`crates/bookx-backup`: unit tests (seal, manifest/paths, store, source walk and cache, settings) and
`tests/repo.rs` (30 end-to-end tests: first run / no-op / incremental / rename, restore equality, journal and point in
time, compaction, encryption and keyed ids, tamper/truncate/missing/oversize for plain and encrypted places, hostile
manifest and links, replayed manifest and rollback, cancel and resume, failing and corrupting places, space guard,
scope and size cap, restore modes and dry run, retention and two-phase gc, gc safety, `.part-` cleanup, legacy
coexistence, repository inside the library, WebDAV with and without `MOVE` and cancel mid-file, hostile settings, files
changing while copied, cache use, unreadable files); `tests/perf_budgets.rs`. `crates/bookx-cli/tests/backup_repo.rs`
(folder, encrypted folder, WebDAV, journal, usage errors) via the in-process `run`. `src/backups.rs`: glue tests.
Performance (release, this machine): seal + open 32 MiB 65 ms; unchanged library of 4 000 files (cache hits) no-op run
27 ms, first run 38 ms (in-memory place); 200 000-file manifest serialise + parse + validate 110 ms. Budgets: 1.5 s,
1.5 s / 8 s, 2.5 s (debug x30).

### Limits and caveats (D9)

* One writer per repository; no locking in the place (a second writer could reuse a snapshot or journal number; the
  rollback check and per-run listing make it detectable, not impossible).
* WebDAV: not run against a real Nextcloud/ownCloud; tested with an in-process stand-in (with and without `MOVE`). The
  Python `build-aux/fake-webdav.py` has no `MOVE`, which exercises the direct-PUT fallback (mid-file cancel is then
  best-effort). Uploads are one `Connection: close` request per object, so many small files over a high-latency link
  are slow (3 in flight by default); a server limit on request size (e.g. 100 MB on some proxies) will fail big files
  with a clear "server answered 413"; chunked upload is not implemented.
* Verify-after-write reads every uploaded object back: it doubles traffic of the first backup over a network place
  (`RunOptions::verify_writes = false` to skip; the UI should offer that only for fast links).
* A new file is read twice (hash, upload). Files that change *while* being copied are left out and reported, then taken
  next run.
* The manifest of a very large library (hundreds of thousands of files) is read and written in memory (~150 bytes per
  file); 1 000 000 files is the hard limit.
* `ReplaceAll` restoring a library the app has open is the caller's responsibility (the CLI takes the library lock; the
  UI must close the library first). The catalogue is replaced through SQLite's online-backup API, so the open file is
  consistent, but the app's in-memory books are stale.
* Collections are in the notes-state blob and the catalogue, but *journal records do not carry collections* (only
  highlights, notes, bookmarks, position, progress and reading status): a collection created between snapshots is
  protected at the next snapshot.
* Journal replay never creates or deletes books: notes of a book the restored catalogue lacks are counted
  (`journal_books_missing`), not recreated.
* Passphrase change: a repository keeps the passphrase it was created with; changing it needs a new repository
  (a different folder), as with `.bookxbackup`.
* Windows/macOS file-name rules: names Windows cannot hold (reserved device names) are skipped with a report; names with
  `:` or trailing dots are kept (restoring on Linux/macOS is fine).
* The CLI `backup run` takes no library lock (it only reads the catalogue through SQLite's backup API, safe while the app
  runs); `backup restore` into an existing library takes it and refuses when another Bookx holds it.

### The Backups UI (done)

Wired in `src/ui/backup.rs` (page, place rows, dialogs, scheduling), `src/ui/health.rs` (page text, Back Up button) and
`src/backups.rs` (`backups::repo`: pure helpers with unit tests). Decisions:

* **The first copy of a place never runs by itself.** `Destination.auto_ok` (settings key `auto_ok`, default false, so
  every place from before the update starts at false): the scheduled copies (`scheduled_places`: on close, once a day)
  and the notes timer (`journal_places`) skip a place until `auto_ok`. It becomes true when a repository run to
  the place succeeded (`apply_repo`) or when the user switches on *Make the first copy automatically* in the place's
  settings after seeing the estimate. The page shows a one-time row, *Include your books in backups*, with the size of
  the first copy and a Back Up button, while a place waits (`banner_text`). A failed or cancelled first run does not set it.
* **Estimate before a run**: every place row shows `plan_text` (first copy size or what the next copy adds) loaded by a
  worker per place when the page opens/refreshes (no repository: file sizes only; repository: the plan with the cache).
  *Not enough free space* and *N files are left out* are part of that text; they do not block anything.
* **Progress**: one operation at a time (`busy`), shared by Back Up, Check, Remove Old Backups and Restore: the window shows
  stage (`phase_label`), bar (`progress_fraction`), files/bytes/time left (`progress_detail`) and Cancel; the same operation
  is a cancellable `jobs::Kind::Backup` job in the Jobs panel (Cancel there reaches the worker through `watch`). Scheduled
  runs are jobs too. Failures stay per place; the message after a run is `outcomes_text`.
* **Place folder that does not exist yet** is created by the first backup when its parent exists (`will_be_created`);
  a missing parent (drive not plugged in) is never created. Looking at a place (estimate, summary) never writes
  (`check_readable`; the engine's `check()` creates the folder and a probe file).
* **Local snapshot**: Back Up still takes the catalogue snapshot on this computer (`.bookx-backups`, used by the
  *Restore from a backup* list and "before a restore" safety); the repository run follows. Manual runs make manual snapshots
  (never pruned); scheduled ones are automatic (pruned by *Automatic backups to keep here*).
* **Scheduling**: on close `run_due_repo_within` (20 s ceiling for everything) then `journal_within` for the places not
  just copied with what is left of the 20 s; once a day `tick` (every 10 minutes it looks, copies when a place is 24 h
  old; a failed place is retried at most hourly); `journal_tick` every 5 minutes (`JOURNAL_EVERY_SECS`) records changed notes
  at every enabled, allowed place, quietly (one toast per failure streak). Nothing runs when the library is read-only.
* **Restore UI**: `Restore` on a snapshot opens a window (*Restore into*: this library or another folder; *What to restore*:
  replace everything or only the missing books, replace chosen by default when the library is empty); *Preview* runs the dry
  run and prints `N files to restore, M already present` plus catalogue and newer-file warnings; *Restore* is enabled only
  after a preview with the current choices. Naming a snapshot (not "the newest") skips the engine's rollback check, so an
  older point can be chosen on purpose. Restoring into the open library reloads it (`reload_library`); into another folder
  the message says where it is. A missing-only restore never touches the catalogue, so on a new computer with an empty
  library *replace* (the default then) is what brings the catalogue back. Legacy `.db` / `.bookxbackup` rows stay
  (marked "catalogue only") with the old restore path.
* **Check Backup / in Depth / Remove Old Backups** are in a ⋯ menu per place (`check_place`, `clean_place`: prune to the
  place's keep setting, compact the journal, gc with no grace, confirmation first).

Tests: `src/backups.rs` unit tests (first-copy rule, progress, banner, messages, check/clean, restore text, scope label,
folder creation); Broadway scenarios `backup_scope_explanation`, `backup_destinations`, `backup_incremental`,
`backup_catalogue_only`, `backup_estimate_and_banner`, `backup_old_place_waits`, `backup_restore_new_folder`,
`backup_restore_missing_only`, `backup_restore_dialog`, `backup_restore_from_destination`, `backup_restore_encrypted`,
`backup_journal`, `backup_check_and_clean`, `backup_cancel`, `backup_on_exit`, `backup_webdav*`, `health_backup_restore`.

Untested here (needs real hardware/services): a real Nextcloud/ownCloud (per-object upload limits, MOVE support, quota
reporting), a NAS over SMB/NFS through GVfs (FUSE rename-over-existing, free space), Google Drive / OneDrive mounts,
a removable drive being pulled out in the middle of a run, the 20 s close limit against a network place that hangs, the
real restore dialog by hand (the Broadway scenarios drive it through the hooks, with the real widgets), the Jobs panel
look during a long copy, and the wiki screenshot of the Backups page (needs the session screenshots to be retaken).
The `i18n` catalogues (`po/`) were not updated: see the msgid list in the change report.

## Not done, or only partly

- **Real services**: not run against Google Drive, OneDrive, iCloud, a real SMB/NFS share, Nextcloud/ownCloud, or a real
  Secret Service session (GNOME Keyring, the Flatpak portal). The routes are covered with local folders, a Python WebDAV
  server and the in-memory vault. The `Oo7Store` code compiles and follows the oo7 API but has never talked to a keyring
  in a test; if the session has none, `open_default` falls back to `Unavailable`.
- **Streaming** (old `.bookxbackup` path only; the repository streams, see D9): backups are read fully into memory (`Vec<u8>`) to encrypt, hash and PUT. WebDAV downloads are capped at
  2 GB and uploads are one request. Fine for catalogues up to a few hundred MB; a chunked upload would be needed beyond.
- **Progress** is by stage, not by byte; cancel is checked between stages. A cancel during a long PUT takes effect after it.
- **Unit-tested GTK parts**: the add/edit dialog, `mount_uri` and "Online Accounts…" are only exercised by hand (not by
  the UI scenarios); the scenarios use the dev hooks that bypass the dialog. The Cancel button is only unit-tested at
  the `push` level.
- **Passphrase change** does not re-encrypt existing copies.
- **Windows/macOS**: nothing here is portable beyond folders and WebDAV; GVfs and the Secret Service are Linux.
- The E-mail/Share/KOSync/Device dialogs got a keyring caption; the Device window's WebDAV password shows it in the status
  line (it has no description field).

## Sync

Out of scope; see `docs/library-sync-design.md`.
