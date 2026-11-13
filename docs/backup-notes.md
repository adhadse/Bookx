# Backup destinations and secret storage: internal notes

Status of enhancement 7 (backup and restore UI, destinations, encryption) and of the secret storage work.
User-facing text is in `wiki/docs/backup-and-restore.md`; this file has the decisions, caveats and what is untested.

## What exists

| Part | Where | Tests |
|---|---|---|
| Destinations (`Target` trait: folder, WebDAV, in-memory), `.bookxbackup` encryption, push with verify-after-write and retention, fetch/decrypt, places (GVfs, `/proc/mounts`, iCloud folder), destination settings and "what is due" | `crates/bookx-backup` | 29 unit tests (crypto round trips and tamper cases, verify mismatch, cancel, retention, names, due rules, gvfs names) |
| WebDAV `put_replace`, `get_file`, `delete_file`, `httpc::request_limited` (answers over 8 MB) | `crates/bookx-serve/src/webdav.rs`, `httpc.rs` | one test with a 9 MB file against the in-process WebDAV server |
| `Vault` (SecretStore trait, `MemoryStore`, `Unavailable`, `Oo7Store`), load/persist/migrate | `crates/bookx-secrets` | 6 unit tests |
| Config integration: passwords out of `config.json`, migration of plain text, fallback | `src/config.rs` (`secret_fields`, `from_text`, `file_text`), `src/secrets.rs` | 5 + 2 unit tests |
| App glue: push to all places, outcomes, schedule, words | `src/backups.rs` | 6 unit tests |
| Window: places list, add/edit dialog, progress and cancel, restore from a place or file, timer, exit copy | `src/ui/backup.rs`, `src/ui/health.rs` | UI scenarios (below) |
| CLI `library backup --to`, `snapshots --from`, `restore --from` and restore of a `.bookxbackup` path | `crates/bookx-cli/src/libtools.rs` | `crates/bookx-cli/tests/backup.rs` (4 tests) |

UI scenarios in `build-aux/ui-tests.sh`: `backup_destinations`, `backup_restore_from_destination`,
`backup_restore_encrypted`, `backup_remove`, `backup_webdav` (a Python WebDAV server, `build-aux/fake-webdav.py`, with
basic auth), `backup_webdav_wrong_password`, `backup_on_exit`, `secrets_migrate`, `secrets_in_keyring`,
`secrets_no_keyring`. `BOOKX_SECRETS=memory` is set for every scenario (and the session tests); `none` pretends there
is no keyring.

Dev hooks (`library:health=…`): `backup-add=NAME|PATH[|encrypt][|keep=N]`, `backup-add-dav=NAME|URL|USER|PASSWORD|FOLDER[|encrypt]`,
`backup-passphrase=…`, `backup-remove1`, `backup-reload`, `backup-restore-remote1`, `backup-pick=N`, `backup-cancel`, `backup-dump`.

## Decisions

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

## Not done, or only partly

- **Real services**: not run against Google Drive, OneDrive, iCloud, a real SMB/NFS share, Nextcloud/ownCloud, or a real
  Secret Service session (GNOME Keyring, the Flatpak portal). The routes are covered with local folders, a Python WebDAV
  server and the in-memory vault. The `Oo7Store` code compiles and follows the oo7 API but has never talked to a keyring
  in a test; if the session has none, `open_default` falls back to `Unavailable`.
- **Streaming**: backups are read fully into memory (`Vec<u8>`) to encrypt, hash and PUT. WebDAV downloads are capped at
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
