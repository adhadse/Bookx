# Remaining work

The single place for open work on Bookx. Every other document may mention open items; this one is the list that is kept current.

Status: living | Last reviewed: 2026-11-23 | Owner: @adhadse (agents update it in the same change that opens or closes an item)

## Contents

- [How to read this list](#how-to-read-this-list)
- [Summary](#summary)
- [P1: beta.3](#p1-beta3)
- [Release and distribution](#release-and-distribution)
- [Devices and sync](#devices-and-sync)
- [Annotations and history](#annotations-and-history)
- [Reader and editor UX](#reader-and-editor-ux)
- [Data and storage](#data-and-storage)
- [CI and quality](#ci-and-quality)
- [Platforms](#platforms)
- [Docs and translations](#docs-and-translations)
- [Owner-only list](#owner-only-list)
- [Decided, do not ask again](#decided-do-not-ask-again)
- [What was dropped as already done](#what-was-dropped-as-already-done)

## How to read this list

- **Status**: `open`; `in progress` with the pull request or branch; `blocked on owner` (needs an account, a device, a
  decision or a GitHub setting); `unverified` is added when the item was carried over from an older document and not
  re-checked against the code on 2026-11-23.
- **Priority**: **P1** = beta.3, **P2** = before 1.0, **P3** = later.
- **Who**: `agent` = doable in this repository without hardware or accounts; `owner` = needs the owner; `both` = the agent
  builds, the owner verifies.
- **Pointer**: the document or issue with the detail. Documents under `archive/` are history; their open points are
  repeated here and nowhere else.
- When an item is done, delete the row in the pull request that finishes it (the CHANGELOG and `git log` are the record).
  New open work goes here first, never into a new notes file.

## Summary

| Area | Items | P1 | P2 | P3 |
|---|---|---|---|---|
| Release and distribution | 12 | 3 | 5 | 4 |
| Devices and sync | 10 | 2 | 5 | 3 |
| Annotations and history | 4 | 2 | 1 | 1 |
| Reader and editor UX | 11 | 0 | 5 | 6 |
| Data and storage | 10 | 3 | 5 | 2 |
| CI and quality | 9 | 1 | 6 | 2 |
| Platforms | 6 | 0 | 2 | 4 |
| Docs and translations | 7 | 0 | 5 | 2 |
| **Total** | **69** | **11** | **34** | **24** |

## P1: beta.3

| ID | Item | Status |
|---|---|---|
| REL-1 | Rust 2024 edition | done, merged (PR #37) |
| REL-2 | AUR packages `bookx` and `bookx-bin` | merged (PR #36); publishing blocked on owner (AUR account and SSH key) |
| REL-3 | Debian 13 (trixie) support | dropped (owner, 2026-11-23: GTK 4.18 is too old; use the Flatpak) |
| DEV-1 | Issue #39: Boox device names, "sync never copies or deletes books", device zoo emulation tests | merged (PR #42); ships in beta.3 |
| DEV-2 | First sync of KOReader plugin 0.3.1 on the owner's Kobo, with the 7-step live-update checklist | blocked on owner, unverified |
| ANN-1 | History and restore of highlights, notes and bookmarks (SQLite journal) | merged (PR #44); ships in beta.3 |
| ANN-2 | Annotations in their own tables `highlights` and `bookmarks` (design-audit step 3) | open (beta.3 candidate; builds on ANN-1 and DATA-1) |
| DATA-1 | Library update safety: copy first, check after (`bookx-library.db.pre-v<N>-<time>`), and the design audit | merged (PR #41) |
| DATA-2 | Statistics, vocabulary and the sync ledgers into the library database (design-audit step 4) | open (beta.3 candidate) |
| DATA-3 | Rotate passwords that sat in old plaintext config copies; try the master-password dialogs by hand | blocked on owner |
| CI-1 | Flaky UI scenario `win11_scale_library` (`build-aux/ui-tests-win8.sh`) | open |

## Release and distribution

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| REL-1 | Rust 2024 edition (every crate is `edition = "2024"` on `main`) | done, merged (PR #37) | P1 | PR #37 | agent |
| REL-2 | AUR packages `bookx` and `bookx-bin` (PKGBUILDs, tested) | merged (PR #36); publish blocked on owner | P1 | PR #36, `releasing.md` | owner (account, SSH key), agent |
| REL-3 | Debian 13 (trixie): `src/app.rs` uses `adw::ShortcutsDialog` (libadwaita 1.8) which Debian 13 lacks; replace it with a widget available in 1.5 or newer, then add a Debian 13 build job for the .deb (`build-aux/distro-matrix.sh` today covers Debian testing only) | dropped (owner decision 2026-11-23: GTK is too old, Debian 13 uses the Flatpak) |  - | `build-aux/distro-matrix.sh`, `releasing.md` | agent |
| REL-4 | Start the Flatpak bundle by hand once (the pipeline installs and starts .deb, .rpm and tarball in `distro-matrix.sh`, not the GUI of the Flatpak) | blocked on owner, unverified | P2 | `release-0.3.0.md` §5 | owner |
| REL-5 | SBOM in `release.yml` (no `cyclonedx` or `sbom` step exists, verified by grep) and a note on what is and is not reproducible | open | P2 | `release-plan.md` §2, `security.md` G13 | agent |
| REL-6 | Pin the remaining third-party actions by SHA (a few are left, among them `actions/upload-artifact@v4` and `msys2/setup-msys2@v2`; Dependabot PR #14 is open) | open | P2 | `security.md` G13 | agent |
| REL-7 | GitHub social preview image upload (no API) and tag protection for `v*` | blocked on owner, unverified | P2 | `release-0.3.0.md` §7 and §8 | owner |
| REL-8 | Decide whether binaries get `strip = "symbols"` (the tarball and Flatpak ship unstripped; backtraces in bug reports become useless when stripped) | blocked on owner | P2 | `release-0.3.0.md` §9 | owner |
| REL-9 | Decide the crates.io licence and publish policy (all crates have `publish = false` today) | blocked on owner | P3 | `Cargo.toml` | owner |
| REL-10 | Man pages and shell completions for `bookx-cli` (neither exists, checked 2026-11-23) | open, unverified | P3 | `release-0.3.0.md` §0 (history) | agent |
| REL-11 | Own Flatpak repository on GitHub Pages (needs a GPG key-custody decision) and the Flathub submission (manifest and PR written by the owner; exit criteria in `release-plan.md` §5) | blocked on owner | P3 | `release-plan.md`, `flathub-submission.md` | owner |
| REL-12 | Homebrew, winget, Scoop, MSYS2 publication (after macOS and Windows builds are tested on real machines) | open | P3 | `release-plan.md` §4, `macos-port.md`, `windows-port.md` | owner, agent |

## Devices and sync

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| DEV-1 | Issue #39 (Boox Leaf 2 not syncing books): Boox device names, an honest explanation that sync never copies or deletes books, e-reader device zoo emulation tests | merged (PR #42); ships in beta.3 | P1 | issue #39, `hardware-test-plan.md`, `enhancements.md` | agent |
| DEV-2 | First sync of plugin 0.3.1 on the owner's Kobo (installed, first sync not confirmed) and the 7-step live-update checklist (server and device name, Sync now, Notes list, live highlight/colour/delete/bookmark, "device is ahead" and edit-conflict banners, Wi-Fi off and on). Checklist text: Part 2 A.1 of `remaining-work.md` | blocked on owner, unverified | P1 | `koreader-plugin-testing.md`, `remaining-work.md` | owner |
| DEV-3 | Kobo native highlight colour numbers (`Color` 0..3) and KOReader 2024 or newer reading the notes file Bookx writes (bugs 12 and 22) | blocked on owner, unverified | P2 | `bugs.md` 12, 22, `koreader-annotation-sync.md` | owner |
| DEV-4 | Hardware verification: Kindle, PocketBook, Boox, other Kobo models, touch and e-ink panels | blocked on owner | P2 | `hardware-test-plan.md` | owner |
| DEV-5 | Flatpak sandbox checks: `/run/media`, MTP through GVfs, voices, portal, network places, `bookx://`, Open With | blocked on owner, unverified | P2 | `flathub-submission.md` §5, `packaging-notes.md` | owner |
| DEV-6 | Real-service checks: Wallabag, Hardcover (`me.user_books`, status ids), Anki, Readwise, OPDS catalogues, an MCP client, the web client in Firefox and Chromium (service worker under a sub-path, Basic-auth prompt) | blocked on owner, unverified | P2 | `archive/serve-sources-notes.md`, `archive/screens-notes.md`, `mcp-notes.md` | owner |
| DEV-7 | Popup race fix of bug 12 relies on a 90 ms delay, checked on one machine | open, unverified | P2 | `bugs.md` 12 | agent |
| DEV-8 | Per-user accounts: scope is the tags of a book; accounts that see shelves instead of tags are not offered (the server only knows tags). Owner decides whether to add it | blocked on owner | P3 | `archive/screens-notes.md` | owner |
| DEV-9 | Multi-device library sync (backups are one-way). Decide whether it is wanted | blocked on owner | P3 | `library-sync-design.md` | owner |
| DEV-10 | Unify `devices::full_sync_plan` with the `device_sync` plan, direct MTP (gvfs only today), Kindle native reading state, FTP/SSH | open, unverified | P3 | `archive/serve-sources-notes.md`, `enhancements.md` 4 | agent |

## Annotations and history

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| ANN-1 | History and restore of highlights, notes and bookmarks, with a SQLite journal (`annotation_history` table written by the store in the same transaction as the change) | merged (PR #44); ships in beta.3 | P1 | `design-audit.md` §4 step 2 (PR #41), `enhancements.md` | agent |
| ANN-2 | Move highlights and bookmarks out of the per-book JSON into tables (schema v11, counts verified per book and in total, arrays dropped from `data`); lazy loading of the open book's annotations follows | open | P1 | `design-audit.md` §4 step 3 (PR #41) | agent |
| ANN-3 | Editing the text of a highlight (roadmap R31) and an automatic last-position bookmark (F6) | open, unverified | P2 | `roadmap-status.md` R31, F6 | agent |
| ANN-4 | Notes view: the date filters still hold the date as hidden entries, and the reader sidebar has its own copy of the note row | open, unverified | P3 | `archive/ux-gaps-notes.md` | agent |

## Reader and editor UX

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| UX-1 | Reader fidelity: real table layout, positioned text, hanging punctuation, `word-break`, emphasis dots, vertical tables (R1); fixed-layout spreads and max-width (R46, R51); SMIL (R18) | open | P2 | `roadmap-status.md` | agent |
| UX-2 | Reader and Preferences at phone size (360 px, X5) | open, unverified | P2 | `roadmap-status.md` X5 | agent |
| UX-3 | Performance at scale: scroll fps in a real session at 100k books, PDF open in the app, lazy `BookItem`s, memory (600 MB at 100k) | open, unverified | P2 | `performance.md`, `perf.md` | agent |
| UX-4 | The library title and subtitle are not re-translated when the language changes at run time; the `Gtk-WARNING` about snapshotting a `GtkScrolledWindow` without allocation in `library_hero_*` resize; at 1024 px the details panel does not narrow the grid (`grid_reflow_1400x900` assertion relaxed) | open, unverified | P2 | `archive/qa-wave5.md`, `remaining-work.md` Part 2 E | agent |
| UX-5 | Scenarios that do not exist yet: touch and long-press, the `show_action` toast, `book_formats::merge` undo, metadata provenance UI | open, unverified | P2 | `remaining-work.md` Part 2 G | agent |
| UX-6 | Access keys (`_` mnemonics) for the three menus (HIG 5.6.1, decision D11: later) | open | P3 | `archive/human_interface_backlog.md` | agent |
| UX-7 | Library: bookshelf and cover-flow view (L17), interactive cover crop, column colouring rules, tag drag and drop, saved searches with OR and parentheses, RAR in the multi-book archive picker, MCP custom columns | open, unverified | P3 | `roadmap-status.md`, `remaining-work.md` Part 2 D | agent |
| UX-8 | Lookup and statistics: statistics calendar export, PDF book map and thumbnail, half-star ratings, a typed-identifier editor (`bookx_sources::identifiers` is not used by `src/`) | open, unverified | P3 | `archive/ux-gaps-notes.md`, `archive/screens-notes.md` | agent |
| UX-9 | Editor: per-family preview fonts, "lines above sync point", vertical-text markers in the preview, editor plugin panels, CSS transforms in the UI | open, unverified | P3 | `remaining-work.md` Part 2 D, `AGENTS.md` §3 | agent |
| UX-10 | Read aloud and audiobooks: MP3/OGG export, Kokoro Japanese/Chinese, Orpheus tier, `cpal` sink; Opus playback, up to 50 ms of silence from AAC priming, first scan of a huge folder on the UI thread | open, unverified | P3 | `tts-notes.md`, `audiobook-notes.md` | agent |
| UX-11 | Formats: OCR hook, PDF output (C7), LRF, AZW4, KF8 flows, DOCX TOC fields, remote reader control over HTTP (R61) | open, unverified | P3 | `roadmap-status.md` | agent |

## Data and storage

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| DATA-1 | Library update safety: a copy of the database before a schema update, integrity and book-count check afterwards, refuse to start when the copy cannot be made, newest two copies kept | merged (PR #41) | P1 | `design-audit.md` §3.1 | agent |
| DATA-2 | Statistics (every reading session), vocabulary, the KOReader ledger and raw annotations, KOSync state, sync activity and shelves from about 16 whole-file JSON files into the library database (schema v12 or newer, old files kept as `.migrated`); they are not in library backups today | open | P1 | `design-audit.md` §3.2 and §4 step 4 | agent |
| DATA-3 | Rotate passwords that were in old plaintext config copies; try the master-password dialogs by hand; GNOME Keyring and the Flatpak secrets portal | blocked on owner | P1 | `security.md`, `backup-notes.md` | owner |
| DATA-4 | Settings file gets a `version` and per-version steps instead of "reset once" flags | open | P2 | `design-audit.md` §4 step 5 | agent |
| DATA-5 | Every schema change from now on: numbered migration in one transaction, the pre-update copy, a test from the previous version's database, a progress dialog for big libraries (also to be written into `AGENTS.md` §5) | open | P2 | `design-audit.md` §4 | agent |
| DATA-6 | Backup verification on real places: WebDAV, Nextcloud, NAS, Google Drive (chunked upload too) | blocked on owner | P2 | `backup-notes.md`, `hardware-test-plan.md` | owner |
| DATA-7 | Backup passphrase change and take over from the CLI, per-byte progress, a GTK add/edit scenario | open, unverified | P3 | `backup-notes.md`, `enhancements.md` 7 | agent |
| DATA-8 | Plugin signature verification (G10): needs one audited ed25519 crate (none is a dependency, verified), then the "unverified plugin" banner | blocked on owner (crate decision) | P2 | `security.md` G10, `plugin-development.md` | owner, agent |
| DATA-9 | Operation log (200 entries, JSON next to the database) into the database in the same transaction as the change; lazy per-book loading and a query path for the CLI, MCP and sharing server that does not load everything | open | P3 | `design-audit.md` §3.3, §3.4 | agent |
| DATA-10 | The sharing server (`CatalogWatcher`) re-reads and clones the whole catalogue on every commit | open, unverified | P2 | `design-audit.md` §2 | agent |

## CI and quality

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| CI-1 | Flaky `win11_scale_library` (UI scenario in `build-aux/ui-tests-win8.sh`); also `koreader_annotations_live_reader` was reported as often failing on the runner | open | P1 | `ci-gates.md` | agent |
| CI-2 | `build-aux/session-tests.sh` (real GNOME Shell session) is not run by any workflow (none names it); the harness is flaky with 3 or more concurrent sessions (inotify) | open | P2 | `test-notes.md` | agent, owner |
| CI-3 | Nightly `cargo fuzz` job and real fuzz hours (13 targets are only replayed on seeds); targets for audiobook tags, StarDict and the Calibre `metadata.db` | open | P2 | `security.md` §12, `security-log.md` | agent, owner |
| CI-4 | Sandbox for non-Flatpak installs (bwrap or Landlock) | open | P3 | `security.md` G11 | agent |
| CI-5 | Backup crypto second review; `cargo vet`; revisit the seven "unmaintained" notices in `deny.toml` | open | P2 | `security.md` P2.4, P2.5 | owner, agent |
| CI-6 | Threat-model review cadence, incident runbook, authentication by default for servers, MCP write audit view | open | P2 | `security.md` P3, P4 | owner, agent |
| CI-7 | Check `koreader_annotations.rs::the_real_plugin_registers_and_syncs_against_a_real_server` (14 versus 15 highlights) | open, unverified | P2 | `remaining-work.md` Part 2 G | agent |
| CI-8 | Network fixtures run only with `NET=0` in CI; real mouse-drag scenarios call handlers directly; perf budgets for scroll fps and 100k cold start | open, unverified | P2 | `ci-gates.md`, `test-notes.md` | agent |
| CI-9 | Reproducible builds: the tarball packing is deterministic, the binaries are not; write down what is and is not reproducible | open | P3 | `release-plan.md` §3 | agent |

## Platforms

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| PLAT-1 | macOS: bundle, Homebrew formula and cask never run on a real Mac; no native audio sink for read aloud and audiobooks | open | P2 | `macos-port.md`, `package-native.yml` (manual) | owner, agent |
| PLAT-2 | Windows: package scaffolds (winget, Scoop, MSYS2) never run on a real machine; `bookx-backup` and `bookx-tts` use `std::os::unix` (check whether outside tests) | open, unverified | P2 | `windows-port.md` | owner, agent |
| PLAT-3 | Read aloud and audiobooks on a real sound card (pause/drain, prefetch, MPRIS, `pacat`/`pw-cat` latency, 3x speed, part-boundary clicks); aarch64 never run | blocked on owner | P3 | `tts-notes.md`, `audiobook-notes.md` | owner |
| PLAT-4 | CBR (`unrar` or `7z`) and DJVU (`ddjvu`) with real RAR5 and DjVu files; both are subprocess paths never run on real archives | blocked on owner, unverified | P3 | `roadmap-status.md`, `security.md` | owner, agent |
| PLAT-5 | AppImage yes or no, aarch64 yes or no | blocked on owner | P3 | `release-plan.md` §4 | owner |
| PLAT-6 | Apple Developer ID and notarization; Microsoft signing | blocked on owner | P3 | `macos-port.md`, `windows-port.md` | owner |

## Docs and translations

| ID | Item | Status | Pri | Pointer | Who |
|---|---|---|---|---|---|
| DOC-1 | Create the Weblate project `bookx` on hosted.weblate.org (component file `weblate/component.yml` exists), add the webhook, update the wiki page Translating | blocked on owner | P2 | `translations-hosting.md` | owner |
| DOC-2 | Native review of the machine-translated de, es, fr and hi (all four are `machine-translated` in `po/STATUS.md`); decide who reviews | blocked on owner | P2 | `po/STATUS.md`, `translations-hosting.md` | owner |
| DOC-3 | Move the finished documents that are still pointed to from `enhancements.md` or source comments into `archive/` once those pointers can be changed: `remaining-work.md`, `pain-points-calibre.md`, `pain-points-ebook-managers.md`, `tts-research.md`, `pdf-research.md`, `shortcuts-research.md`, `kobo-real-device-report.md`, `editor-sourceview.md` | open | P2 | `README.md` (this folder) | agent |
| DOC-4 | The last paragraph of `bugs.md` still says bugs 27 and 29 are open (both rows are `fixed`); fix it when nobody else is editing the table | open | P2 | `bugs.md` | agent |
| DOC-5 | Public roadmap (a wiki page or a pinned issue derived from `bookx-feature-roadmap.md`, user-facing items only), GitHub Discussions (off today), at least 5 open `good first issue` items, triage within about 3 days | blocked on owner | P2 | `release-plan.md` §3 | owner, agent |
| DOC-6 | Wiki screenshots for the newest windows and any that are stale (Sync window rows, Import/Export Calibre, About, reader Properties); mockups for new windows; duplicate sections in `wiki/docs/editing.md` | open, unverified | P3 | `wiki-notes.md` | agent |
| DOC-7 | Keep the wiki's "Hardware tested so far" honest until DEV-2 and DEV-4 are done | open | P3 | `hardware-test-plan.md` | agent |

## Owner-only list

Everything above that only the owner can do, in one place.

| What | Items |
|---|---|
| Accounts and settings | AUR account and SSH key (REL-2); Weblate project (DOC-1); social preview upload and tag protection (REL-7); GitHub Discussions (DOC-5); GPG key custody for an own Flatpak repo and the Flathub account (REL-11) |
| Decisions | crates.io licence and publish policy (REL-9); strip symbols (REL-8); ed25519 crate for plugin signatures (DATA-8); multi-device library sync (DEV-9); shelves for accounts (DEV-8); AppImage and aarch64 (PLAT-5); who reviews translations (DOC-2) |
| By hand | rotate old plaintext passwords and try the master-password dialogs (DATA-3); first Kobo sync with plugin 0.3.1 (DEV-2); Flatpak GUI start (REL-4) |
| Hardware | Kobo colour numbers (DEV-3); Kindle, PocketBook, Boox, other Kobo models (DEV-4); real sound cards (PLAT-3); WebDAV, NAS and cloud-drive backups (DATA-6); CBR and DJVU files (PLAT-4); a Mac and a Windows machine (PLAT-1, PLAT-2) |
| Real services | Wallabag, Hardcover, Anki, Readwise, OPDS, an MCP client, a browser for the web client (DEV-6) |

## Decided, do not ask again

| Date | Decision | Where |
|---|---|---|
| 2026-11-23 | Debian 13 (trixie), Debian 12 and Ubuntu 24.04 are not supported by the native packages (GTK older than 4.20); they use the Flatpak. Minimum stays GTK 4.20 / libadwaita 1.8 | `install.md` (wiki), REL-3 |
| 2026-11-23 | Deleting a highlight on a device deletes it in the library (per-highlight sync); a book is never copied or deleted by sync; History restores what was removed | `sync-rules.md`, wiki `annotations.md` |
| 2026-11-22 | GitHub Releases is the only channel for now; Flathub and the other stores come after the project has history | `release-plan.md` |
| 2026-11-21 | Translations on Weblate (hosted.weblate.org); no translator credits for AI text | `translations-hosting.md` |
| 2026-11-21 | Release signing: cosign keyless and build provenance; private vulnerability reporting, advisory owner @adhadse | `security.md` §11, §13 |
| 2026-11-20 | Wireless sync is KOReader only (D10); stock firmware is USB; the Calibre wireless protocol is not planned | `bookx-feature-roadmap.md` Part 2, `outdated/calibre-wireless-device-plan.md` |
| 2026-11-21 | OPDS server is the shared server, Online Catalogues the client list (D10); mnemonics later (D11); catalogues are a sidebar destination (D12); no label ends with an ellipsis (D14) | `archive/human_interface_backlog.md` §7 |

## What was dropped as already done

Checked against the code, `gh` or `git log` on 2026-11-23, so they are not on the list:

- Bugs 27 and 29 of `bugs.md` (lazy MCP token, accessible name of *Show All*): rows say `fixed`; every row of the table is fixed.
- Branch protection with the `required` check is set on `main` (read with `gh api`); CI and the release pipeline have run on GitHub (v0.3.0-beta.1 and beta.2 are published with `.sigstore.json` bundles and `SHA256SUMS`).
- Install tests on Fedora 43/44/45, Ubuntu 25.10/26.04, Debian testing and Arch run in the release pipeline (`distro-matrix.sh`).
- The metainfo carries a `content_rating` (OARS).
- The 24 PARTIAL/MISSING roadmap rows closed after the 2026-11-21 sample, the 213 GNOME HIG rows (all addressed), the screens for accounts, Hardcover, recipes and lookup order, and X18 windowed layout (`performance.md`).
- Bug 16 (main-thread stall on a 20 000-entry contents list) is fixed.
