# Working agreement for this repository

These rules apply to every request, from anyone (human or AI assistant) changing Bookx.

## 1. Every request ships with tests

Whenever you add a feature, change behaviour or fix a bug, add or update tests **in the same change**:

- **Logic first.** Put decisions in pure functions (no GTK) and unit-test them next to the code
  (`#[cfg(test)] mod tests`). Examples: `web::pick_translate_target`, `reader::estimate_book_pages`,
  `reader::chrome_busy`, `query::Query`, `opf` edits.
- **UI behaviour gets an end-to-end scenario** in `build-aux/ui-tests.sh`. Add a dev command in the
  component's `Dev` handler if the scenario needs to drive or inspect a widget (see `reader:ui-note`,
  `reader:dump`), then add a `scenario NAME 'dev script' 'checks'` block.
- **A bug fix needs a regression test** that fails without the fix.
- Update the README / `_developer/docs.md` if user-visible behaviour or tooling changed, and the mockups
  (`cargo run --example mockups`) if the design changed.

### Two kinds of documentation, kept apart

- **Public wiki (`wiki/`)**: what users and plugin authors read at https://bookx.adhadse.com (Zensical, deployed
  by GitHub Pages). Whenever a change is user-visible (a feature, menu entry, shortcut, setting, CLI command or
  option, file format, plugin or patch format, sharing/device behaviour, keyboard or dev-hook-free workflow), update
  or add the matching wiki page **in the same change**, with up-to-date screenshots/mockups where the page has them.
  Write for users: how to do it, what to expect, limits. No internal status, no agent notes.
- **Internal notes (`docs/`)**: progress logs, roadmap and decisions, research, gap analyses, bugs
  (`docs/bugs.md`), enhancements (`docs/enhancements.md`), caveats, untested-on-hardware lists, performance
  numbers, design drafts. Never put these on the wiki. Record work progress and caveats here, not in the wiki.
- `README.md` stays short (what Bookx is, how to build, link to the wiki). Dev hooks and test tooling live in
  `_developer/docs.md`.
- Build check: `build-aux/wiki.sh` (builds the wiki with Zensical and fails on broken links); run it when you
  touched `wiki/`.

## 2. Tests must run in parallel

- Rust tests are independent (no shared files, ports or global state) so `cargo test` can run them in parallel.
- UI scenarios run concurrently: each gets its own Broadway display, D-Bus session (`dbus-run-session`),
  config, cache and library. Never rely on state from another scenario, never `pkill bookx` globally.
- Tests that touch the network must be skippable (`NET=0`).
- Concurrency is configurable: `JOBS=16 build-aux/ui-tests.sh` (default = CPU cores, `0` = unlimited) and
  `RUST_TEST_THREADS=16 cargo test`. Do not hard-code a core count. Why processes: GTK is single threaded and
  `#[gtk::test]` runs serially, so parallelism comes from one process per scenario (what Relm4's own tests do too).

## 3. Workspace map and ownership

The feature roadmap is `docs/bookx-feature-roadmap.md` (read Part 2 first: the owner's decisions).
Parallel agents work in different crates and meet only at `crates/bookx-core/src/contracts.rs`
(`FormatReader`, `FormatWriter`, `Transform`, `MetadataSource`, `DeviceDriver`, `Store`). Changing a
signature there is a cross-track change: say so in the change description.

| Path | Contents | Roadmap tracks |
|---|---|---|
| `crates/bookx-core` | GTK-free engine: EPUB, editing, conversion, library, query, contracts | core |
| `crates/bookx-store` | the library catalogue in SQLite (`open_library`, `CatalogBackend`, JSON migration), FTS5 search incl. book text, Calibre import, collections, backup, library registry | T3 |
| `crates/bookx-formats` | input readers: FB2/FBZ, RTF, DOCX, ODT, HTMLZ/TXTZ, CBZ/CBT, MOBI/AZW3 | T4 |
| `crates/bookx-convert` | writers (EPUB, KEPUB, FB2, MOBI, ...), transforms, pipeline, presets | T5 |
| `crates/bookx-text` | hyphenation, Knuth-Plass, CSS subset, highlights, typography settings, page numbering | T1 |
| `crates/bookx-lookup` | StarDict, vocabulary, statistics, in-book search, RSVP, annotation export | T2 |
| `crates/bookx-serve` | devices, KOSync, OPDS server/client, content server, mail | T6 |
| `crates/bookx-sources` | metadata sources (Open Library, Google Books, Wikidata), ISBN/identifier/author tools, filename guessing, duplicate detection, template language, read-later and news-to-EPUB, cover tools. Network only through the `Fetcher` trait; the edit dialog uses it for *Fetch Metadata*, the rest is not wired into the app yet | T8 |
| `crates/bookx-editing` | editor logic without GUI: reports, diff, checkpoints, split/merge, spell check, auto-fixes, EPUB 2 to 3, accessibility, font subsetting, image tools, link tools, CSS transforms. The editor's Tools menu uses fixes, upgrade, inline TOC, accessibility, images, fonts, arrange, reports, checkpoints and compare; spell check (Tools → Check Spelling, system Hunspell dictionaries); split at headings and merge with next; `files` (rename/bulk rename/new file/link stylesheets/export/quick open), `tags` (tag-pair scanner and markup actions), `toc_gen` (Contents generation); CSS transforms are not in the UI yet | T7 |
| `crates/bookx-plugin` | sandboxed WebAssembly plugin host (wasmi): manifest, ABI v1, adapters for the contracts, manager. The app lists/enables plugins in Preferences; they add metadata sources, file readers, output formats, transforms and device drivers. Also user patches (`patch.rs`, TOML regex/CSS rules); guide in `docs/plugin-development.md` | T10 |
| `crates/bookx-backup` | off-site backups of the catalogue: destinations (folder/drive/network/cloud-drive mounts, WebDAV), `.bookxbackup` encryption, verify-after-write, retention per place, restore, places detection; UI in `src/ui/backup.rs`, glue in `src/backups.rs` | enh. 7 |
| `crates/bookx-secrets` | passwords in the Secret Service (`oo7`) behind the `SecretStore` trait; `BOOKX_SECRETS=memory` in tests; wired in `src/config.rs` / `src/secrets.rs` | enh. 7 |
| `crates/bookx-mcp` | Model Context Protocol server for AI assistants, GTK-free: JSON-RPC protocol (`server.rs`), stdio and Streamable HTTP with bearer token (`stdio.rs`, `http.rs`), tools (`tools.rs` read, `tools_write.rs` behind `--allow-write`), resources, prompts, audit log; read-only by default, writes only through `Library` (operation log, undo). `bookx-cli mcp`; glue and page in `src/mcp_host.rs` / `src/ui/mcp_settings.rs`; notes in `docs/mcp-notes.md` | enh. 28 |
| `crates/bookx-tts` | read aloud, GTK-free: sentence splitting, `Engine`/`Session` (look-ahead, highlight events), gapless `PipelineEngine`, engines (OpenAI/XTTS/Fish voice server, Kokoro and Piper over ONNX Runtime with system espeak-ng; the speech-dispatcher engine was removed), first-voice download helpers (`setup`), model catalogue + resumable hash-verified downloads, sentence cache; UI in `src/ui/reader_tts.rs`, the player card `src/ui/tts_player.rs` (state model `src/tts_player.rs`, MPRIS `src/mpris.rs`) and `src/ui/tts_settings.rs`, glue in `src/tts.rs`; research in `docs/tts-research.md` | enh. 9 |
| `crates/bookx-cli` | `bookx-cli` binary: convert, meta, check, library, search, serve | X2 |
| `src/` (package `bookx`) | GTK/Relm4 app: reader, library, editor UI. Pure glue lives in `src/*.rs` (stats, import, export, devices, sharing, vocabulary, fulltext, dictionaries, gestures, eink, editor_tools, duplicates, web_import, plugins, metadata_fetch) with unit tests; GTK parts in `src/ui/` | T9 |
| future `crates/bookx-*` | plugin host and others, see roadmap section 4 | T7, T8, T10 |

`bookx-core` must never depend on GTK. `src/main.rs` re-exports it as `crate::core`.
Performance budgets live in `crates/*/tests/perf_budgets.rs`: a slower operation fails
`cargo test` (budgets are for release builds, debug gets 30x room, `BOOKX_PERF_SCALE` scales them;
`make perf` runs them in release). New hot paths get a budget there.

## 4. Before you finish

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
build-aux/ui-tests.sh          # needs gtk4-broadwayd; skips itself when it is missing
build-aux/session-tests.sh     # real GNOME Shell/Wayland session, real input; needs gnome-shell, skips itself when missing
```

All four must pass. Then rebuild and reinstall for the user with `./run.sh --no-run`.

## 5. Conventions

- Rust only for application code. New third-party assets must be licensed for redistribution and listed in
  `NOTICE.md`, with attribution in the source file that embeds them.
- Dev hooks (`BOOKX_DEV_SCRIPT`, `reader:*`, `library:*`, `editor:*`) are test infrastructure: keep them working.
- Do not leave debug output (`eprintln!`, custom panic hooks) in committed code.

## 6. Security

Bookx reads files it did not create and runs network services, so a change that touches untrusted input
is a security change. The model, the severity rubric, the handling process and the roadmap are in
`docs/security.md`; the public reporting policy is `SECURITY.md`.

- Run the per-change checklist in `docs/security.md` §8 in the change description when a change parses
  untrusted bytes, handles a path from a file or the network, spawns a process, opens a listener or
  fetches a user-supplied URL, adds a secret, or adds a dependency.
- A security fix needs a regression test that **fails before** the fix, plus an entry in the advisory
  (never only a commit message).
- The library crates are `#![forbid(unsafe_code)]`. Do not weaken it. The only `unsafe` allowed is gtk-rs
  FFI glue in `bookx` (`src/ui/`, object data and the accessibility vtable), and never on data that came
  from a file or the network.
- No setuid/setgid helper, no shell-string interpolation, no `PATH` lookup for a program that matters, and
  no "an unprivileged user cannot do that" as a mitigation.
- Report a vulnerability privately per `SECURITY.md`, never in a public issue.
