# Hardware test plan

What to run on real devices before a release. Nothing here can be automated in CI: the checks need a Kindle, a Kobo, a
PocketBook, a Boox, a touch screen and an e-ink panel. Everything the computer side can verify is verified by
`bookx-cli device-check`, so run that first and attach its output to every report.

## 0. Setup for every device

1. Build or install the release: `./run.sh --no-run` (installs `bookx` and `bookx-cli`).
2. Make a small test library (once): `bookx-cli --library ~/bookx-hw library add ~/some-epubs/` (add any five
   books: a plain novel, one with images and a table, one with RTL text, one fixed-layout/comic, one 20 MB book).
3. Connect the device with USB and let it mount. Note the mount point (`findmnt -t vfat,exfat,ext4 | grep -i -E "kindle|kobo|pocket|boox"`).
4. Confirm the layout before copying anything:

   ```bash
   bookx-cli device-check /run/media/$USER/KOBOeReader
   ```

   Expected: `device kind: Kobo`, exit code 0 (warnings are fine on a fresh device, for example *books folder does not
   exist yet*). Exit code 1 means the folder layout is not recognised: stop and report the output.
5. Start a log file for the session: `script -q hw-<device>-<date>.log` (stop with `exit`). Keep the device model,
   firmware version (Kobo: `.kobo/version`, Kindle: Settings > Device Info, PocketBook: Settings > About, Boox: Settings
   > About tablet) and the Bookx version (`bookx-cli --version`) at the top of the report.

Always collect, for any failure: the output of `device-check --json <mount>`, the file as copied (`ls -l` and `sha256sum`
on the computer and on the device if you have a shell), a photo or screenshot of the device screen, and
`RUST_LOG=debug bookx` stderr if the failure is on the computer side.

## 1. Kindle (AZW3 / MOBI)

Models to cover: one e-ink Kindle with firmware 5.16 or newer (Paperwhite/Oasis/Basic), one older Kindle (Kindle 4/Touch,
firmware 4.x/5.x) if available.

| # | Steps | Expected | Collect on failure |
|---|---|---|---|
| K1 | Convert: `bookx-cli convert novel.epub novel.azw3 --device kindle`, then the same to `.mobi`. | Exit 0, files non-empty. | Command output, input file. |
| K2 | Copy both into the `documents/` folder on the mount (or use Bookx *Send to device*). Run `bookx-cli device-check <mount> --book novel.azw3`. | `valid PalmDB/MOBI container (MOBI header version 8)` for AZW3, version 6 for MOBI. `PASS`. | `device-check --json` output. |
| K3 | Eject, wait for the Kindle to index. Open the home screen. | The book appears with the right title, author and cover (cover shows after indexing, up to a minute). | Photo of the library list; `documents/` listing; a `.sdr` folder appearing is normal. |
| K4 | Open the book. Page through the first chapter, open the table of contents (Go To), jump to a chapter. | Text is readable, no raw HTML or `&amp;` entities, chapters in the TOC match the book, jump lands on the chapter start. | Photo of the offending page, chapter name. |
| K5 | Check formatting: italics, bold, a blockquote, a footnote link, an image, a table. | Italic/bold render, blockquote is indented, footnote link opens a popup and returns, image is scaled to the screen width, table is readable. | Photos; the source XHTML of that chapter (`bookx` editor). |
| K6 | Use the Aa menu: change font size to the largest and back; change the font. | Text reflows, no pages lost, reading position stays near the same text. | Photo before/after. |
| K7 | Highlight a sentence, add a note, close the book, reopen. | Highlight persists; position resumes. | None expected. |
| K8 | Send an EPUB directly (newer firmware only): copy `novel.epub` into `documents/`. | `device-check` prints a warning that EPUB needs recent firmware. If the Kindle refuses it, that is expected on old firmware; record the firmware version. | Firmware version. |
| K9 | Unicode file names: send `Ünïcode — Тест.azw3`. | `device-check` passes; the Kindle shows the title from metadata, not the file name. | Photo. |
| K10 | Large book (20 MB AZW3). | Opens in under 10 s on Paperwhite class hardware; no reboot. | Time, model. |

## 2. Kobo (KEPUB)

Models: one current Kobo (Clara/Libra/Sage/Elipsa), one older (Aura/Glo) if available.

Done so far: Kobo Libra Colour, firmware 4.46.23852, read-only study and a sandbox send (no eject); see
`docs/kobo-real-device-report.md` for what that covered and which of B3-B9 are still open.

| # | Steps | Expected | Collect on failure |
|---|---|---|---|
| B1 | `bookx-cli convert novel.epub novel.kepub.epub --device kobo`. Check: `unzip -p novel.kepub.epub OEBPS/*.xhtml | grep -c koboSpan`. | Exit 0, count greater than 0. | Command output. |
| B2 | Copy into `Bookx/` on the mount (the *Send to device* folder). Run `bookx-cli device-check <mount>`. | `KEPUB markup present`, `library database present`, `PASS`. A plain `.epub` gives the warning *send KEPUB*. | `device-check --json`. |
| B3 | Eject. The Kobo shows *Processing* and imports the book. | Book appears with cover, title and author within a minute; no `.kobo/KoboReader.sqlite` corruption (the device does not reboot loop). | Photo; `.kobo/Kobo/Kobo eReader.conf` is not needed, but the firmware version is. |
| B4 | Open the book, page through, open the contents. | Reading progress bar and "x min left in chapter" are shown (these only appear for KEPUB). | Photo. |
| B5 | Fonts: pick a built-in font and a sideloaded font (`fonts/` folder on the device, if used). Change size and line spacing from the Aa menu. | Reflows, setting persists. | Photo. |
| B6 | Highlight and annotate. Eject, run `bookx-cli device-check <mount>` again. | Still `PASS`; the database file is untouched by Bookx (it is only written when a shelf name is set in the Device window; reading it is done on a copy). | Output. |
| B7 | Send the same book as plain EPUB. | Opens, but without the chapter progress info. | Photo. |
| B8 | Dictionary / word lookup on a long press (Kobo's own). | Works inside the KEPUB (this verifies the spans are placed on words). | Photo. |
| B9 | Book with images and a fixed-layout or comic page. | Images scale; fixed-layout books display page by page. | Photos, file. |
| B10 | Wi-Fi/OPDS: start `bookx-cli serve`, add the catalog in KOReader/Kobo browser (`http://<host>:8080/opds`) if the firmware has a browser. | Catalog lists books; download works. | Server log. |

## 3. PocketBook

Models: one recent (Verse/Touch HD/InkPad), firmware 6.x.

| # | Steps | Expected | Collect on failure |
|---|---|---|---|
| P1 | `device-check <mount>`. | `device kind: PocketBook` (it has `system/config`), `PASS`. | Output. |
| P2 | Copy `novel.epub`, `novel.fb2`, a PDF into `Books/`. | `device-check` passes all three. | Output. |
| P3 | Eject; the device scans the library (may take a minute). | The books appear under *Library*, with covers; the title and author come from metadata. | Photo. |
| P4 | Open the EPUB: TOC, bookmarks, font change, hyphenation, dictionary lookup. | Text reflows, TOC navigates, hyphenation follows the book language. | Photo. |
| P5 | EPUB 3 specifics: a book with a nav document, footnotes (`epub:type=noteref`), and embedded fonts. | TOC from nav, footnotes pop up, embedded fonts used when "publisher style" is on. | Photos. |
| P6 | Unicode and long names (200 characters). | `device-check` passes; the device lists the book. | Output. |

## 4. Boox (Android e-ink)

Models: one Boox running Android 11 or newer (Note Air/Go/Tab Ultra).

| # | Steps | Expected | Collect on failure |
|---|---|---|---|
| X1 | Connect USB in file-transfer mode. The Boox mounts as MTP on many models, so mounting a folder may be impossible; use the *USB storage* option if the device has it, otherwise copy through the file manager of your desktop and run `device-check` on a local copy of the `Books` folder structure. | `device-check` passes for a folder containing `Books/`. | Output. |
| X2 | Put `novel.epub` and `novel.cbz` in `Books/`. Open Neoreader (built in). | Both listed; EPUB opens with correct styles; CBZ opens page by page. | Photo. |
| X3 | Refresh behaviour: page through 30 pages in the default and in the "A2/fast" refresh modes. | No ghosting artefacts from pages that Bookx produced (all rendering is done by the Boox reader). | Photo of ghosting, mode. |
| X4 | Serve over Wi-Fi: `bookx-cli serve --bind 0.0.0.0`, open `http://<host>:8080/opds` in the Boox browser or in KOReader for Android. | Catalog pages load; a book downloads and opens. | Server log, browser screenshot. |

## 5. The Bookx app on touch screens

Devices: a laptop/convertible with touch, a tablet or phone running a Wayland session (Phosh/Plasma Mobile) if available.

| # | Steps | Expected | Collect on failure |
|---|---|---|---|
| T1 | Open a book. Tap the right third, the left third and the middle. | Right = next page, left = previous, middle = toggles the toolbars (see Preferences > Gestures for the active map). | Screen recording; Preferences gesture settings. |
| T2 | Swipe left/right across the page. | Page turns, no accidental text selection. | Recording. |
| T3 | Long-press a word, drag handles, release. | Selection menu appears (Highlight, Note, Dictionary, Translate, Copy); the menu buttons are at least 44 px high. | Photo. |
| T4 | Pinch to zoom on an image; double-tap an image. | Image opens full-screen and can be dismissed by tapping. | Recording. |
| T5 | Library: scroll 1,000 covers with a finger, flick fast. | Scrolling follows the finger without stutter (about 60 fps), covers appear without blank tiles for more than a moment. | Recording; number of books; `perf`-style timing if available. |
| T6 | Rotate the screen (or resize the window to 360 px wide). | Layout switches to the narrow layout, no clipped buttons, the reader keeps its position. | Screenshots before and after. |
| T7 | On-screen keyboard: open the search box in the editor and the reader. | The text field stays visible above the keyboard. | Photo. |
| T8 | Stylus/pen (if available): select text with the pen. | Same as touch. | Recording. |

## 6. E-ink mode of the Bookx app

On an e-ink monitor or a Boox running a Linux desktop session (or any screen with *Preferences > E-ink mode* enabled
to check the behaviour, ghosting needs a real panel).

| # | Steps | Expected | Collect on failure |
|---|---|---|---|
| E1 | Enable E-ink mode in Preferences. | Animations are off (no page transitions, no fades), high-contrast theme, no gradients. | Screenshot. |
| E2 | Turn 50 pages in a row. | No animation frames between pages; a full refresh happens on the configured interval (every N pages), not after every page. | Photo of the panel; count of full refreshes. |
| E3 | Highlights, popovers and menus. | They are drawn in black/white/grey with 1 px borders; no translucency. | Photo. |
| E4 | Leave the reader idle for 1 minute. | No clock or progress animation redraws the panel (battery and ghosting). | Observation. |
| E5 | Switch E-ink mode off. | Normal theme and animations return. | Screenshot. |

## 7. Reporting

One file per device: `hw-<device>-<yyyy-mm-dd>.md` with the header (model, firmware, Bookx version), a table of the
test ids with PASS / FAIL / SKIPPED, and for each failure the data under *Collect on failure*. Attach the
`device-check --json` output for every device, before and after the copy. Product bugs go to `docs/bugs.md`.

## 8. Device zoo (emulated, no hardware)

`crates/bookx-serve/tests/common/zoo.rs` lists popular readers as fixtures (`ZOO`); `tests/device_zoo.rs` and
`tests/sync_rule.rs` run them against the real server and the real device drivers. **Everything is emulated.
Nothing in the zoo was observed on a real device**, except that the Kobo Libra Colour model id (`...390`) and
database schema in the repository were read from the owner's real device earlier; every `reported` name comes from
KOReader's source (`Device.model`: `koreader/frontend/device/<brand>/device.lua`, `android-luajit-launcher`
`DeviceInfo.kt`) or, for `Onyx Scubapilot-IDP`, from the report of issue #39.

| Family | Entries | Looks like | Sync client sends |
|---|---|---|---|
| Boox (Android) | Leaf 2 (as in issue #39 and as KOReader names it), Go 7, Palma 2, Note Air 3C, Tab Ultra C | MTP (gvfs tree `mtp:host=ONYX_...`) | `Device.model` (Build.PRODUCT) |
| Kobo | Clara 2E, Libra 2, Libra Colour, Sage, Elipsa, Clara BW | `.kobo/KoboReader.sqlite` + `.kobo/version` (ids other than 390 are placeholders, shown as "model id") | `Kobo_<codename>` |
| Kindle | Paperwhite 5, Oasis 3, Scribe | `documents/` + `system/version.txt` | `KindlePaperWhite5` ... |
| PocketBook | Verse, InkPad 3, Era | `system/config` + `Books/` | `PB629` ... |
| reMarkable | 2 | no disk | `reMarkable 2.0` |
| Unknown | an Android phone, an unlisted Kobo codename | MTP / none | shown as reported, never guessed |

Each entry is synced twice: as the Bookx plugin does (headers `X-Bookx-*`, annotations) and as stock KOReader
*Progress sync* does (no Bookx headers). The checks: the device row shows the name on the box
(`device_names::friendly_name`), the raw name is kept, a rename wins and survives the next sync, the USB layouts are
detected and books go to the right folder and format, and no sync action touches a library file.

**Add a device:** one `r(...)` entry in `ZOO` with the slug, family, the name Bookx must show, what KOReader
reports, its `Usb` layout and the source of the reported string; add the model to the tables of
`crates/bookx-serve/src/device_names.rs` (with its source) when Bookx should know it. Test it on the real device
and set `seen_on_hardware` only then (the zoo test currently asserts none are).

