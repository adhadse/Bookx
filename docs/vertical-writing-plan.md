# Vertical writing in the reading canvas (PP-G-21, roadmap R4 and R56)

Status (2026-11-13): **built**. This file began as the plan written before the work; it now records what was
built, where the plan was wrong or too coarse, what was measured, and what is not done.

User-facing description: `wiki/docs/reading.md`, section *Vertical writing*. Test books: `docs/test-books.md`.

## What works

| Feature | How |
|---|---|
| `writing-mode: vertical-rl / vertical-lr / horizontal-tb`, `-epub-writing-mode`, `-webkit-writing-mode`, the old `tb-rl`, `tb-lr`, `lr-tb`; `text-orientation: mixed / upright / sideways` (+ prefixed); `text-combine-upright: none / all / digits 2..4` (+ `-epub-text-combine: horizontal`) | `bookx-text::css` (values.rs, cascade.rs: inherited, so they work on `html`, `body`, a `div` or an inline style). `Doc.writing_mode` / `Doc.orientation` come from `html`/`body` (or the first vertical block), every `BlockCss` carries its own. `Style.ruby` and `Style.combine` mark spans. |
| Ruby (`<ruby>`, `<rb>`, `<rt>`, `<rp>`, `<rtc>`, several bases per ruby, rb/rt pairs in order) | `bookx-core::html::walk_ruby`: the annotation goes on the style of its base text, `<rp>` is dropped. Not part of the canonical text (selection, search, TTS, accessibility never see it). |
| Margins of a vertical block | `block_css`: the right/left margins are the block-start/end margins, the top margin is the indent of the line. |
| Transposed flow and pages | `bookx_text::vertical::Frame` (screen mapping) and `vertical::paginate` (page cuts, used for horizontal text too). A page is a stretch of the flow as wide as the page. |
| Glyphs | Pango context with `set_base_gravity(East)` and gravity hint `Natural` (mixed) or `Strong` (upright); `sideways` uses the normal context. The layout stays *horizontal* in its own coordinates; the page is drawn line by line in a cairo frame turned 90° clockwise (`canvas::line_frame`). The font's `vert` forms come for free (brackets turn, `。` moves, `ー` stands up). |
| `vertical-lr` | The same frame with the columns stacked the other way: each line is placed by itself (one turned frame cannot do both orders). |
| Tate-chu-yoko | A Pango shape attribute reserves one em for the combined characters (empty placeholder glyphs, no shape renderer is set so nothing is drawn); the digits are drawn on top, turned back, centred in that em, scaled down when wider than an em (`canvas::decorate`, `draw_decorations`). `bookx_text::vertical::combine_ranges` decides which characters. |
| Ruby drawing | Pango has no ruby: `draw_decorations` lays the annotation out with half the font size and centres it over the base range of its line (`index_to_x`), above in horizontal text and to the right in vertical text (glyphs of a vertical line are centred on the baseline, so the offset is half an em plus half the ruby). A base that wraps takes its annotation along in proportion. An annotation wider than its base spreads the base with a letter-spacing span (`vertical::ruby_spread`). Paragraphs with ruby get at least 1.7 (horizontal) or 1.9 (vertical) line spacing. |
| Kinsoku | Pango follows UAX #14, which already keeps closing brackets, `、。`, `？！`, `々`, `ゝ` off line starts and opening brackets off line ends; it allows a break before small kana and `ー` (class CJ, "loose"). `decorate` forbids those too with an `allow-breaks=false` attribute over the character and the one before it (`vertical::strict_no_break_before`). `vertical::violations` is the oracle: a Pango test for every width from 6 to 14 em, and the Broadway scenario counts violations in a whole chapter (`kinsoku=0`). |
| Punctuation squeezing | `vertical::squeezed` finds the first mark of `」「`, `。」`, `「「`, `、）`...; `decorate` asks the font for its half-width forms (`halt`, `vhal`) on exactly those characters. Pango does not run `chws`/`vchw` (measured: no change in width); `halt`/`vhal` on the whole text would squeeze every bracket. |
| Selection, hit testing, highlights, search matches, read-aloud highlight, annotations, links | All go through `to_flow` (screen to flow position and position along the line), `visible_range_rects` (flow to screen rectangles) and `visible_columns` (frames in reading order). Underline, strike-through and squiggly highlights and the note bar are drawn on the right of the column (underline) or down its middle (strike-through). |
| Page order and turning | A vertical-rl book shows its first page on the right of a two-page spread (`page_of_column`). Arrow keys, tap zones and swipes follow the package's `page-progression-direction` as for Arabic (`rtl`); a Japanese vertical EPUB must say `rtl`. In vertical text Shift+Down/Up select by character and Shift+Left/Right by sentence (`reader.rs`). |
| Scroll mode | The whole width is a window onto the flow (`scroll_frame`), a page down moves 90% of the width; wheel and edge-overscroll as before. |
| Reading position, progress | Unchanged: they use the position in the flow (`y`), which does not know about directions. |
| Accessibility | Page text is made from the lines in flow order, so it is in logical order without the ruby; the geometry callbacks (`range_rect`, `hit`) return screen boxes, checked with AT-SPI character extents in the real session. `a11y.rs`: a sentence takes the closing `」`, `）` after `。` along. |
| Publisher styles off | A vertical book stays vertical: `OpenBook::chapter_doc_layered` reads the writing mode from the book's sheets even when they are not applied. |
| Images | Stand upright on a vertical page: their height runs along the line, their width across the flow. |
| Budgets | `bookx-text/tests/perf_budgets.rs` (100 000 columns paginated, 100 000 characters squeezed and combined, 100 000 breaks checked, a million rectangles mapped); Broadway `vertical_budget` (a chapter of 100 000 characters with 1 200 ruby annotations and 900 tate-chu-yoko runs: layout 0.2 s in a debug build). |

## Where the plan was wrong or too coarse

* **"One transform per column"** (plan, Design table): not enough. A single turned frame stacks the lines of a paragraph from
  right to left, which is right for `vertical-rl` only. `vertical-lr` needs the other order, so every line is drawn with its own
  `translate`/`rotate` (`line_frame`, `show_layout_line`). That also lets the drawing skip the lines that are off the page.
* **"Pango lays glyphs out in the rotated frame, `set_width` is the column height"**: half true. With gravity East Pango keeps the
  *layout* horizontal (lines stack along y, a line is as long as `width`); only the glyphs are turned (CJK upright with the
  `vert` forms, Latin sideways when the hint is Natural), and the baseline is the *centre* of the em box. That is exactly what the
  "rotate, do not rewrite" idea needs, so the old horizontal flow code is reused unchanged and `width` is the column height.
  Measured with a small PyGObject script before any Rust was written.
* **Pagination is not canvas code any more**: it moved to `bookx_text::vertical::paginate` (same algorithm, orphan/widow rule
  included) so the page maths of both directions is tested without GTK.
* **Knuth-Plass** is not adapted: it is switched off for vertical text and for paragraphs with ruby (its forced breaks and edits
  would move the annotation ranges). Pango's justification already spreads CJK lines evenly across characters, which is what
  Japanese typesetting does (*kintou-wari*); lines that kinsoku leaves short are spread too (see the limits below).
* **tate-chu-yoko** needed a shape placeholder: Pango lays a line out along one axis, and the digits have to run across it, so no
  font feature can do it (`hwid` makes the glyphs narrow, it does not turn them; not tried).
* **Squeezing** cannot be a uniform letter spacing: Pango splits the spacing before and after the glyph, so a negative spacing
  moves the ink into the neighbour. The half-width glyph forms of the font are the right tool.

## Fonts

Pango needs a font with Japanese or Chinese glyphs and, for good vertical punctuation, one that has the `vert`/`vrt2` and `halt`/`vhal`
features. On the development machine: Noto Serif CJK JP and Noto Sans CJK JP (`google-noto-serif-cjk-vf-fonts`,
`google-noto-sans-cjk-vf-fonts`), so the checks of this work were made with them. Fallback is whatever fontconfig finds for the language
(the book's language is given to the Pango context); with no CJK font at all, boxes are drawn and the layout is still correct.
The app does not bundle a font (that would be 15-20 MB per language plus an `OFL` entry in `NOTICE.md`). **Flatpak**: the host's fonts
are mounted read-only in `/run/host/fonts`, so a system CJK font is available inside the sandbox; some freedesktop runtime
releases ship no CJK font of their own (flathub/com.discordapp.Discord#556 for 25.08), so we do not rely on the runtime.
Not tried: a Flatpak build here.

## Not done (and why)

* **Hanging punctuation** (*burasage*, `、。` may hang outside the line end): Pango breaks lines itself; hanging needs our own
  breaker with forced breaks (ghost characters like Knuth-Plass uses). Lines that kinsoku pushes (*oidashi*) are shorter and are spread
  evenly instead. A pure greedy breaker with hanging would be a day's work; the effect is cosmetic.
* **`line-break` (strict/loose/anywhere), `word-break`, `text-emphasis` (emphasis dots, *bouten*), `ruby-position`, `ruby-align`,
  `text-underline-position`**: not parsed. Kinsoku is always the strict set; ruby is always centred on its base, over it or to its right.
* **Mixed writing modes in one chapter**: the mode of `html`/`body` rules the whole chapter; a horizontal block inside is laid out
  vertically. EPUB books use one mode per file in practice.
* **Floats, drop caps, wrapping around pictures, code backgrounds and quote bars in vertical text**: off (pictures stand in the flow).
* **Dictionary extensions of R56** (Japanese deinflection, MeCab segmentation): a double click selects the text between two punctuation
  marks because Japanese has no spaces; word-level selection needs a segmenter.
* **Mongolian** (`vertical-lr` with the Mongolian script, joining and its own glyph orientation): `vertical-lr` works with CJK text;
  Mongolian shaping was not tried (no test book with a licence worth the trouble).
* **Keyboard**: Shift+arrows follow the lines' direction, but Page Up/Down, Left/Right and Home/End keep their horizontal meaning (they
  turn pages by the package's direction).
* **Not tried on real hardware**: touch swipes in vertical books (the logic is the one tested for Arabic), e-ink refresh of vertical pages.

## Findings worth remembering

* `Escape` in the reader leaves the book; a real-session scenario that dismisses a popover with it must press it once.
* `Atspi` key events carry ASCII only: a scenario cannot type Japanese into the search entry; it searches for the Latin word in the book.
* A click exactly in the middle of a glyph hits the boundary after it (`trailing`), as for horizontal text; tests aim at the upper or lower
  quarter of a character when they need an exact offset.
* Pango's `justify` stretches CJK lines evenly, so a line that kinsoku left short looks airy; a paragraph made of nothing but marks that
  cannot start a line (`ー、ぁ、ゃ、っ、々、……` in the synthetic book) makes that visible on purpose.

## Test fixtures

Generated, never committed: `build-aux/make-vertical-epubs.py` (`synthetic`, `synthetic-long`, `aozora`), and the Aozora Bunko file
(see `docs/test-books.md`). Scenarios: `build-aux/ui-tests.sh` (`vertical_layout`, `vertical_chapters`, `vertical_scroll_and_styles`,
`vertical_read_aloud`, `vertical_budget`) and `build-aux/session/scenarios_vertical.py` (`vertical_reading`, `vertical_aozora`). Dev hooks: `reader:ui-vertical`,
`reader:ui-vsel:A:B`, `reader:ui-shot:FILE` (documented in `_developer/docs.md`).
