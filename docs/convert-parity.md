# Converter parity with Calibre (enhancement 6)

How the Bookx converters compare with Calibre's `ebook-convert` (9.15 on the test machine), what was fixed,
what differs on purpose and what is still open. Internal notes; the user-facing summary is in
`wiki/docs/converting.md`.

## How it is tested

* **Harness**: `build-aux/convert-parity.sh` (full matrix, `--smoke`, `--report`, `--fixtures`, `--update-doc`) drives
  `crates/bookx-convert/tests/parity.rs`. It skips cleanly without `ebook-convert`; plain `cargo test` runs only a
  3-second smoke subset (`BOOKX_PARITY=1` for everything).
* **Corpus** (`tests/corpus/mod.rs`, generated in code, no third-party files): EPUB 3 with nav, footnotes, tables,
  nested lists, entities, cover and series; EPUB 2 with a 3-level NCX; RTL (Arabic/Hebrew); ruby and CJK; SVG cover
  page; tag soup XHTML; 300-chapter contents; CSS edge cases; 4000 px images; FB2 (binaries, poems, notes, nested
  sections, windows-1251); DOCX (styles, table, list, footnote, hyperlink, image); ODT; RTF (table, unicode escapes);
  Markdown with front matter; HTML and HTMLZ with relative resources; CBZ; TXT in UTF-8, UTF-8 BOM, UTF-16, cp1252,
  cp1251 and hard wrapped CRLF. Plus files **written by Calibre** (MOBI, AZW3, PDB, PMLZ, RB, SNB, FB2, DOCX, RTF,
  TXTZ, HTMLZ) from the same book, so Bookx's readers are tried on real Calibre output. The Calibre-made fixtures
  are committed (`tests/fixtures/calibre-made/`, our own synthetic content converted by Calibre; nothing of
  Calibre's is copied) and are checked without Calibre in `tests/calibre_made.rs`.
* **Method** per source S and target T: the truth is Calibre reading S. Per cell, with `ref` = Calibre re-reading
  its own T file:
  * `W` writer: Calibre re-reads *Bookx's* T file and must keep as much as it kept of its own;
  * `R` reader: *Bookx* reads Calibre's T file, compared with the same yardstick;
  * `E` end to end: Bookx writes and Bookx reads;
  * `L` loss against Bookx's own EPUB; `V` validity of the EPUB output (repo validator and `epubveri`);
  * `N` a side failed. Compared facets: title, authors, language, identifiers, series, cover, contents entries and
    depth, word recall/extra/order (normalised token stream), images, broken internal links.
  Only differences *worse than Calibre* count. `tests/parity_known.txt` lists the accepted ones; anything else fails
  the run. Ideas for corpus cases came from Calibre's documented conversion behaviour and its bug tracker themes
  (encodings, tag soup, nested lists, KF8 skeleton/fragment tables, NCX nesting); no Calibre test code or files were
  copied.
* **Properties and fuzzing** (`tests/roundtrip.rs`, no Calibre needed): random documents (with `&`, `<`, accents,
  CJK, RTL, emoji) through every writer and back (`BOOKX_FUZZ_CASES`, `BOOKX_FUZZ_SEED`; 300 cases pass), damaged
  files (truncation, bit flips, zeroing) for every reader and every Bookx writer output with a 60 s limit per input,
  zip bombs for eight formats, 60000x60000 pixel images as cover and pages in eleven target formats, 5000-deep
  nesting.

## Matrix (`ok` = no difference worse than Calibre; codes are accepted differences, see below)

Rows: source (`cal-…` = file written by Calibre). Columns: target. Codes: check letter, then category.

<!-- matrix:begin -->
| source \ target | epub | mobi | azw3 | fb2 | htmlz | txtz | txt | docx | rtf | pdb | pmlz | rb | snb |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| epub3-features | ok | R:toc-depth | ok | W:text-extra | E:toc-depth<br>R:toc-depth | W:cover | ok | E:cover+toc-depth<br>R:toc-depth+links<br>W:cover+toc-depth | E:toc-depth<br>R:toc-depth<br>W:links | E:toc-depth<br>W:toc-depth | ok | E:toc-depth<br>R:toc-depth | ok |
| epub2-ncx | ok | R:toc-depth | ok | ok | ok | W:cover | ok | E:cover<br>R:links<br>W:cover | W:toc-depth | E:toc-depth<br>R:toc<br>W:toc-depth | ok | ok | ok |
| epub-rtl | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | R:links | W:text-missing | ok | ok | **calibre-rereads-own-output-fails** | ok |
| epub-ruby-cjk | ok | ok | ok | ok | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | R:links | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | **calibre-rereads-own-output-fails** | ok |
| epub-svg-cover | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:cover+text-extra | E:text-extra<br>W:text-extra | E:cover<br>W:cover | ok | ok | ok | ok | E:cover<br>W:cover |
| epub-broken-xhtml | ok | ok | ok | ok | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | ok | ok | ok | ok | ok |
| epub-long-toc | ok | ok | ok | W:text-extra | ok | ok | ok | R:toc+links | ok | R:toc | ok | ok | ok |
| epub-css-edge | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | ok | ok | ok | ok | ok |
| epub-big-image | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing+text-extra | ok | ok | ok | ok |
| fb2-full | E:toc+text-missing<br>W:toc+text-missing | E:toc+text-missing+text-order<br>R:toc-depth<br>W:toc+text-missing+text-order | E:toc+text-missing<br>R:links<br>W:toc+text-missing | E:toc-depth+text-missing<br>R:toc<br>W:toc-depth+text-missing | E:toc-depth+text-missing<br>W:toc+text-missing | E:text-missing<br>R:cover<br>W:cover+text-missing | E:text-missing<br>W:text-missing | E:cover+toc+toc-depth+text-missing<br>R:cover+toc+toc-depth+links<br>W:cover+toc+toc-depth+text-missing | E:toc-depth<br>W:links | W:toc-depth | W:toc | **calibre-rereads-own-output-fails** | E:toc+toc-depth+text-missing<br>W:toc+toc-depth+text-missing |
| fb2-cp1251 | E:text-missing<br>W:text-missing | E:text-order<br>W:text-order | E:text-missing<br>W:text-missing | E:text-missing | E:text-missing<br>W:toc+text-missing | E:text-missing+text-extra<br>W:text-missing+text-extra | E:text-missing+text-extra<br>W:text-missing+text-extra | E:text-missing<br>W:text-missing | E:text-missing<br>W:text-missing+text-extra | ok | ok | **calibre-rereads-own-output-fails** | E:text-missing<br>W:text-missing |
| docx-full | E:text-missing<br>W:text-missing | E:text-missing<br>R:toc-depth<br>W:text-missing | E:text-missing<br>W:text-missing | E:text-missing<br>W:text-missing+text-extra | E:text-missing<br>W:text-missing | E:text-missing+text-extra<br>W:text-missing+text-extra | E:text-missing+text-extra<br>W:text-missing+text-extra | E:text-missing<br>R:toc-depth<br>W:text-missing | E:text-missing<br>W:text-missing+links | E:toc-depth+text-missing<br>W:toc-depth+text-missing | E:text-missing<br>W:text-missing | E:text-missing<br>W:text-missing | E:text-missing<br>W:text-missing |
| odt-full | ok | E:text-extra<br>W:text-extra | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing | ok | ok | ok | ok |
| rtf-full | ok | E:text-extra<br>W:text-extra | ok | ok | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing | ok | ok | ok | ok |
| markdown | ok | E:text-extra<br>W:text-extra | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | R:links | W:text-missing+text-extra | ok | ok | ok | ok |
| cbz | ok | ok | ok | ok | ok | **bookx-rejects-calibre-output**<br>E:text-extra<br>W:text-extra | **bookx-rejects-calibre-output**<br>E:text-extra<br>W:text-extra | R:links | **bookx-rejects-calibre-output**<br>**bookx-rejects-own-output** | **bookx-rejects-calibre-output** | ok | ok | ok |
| txt-utf8 | ok | ok | ok | ok | ok | ok | ok | ok | W:text-missing | ok | ok | ok | ok |
| txt-utf8-bom | ok | ok | ok | ok | ok | ok | ok | ok | W:text-missing | ok | ok | ok | ok |
| txt-utf16le | ok | ok | ok | ok | ok | ok | ok | ok | W:text-missing | ok | ok | ok | ok |
| txt-cp1252 | ok | ok | ok | ok | ok | ok | ok | ok | W:text-missing | ok | ok | ok | ok |
| txt-cp1251 | ok | E:text-extra<br>W:text-extra | ok | ok | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing | ok | ok | ok | ok |
| txt-hardwrap-crlf | ok | E:text-extra<br>W:text-extra | ok | ok | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing | ok | ok | ok | ok |
| html-single | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing+links | ok | ok | ok | ok |
| htmlz-resources | ok | ok | ok | ok | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:text-missing+links | ok | ok | ok | ok |
| cal-epub3-features-mobi | E:toc-depth<br>W:toc-depth | E:toc-depth<br>R:toc-depth<br>W:toc-depth | E:toc-depth<br>W:toc-depth | ok | E:toc<br>R:toc | R:cover<br>W:cover | ok | E:cover+toc<br>R:cover+toc<br>W:cover+toc+toc-depth | W:toc-depth+links | ok | E:toc<br>R:text-extra<br>W:toc | E:toc<br>R:toc | E:toc<br>W:toc |
| cal-epub3-features-azw3 | ok | R:toc-depth | ok | W:text-extra | ok | R:cover<br>W:cover | ok | E:cover+toc-depth<br>R:cover+toc-depth<br>W:cover+toc-depth | E:toc-depth<br>R:toc-depth<br>W:toc-depth+links | E:toc-depth<br>W:toc-depth | ok | ok | ok |
| cal-epub3-features-pdb | E:title+authors<br>W:title+authors | E:title+authors<br>W:title+authors | E:title+authors<br>W:title+authors | E:title+authors<br>W:title+authors | E:title+authors<br>W:title+authors | ok | ok | E:title+authors<br>W:title+authors | E:title+authors<br>W:title+authors+text-missing | ok | ok | E:title+authors<br>W:title+authors | E:title<br>W:title |
| cal-epub3-features-pmlz | ok | ok | ok | W:text-extra | ok | ok | ok | ok | W:links | ok | ok | ok | ok |
| cal-epub3-features-rb | E:text-missing<br>W:text-missing | E:text-missing+text-extra<br>L:text-missing<br>W:text-missing+text-extra | E:text-missing<br>L:text-missing<br>W:text-missing | W:text-extra | E:text-missing<br>R:text-missing<br>W:text-missing | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | E:text-missing<br>W:text-missing | W:links | ok | ok | E:text-missing<br>R:text-missing<br>W:text-missing | E:text-missing<br>W:text-missing |
| cal-epub3-features-snb | ok | ok | ok | ok | ok | R:cover<br>W:cover | ok | E:cover<br>R:cover+links<br>W:cover | ok | ok | ok | ok | ok |
| cal-epub3-features-fb2 | E:toc<br>W:toc | E:toc<br>R:toc<br>W:toc | E:toc<br>R:toc<br>W:toc | E:toc<br>R:toc | E:toc<br>R:toc | E:toc<br>R:cover+toc<br>W:cover | E:toc<br>R:toc | E:cover+toc<br>R:cover+toc<br>W:cover | E:toc<br>R:toc | E:toc<br>R:toc | E:toc<br>R:toc | E:toc<br>R:toc | E:toc<br>R:toc<br>W:toc |
| cal-epub3-features-docx | E:toc-depth+links<br>V:epubveri<br>W:toc-depth+links | E:toc-depth+text-extra+links<br>R:cover+toc-depth<br>W:toc-depth+text-extra | E:toc-depth<br>W:toc-depth | W:text-extra | E:links<br>W:links | ok | ok | E:cover+toc-depth<br>R:toc-depth<br>W:cover+toc-depth | E:toc-depth<br>R:toc-depth<br>W:toc-depth+links | E:toc-depth<br>W:toc-depth | ok | ok | ok |
| cal-epub3-features-rtf | ok | E:text-extra<br>W:text-extra | ok | ok | ok | ok | ok | ok | W:text-missing | ok | ok | ok | ok |
| cal-epub-long-toc-mobi | ok | ok | ok | ok | E:toc<br>R:toc | ok | ok | E:toc<br>R:toc<br>W:toc | W:links | E:toc<br>R:toc<br>W:toc | E:toc<br>R:text-extra<br>W:toc | E:toc<br>R:toc | ok |
| cal-epub-long-toc-azw3 | ok | ok | ok | W:text-extra | ok | ok | ok | R:toc | W:links | R:toc | ok | ok | ok |
| cal-epub-rtl-mobi | ok | ok | ok | ok | E:toc<br>R:toc | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | E:toc<br>R:toc | W:links | ok | ok | **calibre-rereads-own-output-fails** | E:toc<br>W:toc |
| cal-epub-rtl-azw3 | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:links | ok | ok | **calibre-rereads-own-output-fails** | ok |
| cal-epub-ruby-cjk-azw3 | ok | ok | ok | W:text-extra | ok | E:text-extra<br>W:text-extra | E:text-extra<br>W:text-extra | ok | W:links | ok | ok | **calibre-rereads-own-output-fails** | ok |
<!-- matrix:end -->

## Bugs found and fixed

| Area | Bug (user impact) | Fix |
|---|---|---|
| AZW3/KF8 reader | style sheet text appeared as book text, all internal links and footnote links dead, no CSS, flat contents (FDST offset was read 16 bytes off) | real KF8 reader: FDST flows, SKEL/FRAG assembly, `kindle:pos` links, CSS flows with images, NCX tree (`mobi_index.rs`, `mobi_kf8.rs`) |
| MOBI 6 reader | contents ignored the book's NCX | NCX read; anchors via `filepos` |
| TXT / HTML encodings | cp1251, KOI8, Shift-JIS, GBK, Big5, EUC-KR, Greek, Hebrew, Arabic text became mojibake; RB pages with accents became U+FFFD | `legacy_encoding` detection in `decode_text`/`decode_xml` |
| HTML import | pictures, ids and in-page links lost, no chapters, author ignored | `.html` kept as markup, split at headings, `<meta>` author/description |
| EPUB writer | EPUB 3 output carried EPUB 2 `opf:role`/`opf:file-as` (invalid); SVG pages without the `svg` property; tag soup chapters not well formed; duplicate `playOrder` | refinements in EPUB 3, content properties, repair through an HTML parser, shared play orders |
| RTF reader | table cells and the line after a table glued into one word | cells/rows read as `<table>` |
| DOCX reader | bookmarks inside runs lost (dead links), nested `<a>` in `<a>` | bookmarks as `<span id>` |
| HTML based readers (RB, PML, MOBI…) | `<a name>` targets dropped, inline styles, ruby, `start`, image sizes dropped; blocks inside `<i>` made invalid XHTML; SVG wrapped covers lost | `html_blocks` keeps/normalises them |
| Contents | sub-headings never reached the contents; FB2 sections flattened; HTMLZ round trip lost all chapters | nested contents from headings; FB2 nesting; wrappers opened |
| Metadata | untitled files were "Untitled"/"Comic"/"index"; "A & B" stayed one author | file name as title, authors split |
| Markdown writer | `<`, `*`, `_`, `[`… unescaped: words vanished on reading back (found by the property test) | escaping |
| TXTZ writer | cover dropped | cover and OPF entry |
| Zip handling | `Archive::from_bytes` trusted declared sizes (allocation of the lie) and had no expansion limit | 512 MB per member, 2 GB total, 200k members |

## Deliberate or cosmetic differences

* Bookx's TXT/TXTZ output starts with title and author lines (Calibre: none).
* Calibre generates a cover when none exists and adds generated title/contents pages (FB2 annotation, inline
  contents page); Bookx keeps the annotation as the description and does not invent pages.
* Calibre invents contents for sources without any ("Start"); Bookx uses the headings.
* Calibre's own PML/RTF/TXT writers glue list items and table cells into one word ("alphaalpha one"); the reader
  cannot unglue it.
* MOBI 6 written by Calibre has a flat NCX; its nesting only exists in the inline contents page.
* Footnote numbers: Bookx renders `[1]` links, Calibre appends the digit to the word (`underlined1`).
* DOCX has no cover role: Bookx puts the cover on the first page, Calibre does not re-read it as the cover.
* Bookx RTF carries internal links as bookmarks; Calibre's RTF reader cannot follow them (the text shows `bx1`).
* Calibre's PalmDOC reader takes title and author from the first text lines; Bookx uses the database name.
* Image-only comics have no text: txt/pdb/rtf readers reject such files (clear message).

## Still open

* Calibre's DOCX writer puts chapter bookmarks before `<w:pPr>` and uses a TOC field instead of heading styles:
  Bookx rebuilds no contents from that (it needs the field result).
* KF8: `kindle:flow` SVG flows, fonts and audio flows are ignored; link offsets inside a tag resolve to the tag.
* MOBI 6 guide/inline-contents page is kept as a chapter.
* Single-byte encodings that look alike (cp1250 / 8859-2 / cp1252) are not told apart; they decode as cp1252.
* PDF, LIT, LRF remain best effort or unsupported (see the wiki table).
* Not run on real devices; sizes of large (>50 MB) books were not compared with Calibre.
