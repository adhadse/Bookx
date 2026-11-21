# Editor preview sync, splitting, compare, Live CSS (wave 4, internal)

Rows: CE-120/124, CE-220/223..229, CE-241..249, CE-260 (statuses in `calibre-editor-gap-analysis.md`).

## Design decisions
- **Source offsets are not in `Doc`.** The preview parser works on a `scraper` DOM that has no byte positions, and
  adding a field to `Block` would touch every constructor (another agent edits `doc.rs`/`html.rs` for vertical text).
  Instead `bookx_editing::preview_map` cuts the source into *segments* (text/media directly in one block element)
  with a forgiving scan and pairs them with `Doc.blocks` by text, in order (lookahead 24). Unpaired blocks have no
  position. `tag_blocks` puts an anchor `bxsN` first in each paired block; the canvas reads them back
  (`src/ui/canvas_src.rs`: `block_at_point`, `block_top`, `reveal_block`). Cost: one pass over source and blocks
  (perf budget `preview_map_and_side_by_side_on_a_large_chapter`, 1.5 MB).
- Weak spot: a block whose preview text differs from the source text (generated markers are fine, they live in
  `marker`; `<ruby>` annotation text, `display:none` text with styled colours, list items nested oddly) may stay
  unpaired; many consecutive misses (>24 segments) stop pairing for the rest of the file.
- Split: `SplitSpec::{AtOffsets, ByClass, Selector}`; `snap_offset` moves a position out of tags and out of element
  edges. `Selector` is a small XPath subset, not a full XPath engine.
- Live CSS reads rules as text (file, line kept) and matches with `bookx_text::css` selectors; values are the
  declared winners, not computed pixels; no UA sheet; `@media` evaluated at an 800 px screen.
- Compare is a page of the editor centre (`center` stack child `compare`), not a dialog. Colours are translucent tints
  so they work on light and dark; e-ink uses weight/strike-through.

## Dev hooks (`editor:` prefix)
`pv-state`, `pv-where`, `pv-follow=on|off`, `pv-auto=on|off`, `pv-refresh`, `pv-click-block=N` (real hit test),
`pv-click-text=T`, `pv-link=HREF`, `pv-pick-toc=I`, `pv-pick-split`, `pv-pick-cancel`, `pv-hover=N`, `pv-toc`,
`pv-split-cursor`, `pv-split=[preview:]KIND:ARG` (headings, class, pagebreaks, selector, offset),
`pv-dialog=MODE|LEVEL|TAG|CLASS|SELECTOR`, `pv-dialog-status`, `pv-dialog-go`, `pv-find=T`, `pv-copy`,
`cmp-dump|file=|beautify=|context=|search=|next-change|goto-text=|close|book=PATH`,
`cp-dialog|new=|list|rename=ID|NAME|pin=ID|delete=ID|compare=ID`, `css-show|dump|jump=SELECTOR`.
Scenarios: `build-aux/ui-tests-editor-preview.sh` (10), `build-aux/session/scenarios_editor_preview.py` (2).

## Not done / caveats
- Dragging in the Contents is still missing (CE-120).
- Click in the preview places the caret at the start of the block and (wave 11) selects the block's source (`pv.ends`, from `Segment::range.end`). Vertical-text markers: still horizontal text only.
- Marker line and block positions are for horizontal text only.
- CE-228/229 not possible: the catalogue stores no originals.
- `wiki/docs/editing.md` contains some sections twice (an earlier merge by another agent); the new sections are
  single.

## Scroll sync (enhancement 26)
- `bookx_editing::scroll_sync`: `ScrollMap` (anchors `(char offset, flow y)` of the paired blocks plus the two ends,
  monotonic, both directions a binary search) and `Gate` (a cursor move keeps the sync quiet for 300 ms).
- UI in `preview.rs`: code `vadjustment` -> `CodeScrolled` -> 40 ms timer (longer while the gate is closed) ->
  `sync_code_to_preview`; preview `on_position` -> `PreviewScrolled` -> `sync_preview_to_code` only with the option.
  `syncing` guards our own scrolls; `BookCanvas::set_scroll_top` does not fire position events. After each reload
  the map is dropped, the preview synced to the code top, then the cursor kept in view, then synced once more (the
  canvas may apply its own start position late). The anchor cache is keyed by flow height and width.
- Uses the last known map while the text is stale (auto reload off): offsets drift by what was typed since.
- Hooks: `pv-code-scroll=LINE`, `pv-preview-scroll=FRACTION`, `pv-both=on|off`, `pv-sync-state`, `pv-fill=N`,
  `pv-hide=on|off`. No animation: the preview jumps per step (about every 40 ms while scrolling).
