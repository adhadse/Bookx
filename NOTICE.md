# Third-party assets

| Asset | Source | Licence |
|---|---|---|
| `data/icons/hicolor/scalable/actions/wikipedia-symbolic.svg` (Wikipedia "W" glyph) | [Simple Icons](https://simpleicons.org) `icons/wikipedia.svg` | CC0 1.0 Universal |
| `data/icons/hicolor/scalable/actions/funnel-symbolic.svg` (funnel for Filter buttons) | drawn for Bookx | CC0 1.0 Universal |

"Wikipedia" and the Wikipedia logo are trademarks of the Wikimedia Foundation, Inc. Bookx is not
affiliated with or endorsed by the Wikimedia Foundation. The glyph only labels the *Look up on
Wikipedia* action in the reader. Wikipedia article summaries shown in the reader are fetched at
runtime from the Wikipedia REST API and are available under CC BY-SA 4.0.

## Embedded data and libraries

| Component | Used by | Licence |
|---|---|---|
| `hypher` 0.1 and the TeX hyphenation pattern tries it embeds (per-language patterns under LPPL, MPL and similar permissive licences; see the `hypher` crate for each language) | `bookx-text` | MIT OR Apache-2.0 (code); pattern licences per language |
| SQLite (bundled by `rusqlite`/`libsqlite3-sys`) | `bookx-store` | Public domain |
| `md5`, `percent-encoding`, `url`, `base64` | `bookx-serve` (its HTTP server is written in the crate itself) | MIT / Apache-2.0 |
| `image` (JPEG and PNG only) | `bookx-convert` | MIT OR Apache-2.0 |
| `encoding_rs`, `flate2`, `tar` | `bookx-formats`, `bookx-lookup` | MIT / Apache-2.0 |
| `clap` | `bookx-cli` | MIT OR Apache-2.0 |
| `similar`, `spellbook`, `subsetter`, `ttf-parser`, `image`, `encoding_rs` | `bookx-editing` | Apache-2.0 / MIT (spellbook: MPL-2.0; check each crate's licence file when redistributing) |
| `bzip2` (pure Rust, via `libbz2-rs-sys`), `flate2`, `encoding_rs` | `bookx-convert` | MIT / Apache-2.0 / bzip2-1.0.6 |
| `lzxd`, `bzip2-rs` (and `lzxc`, `bzip2` as dev-dependencies) | `bookx-formats` | MIT / Apache-2.0 / BSD-style |
| `ureq` 3 with `rustls`, `ring` (Apache-2.0 AND ISC), `webpki-roots` (CDLA-Permissive-2.0), `rustls-webpki`, `untrusted` (ISC), `url`, `toml` | `bookx-sources` | MIT / Apache-2.0 / ISC / CDLA-Permissive-2.0 |
| `sourceview5` 0.11 (gtk-rs bindings, feature `v5_6`) and the system library GtkSourceView 5 (dynamic link). The code view of the book editor: syntax highlighting from its HTML/XML/CSS language definitions, the Adwaita / Adwaita-dark / classic style schemes, search context, snippets, completion, minimap and gutter. Chosen over `gtk::TextView` + the hand-written highlighter because it brings these features maintained by the GNOME project; the GNOME 49 Flatpak runtime already ships `libgtksourceview-5` 5.18, so the manifest needs no extra module | app (`src/ui/editor`) | MIT (`sourceview5`); LGPL-2.1-or-later (GtkSourceView; its language definitions and style schemes carry their own licence headers) |
| `selectors` (MPL-2.0), `cssparser` (MPL-2.0), `scraper` (ISC) | `bookx-text`, `bookx-sources`, app | MPL-2.0 / ISC |

No test fixtures are committed: every sample book, dictionary and database used by the tests is
generated in code.

## Added dependencies (code, not assets)
- wasmi (MIT/Apache-2.0), wat (Apache-2.0 with LLVM exception), toml, base64, url (MIT/Apache-2.0)
- oxipng (MIT) for PNG optimisation
- hyper, hyper-util, http-body-util, http-body, http, httparse, httpdate, tokio-rustls (MIT / Apache-2.0): HTTP server of `bookx-serve`
- lettre (MIT), email_address, email-encoding, fastrand, mime, socket2, nom, quoted_printable (0BSD): e-mail building and SMTP
- rustls, rustls-pki-types (Apache-2.0 / MIT / ISC), webpki-roots (CDLA-Permissive-2.0): TLS (ring provider)
- `rcgen` 0.14 (MIT OR Apache-2.0, with `ring`) is now also a runtime dependency of `bookx-serve`: it makes the self-signed certificate for HTTPS sharing (pure Rust, no OpenSSL).
- `nix` 0.31 (MIT, features `fs` and `user`) in `bookx-serve`: free space of a device (`statvfs`) and the user id for the gvfs folder; already in the dependency tree.
- `webpki-roots` 1 (CDLA-Permissive-2.0) in `bookx-serve`: trust anchors of the WebDAV client's HTTPS connections; already in the dependency tree through `ureq`. `rusqlite` (MIT, already used by `bookx-store`) reads and writes the Kobo database.
- Backup encryption (`bookx-backup`, pure Rust, RustCrypto): `chacha20poly1305` 0.10 (MIT OR Apache-2.0) with its `aead` STREAM, `argon2` 0.5 (MIT OR Apache-2.0, Argon2id), `sha2` 0.10 (MIT OR Apache-2.0), `getrandom` 0.2 (MIT OR Apache-2.0). Why these and not `age`: see `crates/bookx-backup/src/crypto.rs`.
- Read aloud downloads (`bookx-tts`): `zip` 9 (MIT) reads the directory of the NuGet package of ONNX Runtime (no decompression through it; the member is inflated with `flate2`); the same `zip` crate as the rest of the application. The library itself (ONNX Runtime 1.31.1, MIT) is downloaded by the user from nuget.org or GitHub, never bundled.
- Media keys for read aloud (`src/mpris.rs`): `zbus` 5 (MIT) used directly for the MPRIS service; it was already in the tree through `oo7`.
- Password storage (`bookx-secrets`): `oo7` 0.5 (MIT; Secret Service client and Flatpak secrets portal, with `zbus`, `ashpd`, `async-io` and their dependencies, MIT / Apache-2.0 / MPL-2.0 where noted by those crates), `async-io` 2 (Apache-2.0 OR MIT). `oo7` 0.5 is the newest release that builds with the Rust version this project pins.
- Function mode of the editor's search & replace (`src/editor_search_fn.rs`): `rhai` 1 (MIT OR Apache-2.0, with `rhai_codegen`, `smartstring`, `ahash`, `num-traits`, `thin-vec`, `bitflags`, `no-std-compat` and their dependencies, MIT / Apache-2.0). Why Rhai: Calibre's function mode is Python, which is excluded by "Rust only"; Rhai is a pure Rust scripting engine with no I/O of its own, operation, time, call-depth and string/array/map size limits, so a user's replacement function runs sandboxed (no files, no network, no modules, no `eval`). Built without `no_module`-incompatible features (`no_module`, `no_time`, `no_custom_syntax`).
- MCP server (`bookx-mcp`): no new third-party crate. The Model Context Protocol (JSON-RPC 2.0, specification at modelcontextprotocol.io, MIT licence for the schema/docs) is implemented on `serde_json` with the blocking HTTP server of `bookx-serve`; the official Rust SDK `rmcp` (Apache-2.0) was considered and not used (async/tokio dependency tree, see `docs/mcp-notes.md`).
- Dev only: mail-parser, wat


## Read aloud (`crates/bookx-tts`)
Nothing below is bundled or distributed with Bookx. The models and the runtime are downloaded by the user on demand
(Preferences → Text to Speech), each with its licence shown first and pinned by SHA-256 in
`crates/bookx-tts/src/catalog_data.rs` (generated by `build-aux/tts-catalog.py`).
- **ONNX Runtime** 1.31.1 (MIT, <https://github.com/microsoft/onnxruntime>), loaded at run time from the downloaded
  `libonnxruntime.so` through the `ort` crate (MIT OR Apache-2.0) with its `load-dynamic` feature.
- **Kokoro-82M** v1.0 (Apache-2.0, hexgrad; ONNX export and voice files from
  <https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX>). The phoneme vocabulary in `catalog_data.rs`
  (`KOKORO_VOCAB`) is the `tokenizer.json` of that repository.
- **Piper voices** (<https://huggingface.co/rhasspy/piper-voices>, MIT-licensed project; every voice carries the licence
  of the dataset it was trained on, listed in its MODEL_CARD and shown in the download dialog; the curated list includes
  CC0, CC-BY, a "Blizzard 2013" licence and one CC-BY-NC-SA voice). Bookx runs the `.onnx` voice files itself; Piper's
  GPL-3.0 `piper1-gpl` program is not used.
- **espeak-ng** (GPL-3.0-or-later, <https://github.com/espeak-ng/espeak-ng>) is run as a separate, system-installed
  program to turn text into phonemes. It is neither linked nor bundled.
- **Voice servers** the user may point Bookx at (XTTS v2: Coqui Public Model License, non-commercial; Fish Audio S2:
  Fish Audio Research License, non-commercial; Kokoro-FastAPI, openedai-speech, Speaches: their own licences) are not
  part of Bookx; Bookx only speaks their HTTP APIs.
- Rust crates added: `ort` (MIT OR Apache-2.0), `sha2` (MIT OR Apache-2.0), `flate2` (MIT OR Apache-2.0), `tar`
  (MIT OR Apache-2.0).
- PDF (`bookx-pdf`): `hayro` 0.9 with `hayro-syntax`, `hayro-interpret`, `hayro-font`, `hayro-cmap`, `hayro-ccitt`, `hayro-jbig2`, `hayro-jpeg2000`, `hayro-postscript` (Apache-2.0 OR MIT; pure Rust). Embedded data: `hayro-font` carries the 14 standard PDF fonts as the Foxit `.pfb` files extracted from PDFium (BSD-3-Clause, Copyright 2014 PDFium Authors / Foxit Software; the licence text is `LICENSE_FOXIT` in that crate and applies to binary redistribution), `hayro-interpret` carries two ICC colour profiles (CC0-1.0), `vello_cpu`/`vello_common`/`peniko`/`kurbo`/`fearless_simd`/`pic-scale`/`moxcms`/`zune-jpeg`/`zune-core`/`skrifa`/`read-fonts` (Apache-2.0 OR MIT). It needs Rust 1.92, so `crates/bookx-pdf` declares `rust-version = "1.92"`. No system library is needed, the Flatpak manifest is unchanged. Why: `docs/pdf-research.md`.

## Test tools (not distributed, not linked)
- `epubveri` (AGPL-3.0-only OR commercial, <https://github.com/veripublica/epubveri>) is used only as an external
  command-line oracle by `crates/bookx-editing/tests/epubveri_oracle.rs`, when the developer has installed it
  (`cargo install epubveri --root ~/.cache/epubveri`). It is not part of Bookx, not bundled with it and not linked into it,
  so its licence does not apply to Bookx.

## Test books (downloaded at test time, never committed or distributed)
- *Alice's Adventures in Wonderland* by Lewis Carroll (d. 1898) and *The Masque of the Red Death* by Edgar Allan Poe (d. 1849),
  from Project Gutenberg (<https://www.gutenberg.org/ebooks/11>, <https://www.gutenberg.org/ebooks/1064>). Both are public
  domain (the files say "Public domain in the USA."). They are fetched into `~/.cache/bookx-testbooks` by
  `build-aux/session-tests.sh` and used only by the real-session UI tests; see `docs/test-books.md`.
- *Wagahai wa neko de aru* (吾輩は猫である) by Natsume Soseki (d. 1916), from Aozora Bunko
  (<https://www.aozora.gr.jp/cards/000148/files/789_14547.html>), public domain (the author died in 1916; the Aozora Bunko
  file is made by volunteers and free to use). It is fetched into `~/.cache/bookx-testbooks` by `build-aux/session-tests.sh`
  and converted to a vertical EPUB by `build-aux/make-vertical-epubs.py` for the vertical-writing scenarios. The same text,
  drawn by the reader in the Noto Serif CJK JP font (SIL Open Font Licence, a system font, not distributed), is the
  screenshot `wiki/docs/assets/reader-vertical.png`.
- Cover art for the twelve books of the wiki screenshots (*Moby-Dick*, *Pride and Prejudice*, *Frankenstein*, *Dracula*,
  *The Time Machine*, *Treasure Island*, *Jane Eyre*, *The Odyssey*, *Walden*, *Great Expectations*, *The Art of War*,
  *Siddhartha*) from Standard Ebooks (<https://standardebooks.org>), whose cover art is in the public domain and whose own
  work is dedicated to the public domain (CC0 1.0). It is downloaded into `~/.cache/bookx-wiki-covers` by
  `build-aux/make-wiki-books.py` and appears in the wiki screenshots (`wiki/docs/assets/library-banner-hero-*.png`,
  `library-*.png`, `reader-*.png`); no cover file is committed to this repository.
- The wiki's social cards (the link previews of <https://bookx.adhadse.com>) are drawn by Zensical's `social` plugin
  with the **Inter** typeface, which it downloads from Google Fonts at build time and caches in
  `wiki/.cache/plugin/social` (not committed). Inter is licensed under the SIL Open Font Licence 1.1
  (<https://github.com/rsms/inter>), which permits embedding the rendered glyphs; no font file is redistributed.

`crates/bookx-editing/testdata/cmyk.jpg` (351 bytes) is a CMYK JPEG made for the tests with Pillow from a flat colour: our own file, CC0. It is the one committed fixture (a CMYK JPEG cannot be generated with the `image` crate).
- `unicode_names2` 4 (MIT OR Apache-2.0, with `phf`) in `bookx-editing`: the Unicode character names of the Insert Special Character dialog and the Characters report. Pure Rust, tables compiled in (about 0.5 MB); chosen over reading `UnicodeData.txt` at run time, which would need a bundled data file.

`crates/bookx-convert/tests/fixtures/calibre-made/` (336 KB): files written by Calibre 9.15's `ebook-convert` (GPL-3.0) from the synthetic books of `tests/corpus/mod.rs`, which are our own text (CC0). No Calibre source or sample book is copied; only the program's output for our own content. Regenerate with `build-aux/convert-parity.sh --fixtures`.

## Packaging
- The Flatpak (`build-aux/com.adhadse.Bookx.json`) builds **espeak-ng** 1.52.0 (GPL-3.0-or-later,
  <https://github.com/espeak-ng/espeak-ng>) as its own module and Bookx runs it as a separate program (see the
  espeak-ng note above). The release archive is downloaded and pinned by hash; nothing is copied into this repository.
- `build-aux/cargo-sources.json` lists the crates.io packages (with their sha256 from `Cargo.lock`) the offline Flatpak
  build downloads; it is generated by `build-aux/flatpak-cargo-sources.py` (our own, standard library only; the idea is
  that of `flatpak-cargo-generator`, no code copied).
- `po/fr.po` and `po/hi.po` and the Spanish and German files are machine-assisted translations of our own strings (CC0 /
  GPL-3.0-or-later with the program); they need review by native speakers, see the headers.
