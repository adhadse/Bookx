# Changelog

Hand-written, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [semantic versioning](https://semver.org/). `build-aux/release-check.sh` fails when the newest
versioned heading, `Cargo.toml` and the AppStream metainfo disagree.

## [Unreleased]

## [0.3.0-beta.2] - 2026-11-23

### Fixed
- The packages of 0.3.0-beta.1 were built on Fedora 45 and ran only there (glibc 2.44, OpenSSL 4 in the .rpm
  requirements). They are now built on Fedora 43 (glibc 2.42), the .rpm lists only the packages it needs, and the .deb
  requires glibc 2.42.

### Added
- The release pipeline installs the .deb, .rpm and tarball on Fedora 43/44/45, Ubuntu 25.10/26.04, Debian testing and
  Arch and starts the app before it publishes (`build-aux/distro-matrix.sh`).

## [0.3.0-beta.1] - 2026-11-22

The first beta of 0.3.0, published on GitHub Releases only. Bugs are expected; please report them.

### Added
- Report a Problem (Main Menu): a readable log report and a GitHub issue; a beta notice in About.

### Changed
- Release files are named `Bookx-<version>-linux-<arch>.<ext>`; docs install from GitHub Releases first.
- The wiki dark theme uses the same fonts as the light one.

Everything below under 0.3.0 is part of this beta.

## [0.3.0] - 2026-11-22

The first public release. Everything since 0.2.0 (waves 5 to 11), grouped by what you see.

### Added
- Library: custom columns; live updates of highlights and progress while a KOReader sync runs; QR code to open the
  OPDS server from a phone; Wallabag read-later; safe mode for a damaged settings folder; French and Hindi translations
  (Spanish and German were already there).
- Reading: more typography settings and reading aids; CBR and DJVU through helper programs when installed.
- Backups: lock, change password and rekey of a backup repository.
- Plugins: a trust sheet before a plugin is enabled (the signature format exists; verification is not built yet).
- Editor: syntax-highlighted code editing (GtkSourceView 5), editor preferences, shortcuts and snippets, structure
  tools, editor plugins (plugin ABI v2, `editor-action`), narration while editing, a Problems panel, a command palette
  (Ctrl+Shift+P) and a Main Menu with Preferences, Keyboard Shortcuts, Help and About.
- Library: responsive header, jobs panel, a secondary Library menu (Reading Statistics, Vocabulary, Reading Trackers,
  Audiobooks, Online Catalogues, news), a configurable sidebar (Recent, Home, Favourites, Notes, shelves, Saved
  Searches), Move to Trash with Undo, Library Health with a sidebar of pages (Health, Duplicates, Editions, Needs
  Attention, Backups, Trash), reading goals, Add Folder with a count.
- Devices: the Devices window with the covers of the books on the device, a connect banner, an *Always sync when
  connected* switch (off by default) and a summary of what moves in which direction before a sync.
- Sync: Bookx sync (extended KOSync) for KOReader over Wi-Fi with a small KOReader plugin for highlights, notes and
  bookmarks (colour and style included); a Sync window and a Sync page in Preferences with a four-step checklist, an
  OPDS server and a web page for other apps, and the server and login remembered across restarts.
- Backups: off-site, incremental and content-addressed backups of the catalogue, book files, covers and notes to a
  folder, drive, network or cloud-drive mount or WebDAV, with encryption, verify after write, retention and restore;
  passwords are kept in the system keyring.
- Reading: read aloud with natural offline voices (Kokoro and Piper) and voice servers, a player card with media keys;
  audiobooks (folders, M4B, MP3, chapters, speed, sleep timer, bookmarks); a comic reader (CBZ/CBT, two-page, webtoon,
  smart zoom, guided view, right-to-left); vertical Japanese and Chinese text with ruby; a reading ruler; selection
  handles; a Footer Statistics dialog and Preferences group; Previous and Next Position buttons; a bookmark key
  (Ctrl+D) and Go to key (Ctrl+L); undo toasts for removed highlights, bookmarks and vocabulary terms.
- Vocabulary: add a term with your own definition without a dictionary, glossary import and export.
- Notes: a Notes view with filters (highlights, notes, bookmarks), import of highlights and bookmarks, and jump to the
  highlight in the book.
- Preferences: twelve pages with one search that also reads choices, an MCP page for AI assistants with token
  confirmation, a Keyboard Shortcuts page with import, export and Reset All (with Undo), section keys
  (Ctrl+Page Down and Ctrl+Page Up).
- Packaging: `.deb` and `.rpm` metadata (`cargo deb`, `cargo generate-rpm`), `build-aux/release-check.sh`
  (`make release-check`), `build-aux/tarball.sh`, release workflow with checksums.
- Plugin compatibility test with golden ABI v1 and v2 plugins; the ABI stability policy is documented in the plugin
  guide.
- Public wiki (Zensical, `bookx.adhadse.com`) and a diagnostics report in About.

### Changed
- The shared library server is called the **OPDS server** everywhere (Preferences → Sync, the Sync window, the wiki);
  the list of catalogues you browse is *Online Catalogues*.
- Bookx sync is for KOReader only; other e-readers sync over USB. The Sync menu entry and the Preferences page are both
  named *Sync*; the endpoint is `/bookx`.
- The main menu of the library, the readers and the editor are real menus (arrow keys, menu roles) and end with
  Preferences, Keyboard Shortcuts, Help and About; no label ends with an ellipsis.
- The Footer Statistics group in Preferences has one switch per item with Move Up and Move Down, and its item names are
  searchable.
- The empty library shows one sentence and one primary button; the list of formats moved into a tooltip.
- Preferences has twelve pages (*Dictionaries* is a page, *Patches* a group of *Extensions*, E-ink mode is in General,
  the AI page is called *MCP*).

### Fixed
- The Sync server, its address and the login were not remembered after a restart.
- Highlight colours and styles synced with KOReader were lost or mixed up; existing notes can be repaired.
- Many dialogs did not fit a 900x600 window with larger text; the audiobook player, every Preferences page and the
  secondary windows are covered by the accessibility audit.
- Controls without an accessible name, untranslated labels and tooltips, informational rows that Tab stopped on.
- Flatpak: media keys during read aloud did not work because the sandbox did not allow the player to own its
  media-player name; the AppStream screenshots pointed at a branch that does not exist.

### Security
- Updated `lettre` to 0.11.24 (six SMTP advisories, RUSTSEC-2026-0560 to 0565).
- **Passwords and tokens are never written in clear text.** Without a system keyring Bookx used to keep them in the
  settings file; now they go to an encrypted password store (`secrets.vault`: Argon2id + ChaCha20-Poly1305, master
  password asked once per start, never saved) or, if you decline, stay in memory until Bookx closes. Passwords an older
  version left in `config.json` (and its copies) are removed on first start, and the settings folder is made private;
  you may have to type them again. The assistant token is no longer written to `mcp.json`.
- Image-bomb limits for comic pages; archive entries with `..` are skipped; the Sync server stores only a one-way hash
  of the account password and throttles repeated failures; passwords live in the system keyring, not the settings file.
- A dependency policy (`deny.toml`, `cargo deny` in CI) and the security model in `docs/security.md`.
- Links in books open only as `http`, `https` or `mailto` (never `data:`, `file:`, `javascript:`, `ftp:`); one function
  (`safe_external_url`) now gates the reader, the PDF reader and the editor help links.
- The read-aloud and audiobook player overrides (`BOOKX_TTS_PLAYER`, `BOOKX_AUDIO_PLAYER`) no longer go through `sh -c`:
  an absolute program path plus arguments, run directly. The built-in players are looked up in system directories only.
- Fetching a URL that came from a feed, book or redirect refuses loopback, private, link-local and cloud-metadata
  addresses (checked on the addresses actually connected to, so redirects and DNS rebinding are covered) unless you
  configured or typed that host (OPDS catalogues and WebDAV on your LAN keep working); at most 5 redirects.
- The library server, sync routes and MCP endpoint check the `Host` header (DNS rebinding) and refuse cross-origin
  browser requests; list another public name with `--allow-host` or `BOOKX_ALLOWED_HOSTS`.
- Hostile files: an element with thousands of attributes, HTML nested tens of thousands deep, `:not(` nested deeply in CSS,
  and plugin manifests with tens of thousands of permissions can no longer hang or crash the app; backup and repository
  files can no longer ask for gigabytes of key derivation.
- Release files are signed with Sigstore cosign (keyless) and have build-provenance attestations; see "Verify your
  download" in the install guide.

## [0.2.0] - 2026-11-11

Complete rewrite on the latest GTK 4, libadwaita and Relm4: a new library, a Rust-only reader and a built-in EPUB
editor.

### Added
- Library: covers or table view, tag browser, Calibre-style search, metadata editing, bulk edits, TXT / Markdown /
  HTML import and export, SQLite catalogue with full-text search, Calibre import, backups.
- Reader: two-column pages, single column or scrolling, light / sepia / dark / black themes, highlights, notes,
  bookmarks, in-book search, read aloud with natural offline voices.
- Editor: file browser, code editor with live preview, find and replace, table-of-contents and spine editing, book
  checker.
- Send books to Kobo, Kindle and other e-readers and bring reading progress and notes back.
- Sandboxed WebAssembly plugins (ABI v1) and user patches.
- MCP server for AI assistants (read-only by default), OPDS and KOReader sync servers.
- Translated into Spanish, German, French and Hindi.
- `bookx://` links and a diagnostics report for bug reports.
- Flatpak manifest (GNOME 50 runtime, offline crates), CI workflow.

[Unreleased]: https://github.com/adhadse/Bookx/compare/v0.3.0-beta.2...HEAD
[0.3.0-beta.2]: https://github.com/adhadse/Bookx/releases/tag/v0.3.0-beta.2
[0.3.0-beta.1]: https://github.com/adhadse/Bookx/releases/tag/v0.3.0-beta.1
[0.3.0]: https://github.com/adhadse/Bookx/releases/tag/v0.3.0
[0.2.0]: https://github.com/adhadse/Bookx/releases/tag/v0.2.0
