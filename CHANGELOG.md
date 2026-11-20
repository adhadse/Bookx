# Changelog

Hand-written, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project uses [semantic versioning](https://semver.org/). `build-aux/release-check.sh` fails when the newest
versioned heading, `Cargo.toml` and the AppStream metainfo disagree.

## [Unreleased]

### Added
- Editor: syntax-highlighted code editing (GtkSourceView 5), preferences, shortcuts and snippets, structure tools,
  editor plugins (plugin ABI v2, `editor-action`), narration while editing.
- Library: responsive header, jobs panel.
- Packaging: `.deb` and `.rpm` metadata (`cargo deb`, `cargo generate-rpm`), `build-aux/release-check.sh`
  (`make release-check`), `build-aux/tarball.sh`, release workflow with checksums.
- Plugin compatibility test with golden ABI v1 and v2 plugins; the ABI stability policy is documented in the plugin
  guide.

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

[Unreleased]: https://github.com/adhadse/Bookx/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/adhadse/Bookx/releases/tag/v0.2.0
