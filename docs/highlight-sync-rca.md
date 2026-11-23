# Highlight colour and style through sync: root causes, fix and repair

Internal (2026-11-21). User text: `wiki/docs/annotations.md` ("Colours and styles from your e-reader") and
`wiki/docs/koreader-sync.md`. Earlier rounds: `docs/bugs.md` 12 and 18. Line numbers below are of the tree *before* this
change (`git show HEAD:<file>`) unless marked "now".

Status: living | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## The report

Highlights that come through Bookx sync (the `/bookx` KOSync extension between the KOReader plugin and Bookx, the USB
`.sdr`/Kobo path) arrive in the wrong colour and/or style, in one or both directions, and highlights already stored in the
owner's libraries are wrong.

## What the real data shows

The owner's library (`~/Documents/Bookx Library`, read-only copy; **never modified by this work**): *Building Microservices* has
400 highlights, **all** `yellow` with style `""`. The device (`/tmp/opencode/kobo/dev`, a copy of the Kobo's
KOReader sidecars) has 409 annotations: 282 yellow (12 underscore), 56 purple, 43 cyan, 10 olive, 5 orange, 6 red (5
underscore), 4 green. There is no `.bookx/device-looks.json` and the ledger `annotation-sync.json` is empty (101 bytes), no
`koreader-annotations.json`: the library was filled before the device looks were understood and no sync has run since.

On sandbox copies of that library I ran the current code paths with the real sidecars:

| Run | Result |
|---|---|
| USB import of the device (`kobo_real_check`, before this change) | colours and styles correct, 131 highlights corrected |
| Plugin logic (luajit) → built-in server → merge (`kobo_real_wireless_check`) | correct, 402 linked, 4 kept as notes without a place |
| New `bookx-cli highlights repair --from <device>` | 131 corrected, 269 already right, 5 device rows with no library highlight (nothing created) |

So the *happy path* of the current code is right; what hurt the owner is (a) data stored before, which nothing
corrected unless a device sync or import was run by hand, and (b) the paths below that are wrong in less common situations.

## Root causes, ranked

**R0. Highlights stored before the fix stayed wrong.** The correction of bug 12/18 only ran when a device was read (USB
sync, *Import Highlights*) or when the server's change number moved. `annotation_sync_ui::sync_once` returns early when
`ledger.seen == seq` (`src/annotation_sync_ui.rs:125,131`), and a library opened with no device attached did nothing. The
owner's library therefore still had 400 yellow highlights. *Fix*: a one-time pass on library open
(`highlight_repair::look_pass` → `look_repair::one_time`) and `bookx-cli highlights repair`.

**R1. A second, different mapping table on the wireless path** (`src/annotation_sync.rs:213-249`): `color_in` turned any
word `devstyle` did not know into yellow; `color_out` sent anything but eight names, including a custom `#rrggbb` and
Bookx's own non-palette words, as `yellow`; `style_in` returned `""` for `lighten` and for every unknown drawer, and
`drawer_out` sent unknown styles as `lighten`. The USB path used `devstyle` (which dropped unknown words as "the device said
nothing", `devstyle.rs:63-84`, `look_of:147`). Two tables meant the same highlight looked different depending on the road it
took, and an unknown word was lost on both. *Fix*: one table (`bookx_core::devstyle`), unknown words kept verbatim
everywhere, the wireless functions are thin wrappers.

**R2. Bookx → device lost colours Bookx has no swatch for** (`annotation_sync.rs:218-230`, `send_notes.rs`): a custom colour
went out as yellow (`devstyle::to_koreader_color` mapped a custom RGB to yellow as well, because `HighlightColor::Custom`
has no name). *Fix*: nearest KOReader colour by RGB distance; unknown words verbatim; the sidecar written for KOReader uses the
same function.

**R3. Short quote inside a longer highlight took the longer highlight's colour** (USB path,
`src/device_sync.rs:464-465`): the device rows were matched in file order, `syncplan::same_highlight` treats a quote that is part
of a longer one as the same highlight, so the short row claimed the long library highlight, gave it its colour (the long one
still had the default look) and the long row then found nothing and was added again. `annotation_import::plan` and
`annotation_sync::merge_incoming` already sorted longest first; this path did not. *Fix*: longest first (stable) in the
look loop. Test `device_sync::a_short_quote_inside_a_longer_highlight_does_not_take_its_colour` fails without it.

**R4. United copies kept the copy without a colour** (`src/duplicates.rs:172`): merging duplicate books united highlights by
id and kept the kept book's copy, so a copy that only had Bookx's default look hid the colour and style the other copy got
from the device. *Fix*: `devstyle::take_look_if_default` (a look somebody chose is never replaced; the default never
replaces anything); an empty note takes the other copy's note. Test `duplicates::merging_keeps_the_colour_and_style_of_the_copy_that_has_them`.

**R5. Every spelling of a look was a change** (`annotation_sync.rs:739-744`, `import.rs:601`): the outgoing fingerprint
compared raw words, so `""` → `background` or `Yellow` → `yellow` re-sent the highlight to the device with `updated = now`
(which can win a conflict over a real edit); the importer's `dedupe` treated KOReader's `lighten` and Bookx's `background`
(or `grey`/`gray`) as different looks and kept both copies of one highlight. `annotation_sync::same_color/same_style`
fixed the same comparison for the incoming side (an unknown style was "changed" on every merge). *Fix*: all
comparisons go through `devstyle::full_look`; `import::look_word` folds the aliases.

**R6. The pre-2024 KOReader file lost every style** (`reading.rs:135`): KOReader before 2024 keeps `drawer`/`color` in the
`highlight` table (page → list) and only text and note in `bookmarks`. `parse_sidecar` read only `bookmarks`, so every old
highlight came in plain. *Fix*: the `highlight` table is joined by `datetime`. Fixture `koreader_legacy_highlights.lua`.

**R7. Unknown colours, drawers and Kobo numbers were defaulted** (`devstyle.rs:41-47,63-84`, `bookxsync.lua:57,73`): the plugin
dropped any `drawer` it did not know to `""` (a newer KOReader adds drawers and colours); `kobo_color(n)` was `None` above 3;
`canonical_color` returned `None` for any word. *Fix*: plausible words are kept as written (≤ 32 chars, no control characters,
no `|`), Kobo numbers above 3 become `kobo-<n>`, the plugin sends the word verbatim (control characters stripped, 32
characters, so one odd value cannot make the server refuse the whole request, which it does for > 32).

**R8. The export used another style table** (`src/annotation_export.rs:64-75`): `underscore`/`strikeout` stored as such exported
as plain. *Fix*: `devstyle::style_from_any`.

### Checked and found fine (no change)

* Server upsert (`bookx-serve/src/annotation_sync.rs:341-348`): a colour-only change with an unchanged `updated` is "stale" by the
  rule, but the plugin (`bookxsync.lua:102`) and Bookx (`annotation_sync.rs` outgoing) bump `updated` whenever the fingerprint
  changes, which includes colour and drawer. Not changed; documented here because a client that did not bump would lose a recolour.
* Server stores `color`/`drawer` verbatim (≤ 32 characters); the wire format is unchanged. The released plugin keeps working with
  this Bookx and the new plugin with an older Bookx (a verbatim drawer is stored; an old Bookx would read it as default).
* Kobo `Color` 0..3 = yellow, pink, blue, green is from community documentation of the column (and the schema in
  `tests/fixtures/kobo-schema-176.sql` has `Color INTEGER DEFAULT 0`); **the owner's Kobo has no highlights made with Kobo's own reader**
  (`Bookmark` is empty), so the mapping could not be checked on the device.
* The reader paints through `devstyle::display_rgb`; an unknown word is painted in the picker's yellow (`DEFAULT_RGB`).

### Not fixed, needs the device

* Bookx writes the KOReader notes file with the old `bookmarks` layout (`send_notes::koreader_sidecar`). KOReader ≥ 2024
  migrates `bookmarks` into annotations and takes the `drawer` from the `highlight` table, which Bookx does not write, so on the
  device a Bookx-made highlight may open as a plain bookmark. Writing both tables was not attempted without being able to run
  KOReader. (Wi-Fi sync is not affected: the plugin lists Bookx's highlights in its own inbox and does not paint them.)
* KOReader's rendering of a highlight with no `color` is not known to be gray or yellow; Bookx takes "no colour" as "the device
  said nothing" and keeps its own look.

## The model now (`crates/bookx-core/src/devstyle.rs`)

* One table (`KOREADER_COLORS`, `KOREADER_DRAWERS`, `KOBO_COLORS`), tested for every colour and drawer in both directions
  (`every_koreader_colour_and_drawer_round_trips`, `every_kobo_colour_round_trips`).
* Stored words: Bookx colours (`yellow orange red pink purple blue green gray`), KOReader extras (`olive cyan`), `#rrggbb`, an unknown
  word as written, `kobo-<n>`; styles `background underline strikethrough squiggly invert`, an unknown word as written. `grey` is `gray`.
* Lossy only where the target has no word: pink → red, squiggly → underscore, a custom colour → nearest KOReader name, Kobo: nearest of
  its four. Nothing is lossy inside Bookx.
* Merge rule: a side that says nothing never downgrades the other (`look_of` gives `None` for "nothing"); the default look is the only one
  anything may replace.

## Repair (`crates/bookx-serve/src/look_repair.rs`)

Sources, strongest first: rows that carry the highlight's text (a device folder with KOReader `.sdr` sidecars and `.kobo/KoboReader.sqlite`,
one sidecar or database, the raw records of `koreader-annotations.json`), then ledgers (`annotation-sync.json` `device_look` and
`.bookx/device-looks.json`, which name the highlight itself).

* Matching: by the words of the highlight (`syncplan::norm`), the book by `syncplan::pair`; two highlights with the same words by creation
  time, else by "every row says the same", else counted as *ambiguous* and left. Nothing is created, nothing deleted.
* Only a highlight with the default look changes, only the parts the device said; a look the reader chose stays (`kept`); the ledger guards
  a look changed after the device was last read. Non-canonical spellings (`lighten`, `Grey`) are normalised.
* One undo step (`record_highlight_edit`, `OpKind::RepairHighlights`); the library is saved; no device ledger is written (a corrected
  highlight is no longer default, which protects it, and an undo leaves nothing that stops a second run).
* **On library open**: `highlight_repair::one_time_pass` (called from `app.rs:180`, unchanged) first runs `look_pass`: sources are the settings
  folder (store + ledgers) and every mounted USB reader. The marker `.bookx/highlight-looks-v1` is written only after a source said
  something, so a library with nothing to learn from is looked at again next time (cheap, nothing changes) and one that was repaired is left alone.
* **CLI**: `bookx-cli highlights repair [--from PATH]... [--config-dir DIR] [--apply] [--json]`; the default and `--dry-run` change nothing.
  `library undo` takes an `--apply` back (checked).
* Real data: 131 corrected, 269 right, 5 device rows without a library highlight, on a sandbox copy; the second run changes 0; `undo` restores 400 yellow.

## Security checklist (docs/security.md §8)

* Untrusted bytes: KOReader `metadata.*.lua` (the existing data-only Lua reader `lua::parse`, no code is run; ≤ 64 MiB per file), the
  Kobo database (private copy, the existing reader), `koreader-annotations.json` (existing store parser, ≤ 64 MiB), ledgers
  (`serde_json`, ≤ 64 MiB). Scan budget: 100 000 entries, depth 8. Tests: `a_device_folder_is_read_without_following_links_or_huge_files`
  (symlink not followed, garbage not an error).
* Path from a file or the network: none is joined; paths come from the command line or the mounted device root and `symlink_metadata`
  rejects links.
* Executes something: no. Listener / URL: no. New dependency: none. Secret: none.
* Regression tests fail before the fix: see the list in `docs/koreader-annotation-sync.md` ("Look tests").
* Plugin wire change: `drawer`/`color` are passed verbatim, ≤ 32 characters, control characters removed; the server already limits both to 32.
