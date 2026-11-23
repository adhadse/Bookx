# PDF: research, decision, status (roadmap D3, C4, R17-pdf)

Internal note. User-facing text is in `wiki/docs/reading.md` (section PDF).

Status: finished, pending move to archive/ (still pointed to from enhancements.md or source comments) | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## Requirement

Bookx is GPL-3.0-or-later and Rust only. PDF has to be shown page by page (fixed layout) with text positions (selection,
highlights, search), outline, links, passwords, covers and metadata, and also reflowed to an EPUB for the library, search
and *Convert To*. It runs in a Flatpak (GNOME 49 runtime, `rust-stable` extension). Rendering must not block the UI.

## Options

| Option | Licence | What it is | Verdict |
|---|---|---|---|
| **`hayro` 0.9** (+ `hayro-syntax`, `hayro-interpret`) | Apache-2.0 OR MIT | Pure-Rust parser, content stream interpreter and CPU rasteriser (`vello_cpu`). Handles xref repair, object streams, RC4/AES-128/AES-256 decryption (the crate docs still say "no encryption": stale, the code has it), fonts (Type1, TrueType, CFF, Type3, CID with CMaps; the 14 standard fonts embedded as Foxit/PDFium `.pfb`), images (DCT, JPX, JBIG2, CCITT, flate/LZW/...), shadings, patterns, soft masks. A `Device` trait exposes glyph runs with the Unicode from `ToUnicode`/glyph names, which gives text with positions, also for invisible OCR text. Does not read the outline, labels or links (the object API makes that about 150 lines). No blending/isolation groups, knockout groups, colour key masks. `#![forbid(unsafe_code)]`. | **Chosen** |
| `pdf-extract` | MIT | Text extraction on `lopdf`, no positions API, no rendering | Not enough |
| `lopdf` | MIT | Low level object model and editing (also a writer); no interpreter, rendering or text positions | Only useful for writing PDFs (EPUB to PDF later) |
| `pdf` (pdf-rs) | MIT | Parser + a viewer example (`pdf_render`); renderer unmaintained for years, fewer font/image filters | Behind hayro |
| `pdfium-render` | MIT/Apache wrapper around PDFium (BSD-3 / Apache-2.0) | Best fidelity and speed, but a C++ binary dependency: must be built or bundled per architecture, a Flatpak module of its own (depot_tools build), `unsafe` FFI, not Rust only | Rejected: binary dependency, breaks the Rust-only rule and the build story |
| `mupdf` | AGPL-3.0 | Fast, complete | **Not allowed** (owner rule; also AGPL-vs-GPL-3 compatibility is one-way) |
| `poppler-rs` / poppler-glib | GPL-2.0-or-later (compatible with GPL-3.0-or-later) | Complete, GTK-friendly via cairo, text with positions and links built in | Licence is fine, but it is a C++ system library: the GNOME runtime does not ship `poppler-glib`, so the Flatpak needs a module (poppler + its font, openjpeg, lcms, freetype, cairo stack), it is not installed on the dev machine, and it is not Rust only. Kept as the fall-back if hayro proves too weak |
| cairo-rs only | MPL/LGPL bindings | cairo has no PDF reader | Cannot read PDFs |

## Decision

`hayro` for parsing and rendering, `bookx-pdf` for everything around it (all our own code): outline, page labels (number
tree), named destinations (dictionary and name tree), link annotations, positioned text, reading order, search, reflow,
render worker. Licence: hayro is Apache-2.0 OR MIT, compatible; the embedded Foxit fonts are BSD-3-Clause (NOTICE.md).
No system library, so the Flatpak manifest does not change. Cost: hayro needs Rust 1.92 (edition 2024); the workspace
declares 1.85 and builds with the current toolchain, and `crates/bookx-pdf` declares `rust-version = "1.92"`. The
Flatpak `rust-stable` extension is newer than that.

## How it is built

* `crates/bookx-pdf` (GTK-free): `Document` (open with password, info, outline, labels, links, text), `Renderer`
  (page to RGBA, PNG, content box for cropping), `PageText` (characters with boxes, lines, XY-cut reading order,
  hit testing, ranges to rectangles, search, anchoring a stored quote), `reflow` (paragraphs, headings, header/footer
  removal, hyphen joining, chapters from the outline or headings), `Worker` (a thread that owns the document; requests:
  render, page text and links, search, crop; cancellation tokens; newest render first; panics inside hayro are caught
  and reported as a failed page), `PageCache` (LRU with a byte budget), `testpdf` (the writer used by the tests).
* `crates/bookx-formats/src/pdf.rs`: PDF to EPUB for the importer, CLI and converter. Title and author from the
  document information (junk titles such as "Microsoft Word - x.docx" or "Untitled" are dropped) or from the file name
  (`Author - Title.pdf`), page 1 as the cover, `bookx:reflowed-from` marker. A scan, a password-protected file or a PDF
  with nothing to reflow still imports with a one-chapter note, so the PDF can be opened in the page view.
* `bookx-core` `commit_import`: when the source file is a PDF it is added as a further format of the book, so the library
  holds the reflowed EPUB (search, indexing, devices, Convert To) and the original PDF (page view).
* `src/pdf.rs` (pure, tested): layout (fit width/page/percent, continuous or single, crop, centring, visible range),
  zoom steps, render scale quantisation, page labels in the page box, hit stepping, annotation encoding, settings.
* `src/ui/reader_pdf.rs`: the page. A cairo `DrawingArea` the size of the window with its own scroll model (two
  `gtk::Adjustment`s), so a 500 page document never makes a huge widget. Pages are requested for the visible part and a
  screen around it, come back from the worker, are kept as cairo surfaces in the cache; the previous rendering of a page
  is drawn scaled while a new one is rendered; stale requests are cancelled.
* Annotations: stored in the library as `Highlight` with `spine = page`, `start`/`end` = character range of the page
  text, `text` = the quote, `style = pdf|<style>|x0,y0,x1,y1;...` (rectangles in points). The quote re-anchors the range
  if text extraction ever changes; the rectangles are drawn as stored. The EPUB reader and the highlight list ignore them,
  export (`annotation_export`) labels them "page N".

## Measurements (release build, this machine, `crates/bookx-pdf/tests/perf_budgets.rs`)

All five budgets pass in release and debug; the whole file runs in 0.18 s release. Budgets: open a 500 page PDF with
outline and labels 400 ms; 5 pages rendered at 1.5x 400 ms; text of 5 pages 150 ms; reflow of 120 pages (extracted text) 300 ms;
search in 200 pages of cached text 300 ms. The test PDFs are text only: a page full of large images will be slower
(JPEG/JPX decoding on one CPU thread) and memory is `width x height x 4` per cached page (cache budget 384 MB).

## Text extraction quality

Characters come from glyph runs: `ToUnicode` first, then glyph names. Box width is the distance to the next glyph origin
(hayro gives no advance), height from the font size, so boxes are right for horizontal Latin text and approximate for
everything else. Ligatures split their box. Spaces are inferred from gaps (0.18 em) because many PDFs have none.
Reading order: rows by baseline, split at gaps wider than 2 em, then recursive XY-cut (vertical gutter first, then
horizontal gaps) so that columns are read one after the other and a full-width heading comes first. Known weak spots:
tables (cells become segments; reading order can go down a column), footnotes in the page body, drop caps, rotated and
vertical text (treated as horizontal boxes), fonts without a Unicode mapping (the glyph is skipped), text drawn as paths.

## Encrypted PDFs

RC4 40/128 and AES (V4, V5) are handled by hayro with the empty user password tried first. The password prompt
(`AdwAlertDialog`) re-asks on a wrong password. Tested against our own RC4-40 writer only (R2); AES-128 and AES-256 files
are **not** tested here. Documents with certificate (public key) encryption or Adobe DRM (`Adobe.PubSec`) are refused; the
importer's DRM detection already names those.

## What is not done / not tested

* **OCR**: design note only. No pure-Rust OCR worth shipping exists (`ocrs` is MIT and pure Rust but needs two ONNX models
  of about 12 MB, English-centric, slow on CPU, no layout). Plan: a plugin hook (the wasm plugin host has transforms) that
  receives the rendered page and returns text with boxes, which would be stored as an invisible text layer next to the
  PDF and feed the same `PageText`; or `tesseract` as an optional system program like Hunspell, never linked. Until then
  a scan imports with a note and the page view works without text.
* EPUB to PDF (C7): not required; `lopdf` or `krilla` with the canvas layout would be the route.
* No real-world PDF corpus in the tests (generated files only). Not verified: papers typeset with LaTeX (Type1/CM fonts
  with odd encodings), scans with an OCR layer, PDFs with Type3 bitmap fonts, linearised and hybrid-reference files,
  PDF 2.0, AES files. hayro's own regression suite covers over 1000 files from pdf.js and PDFBox, which is the evidence for
  the renderer; the extraction and reflow heuristics are only tested on synthetic layouts.
* Selection across pages, form fields, annotations other than links, layers (OCG) toggling, rotated pages are shown but
  not interactive, text-to-speech for PDFs, KOSync position for PDFs (the progress is a page, not an EPUB spine
  position), device sync of PDF highlights (they are stored with `spine = page`; `device_sync` and `send_notes`
  assume EPUB spine numbers and should skip `pdf|` highlights, not changed here because those files belong to other tracks).
* The command palette shows library commands while a PDF is open (the PDF page registers no `reader.*` actions).
* Chrome auto-hide of the EPUB reader is not in the PDF page (bars stay, as in e-ink mode).
* Touch: pinch zoom and one finger panning are wired but untested on a touch screen.
