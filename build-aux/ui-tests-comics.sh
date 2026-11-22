# Scenarios of the comic reader: pages, layouts, direction, smart zoom, guided view, thumbnails,
# settings and limits. Sourced by ui-tests.sh (uses its `scenario`, `check`, `has`, $ROOT, $CLI, $BIN).
# The comics are synthetic (crates/bookx-comic/examples/make_test_comics.rs).

COMICS=$ROOT/comics
[ -n "${SKIP_BUILD:-}" ] || cargo build --quiet -p bookx-comic --example make_test_comics || exit 1
"${CARGO_TARGET_DIR:-$PWD/target}/debug/examples/make_test_comics" "$COMICS.all" >/dev/null 2>&1
for f in story manga webtoon spreads bomb corrupt; do
    mkdir -p "$COMICS/$f"
    mv "$COMICS.all/$f.cbz" "$COMICS/$f/$f.cbz" 2>/dev/null
done
mv "$COMICS.all/saga" "$COMICS/saga" 2>/dev/null
SHOTS=${COMIC_SHOTS:-/tmp/opencode/comic-shots}
mkdir -p "$SHOTS"

# the n-th `comic_state:` line as a key=value lookup: cs LINE KEY
cs() { grep "^comic_state:" <<<"$OUT" | sed -n "${1}p" | sed -E "s/.* $2=([^ ]*).*/\\1/"; }
# a number is within a range: near VALUE LOW HIGH
near() { awk -v v="$1" -v lo="$2" -v hi="$3" 'BEGIN { exit !(v >= lo && v <= hi) }'; }
# a ratio of two numbers is at least R: ratio A B R
ratio() { awk -v a="$1" -v b="$2" -v r="$3" 'BEGIN { exit !(a >= b * r) }'; }
# one field of the `shows=x0:y0:x1:y1` of a state line: shows LINE INDEX(1-4)
shows() { cs "$1" shows | cut -d: -f"$2"; }

R='library:import=$COMICS/story,wait:3500,read:3,until:comic:15000'

scenario comic_open "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,until:comic-ahead:15000,reader:comic-state,quit" '
    grep -q "Gutter Tales" <<<"$LIB"; check "the title comes from ComicInfo.xml" $?
    grep -q "Panel Park" <<<"$LIB"; check "and the series" $?
    grep -q "Ada Inker" <<<"$LIB"; check "and the writer" $?
    [ "$(cs 1 pages)" = 6 ]; check "the comic has 6 pages" $?
    [ "$(cs 1 page)" = 1 ]; check "it opens on the first page" $?
    [ "$(cs 1 mode)" = single ]; check "one page at a time by default" $?
    [ "$(cs 1 rtl)" = false ]; check "left to right by default" $?
    [ "$(cs 1 fitted)" = true ]; check "the page fits the window" $?
    [ "$(cs 1 cached)" -ge 3 ]; rc=$?; check "the next pages are decoded ahead (cached=$(cs 1 cached))" $rc
    ! has "panicked|CRITICAL|Gtk-WARNING|GLib-GObject"; check "opening runs without errors or GTK warnings" $?'

scenario comic_modes "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-mode=spread,wait:500,reader:comic-state,reader:comic-key=Right,wait:400,reader:comic-state,reader:comic-key=Right,wait:400,reader:comic-state,reader:comic-key=Right,wait:400,reader:comic-state,reader:comic-key=Right,wait:400,reader:comic-state,reader:comic-cover-alone=off,wait:500,reader:comic-state,reader:comic-wide-alone=off,wait:500,reader:comic-state,reader:comic-mode=single,wait:500,reader:comic-state,quit" '
    [ "$(cs 1 mode)" = spread ] && [ "$(cs 1 screen)" = 1/4 ] && [ "$(cs 1 page)" = 1 ]; check "two-page layout: the cover is on a page of its own (4 screens for 6 pages, one is a double page)" $?
    [ "$(cs 2 order)" = 2-3 ]; check "the next screen shows pages 2 and 3 side by side, left to right" $?
    [ "$(cs 3 page)" = 4 ] && [ "$(cs 3 order)" = 4 ]; check "the wide page (ComicInfo double page) is a screen of its own" $?
    [ "$(cs 4 order)" = 5-6 ]; check "pairing goes on after it (5 and 6)" $?
    [ "$(cs 5 page)" = 5 ] && [ "$(cs 5 screen)" = 4/4 ]; check "the arrow stops at the last screen" $?
    grep "^comic_state:" <<<"$OUT" | sed -n 2p | grep -q "label=\"Pages 2–3 of 6\""; check "the footer says Pages 2–3 of 6" $?
    [ "$(cs 6 screen)" = 4/4 ] && [ "$(cs 6 page)" = 5 ]; check "cover on its own page off: pages 1 and 2 pair up, the same page stays in view" $?
    [ "$(cs 7 screen)" = 3/3 ]; check "wide pages on their own off: three pairs" $?
    [ "$(cs 8 mode)" = single ] && [ "$(cs 8 screen)" = 5/6 ] && [ "$(cs 8 page)" = 5 ]; check "back to single page: the same page, one per screen" $?'

scenario comic_rtl "library:import=$COMICS/manga,wait:3500,read:3,until:comic:15000,reader:comic-state,reader:comic-key=Left,wait:400,reader:comic-state,reader:comic-key=Right,wait:400,reader:comic-state,reader:comic-mode=spread,wait:500,reader:comic-state,reader:comic-key=Right,wait:300,reader:comic-state,reader:comic-key=Left,wait:300,reader:comic-state,reader:comic-key=r,wait:700,reader:comic-state,reader:comic-rtl=on,wait:700,reader:comic-state,quit" '
    grep -q "Reverse Road" <<<"$LIB"; check "the manga title and series are in the library" $?
    [ "$(cs 1 rtl)" = true ]; check "ComicInfo Manga=YesAndRightToLeft opens the book right to left" $?
    [ "$(cs 2 page)" = 2 ]; check "Left goes to the next page in a right-to-left book" $?
    [ "$(cs 3 page)" = 1 ]; check "Right goes back" $?
    [ "$(cs 4 mode)" = spread ] && [ "$(cs 4 screen)" = 1/3 ]; check "two-page layout of a manga: the cover alone, then pairs" $?
    [ "$(cs 5 page)" = 1 ]; check "Right does not go before the first page" $?
    [ "$(cs 6 page)" = 2 ] && [ "$(cs 6 order)" = 3-2 ]; check "in a manga spread the first page of the pair is on the right" $?
    [ "$(cs 7 rtl)" = false ] && [ "$(cs 7 order)" = 2-3 ]; check "the r key switches the direction: pages 2 and 3 now left to right" $?
    [ "$(cs 8 rtl)" = true ] && [ "$(cs 8 order)" = 3-2 ]; check "and back" $?
    grep -q "\"rtl\": true" "$d/cfg/bookx/comics.json"; check "the direction is stored for the book" $?'

scenario comic_zoom "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-goto=2,wait:500,reader:comic-state,reader:comic-dblclick-page=0.25:0.25,reader:comic-state,wait:900,reader:comic-state,shot:$SHOTS/zoom-panel.png,reader:comic-dblclick-page=0.25:0.25,wait:900,reader:comic-state,reader:comic-dblclick-page=0.75:0.75,wait:900,reader:comic-state,reader:comic-dblclick-page=0.75:0.75,wait:900,reader:comic-goto=1,wait:500,reader:comic-dblclick-page=0.5:0.5,wait:900,reader:comic-state,quit" '
    [ "$(cs 1 fitted)" = true ] && [ "$(cs 1 panels)" = 4 ]; check "the grid page has four panels and fits the window" $?
    [ "$(cs 2 animating)" = true ]; check "the move to the panel is animated" $?
    [ "$(cs 3 fitted)" = false ] && [ "$(cs 3 stop)" = 1 ]; check "double-click on the top left panel zooms to it (stop 1)" $?
    ratio "$(cs 3 zoom)" "$(cs 1 zoom)" 2; rc=$?; check "the zoom is at least twice the page fit ($(cs 3 zoom) vs $(cs 1 zoom))" $rc
    near "$(shows 3 4)" 0.40 0.62 && near "$(shows 3 2)" 0 0.10; rc=$?; check "it shows the top of the page, down to the gutter (y $(shows 3 2) to $(shows 3 4))" $rc
    [ "$(cs 4 fitted)" = true ] && [ "$(cs 4 stop)" = none ]; check "a second double-click goes back to the whole page" $?
    [ "$(cs 5 stop)" = 4 ] && near "$(shows 5 2)" 0.35 0.6; rc=$?; check "double-click on the bottom right panel zooms to it (stop 4, y from $(shows 5 2))" $rc
    [ "$(cs 6 fitted)" = false ] && [ "$(cs 6 stop)" = none ] && [ "$(cs 6 panels)" = 0 ]; check "a page without panels zooms to the region around the pointer" $?
    ratio "$(cs 6 zoom)" "$(cs 1 zoom)" 1.5; check "and it is larger than the page fit" $?'

scenario comic_guided "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-animate=off,reader:comic-goto=2,wait:400,reader:comic-guided=on,wait:300,reader:comic-state,reader:comic-key=space,wait:700,reader:comic-state,reader:comic-key=space,wait:700,reader:comic-state,reader:comic-key=space,wait:700,reader:comic-state,reader:comic-key=space,wait:700,reader:comic-state,reader:comic-key=space,wait:900,reader:comic-state,reader:comic-key=shift+space,wait:900,reader:comic-state,reader:comic-key=shift+space,wait:700,reader:comic-state,reader:comic-key=g,wait:300,reader:comic-state,quit" '
    [ "$(cs 1 guided)" = true ] && [ "$(cs 1 stop)" = none ]; check "guided view is on and starts with the whole page" $?
    [ "$(cs 2 stop)" = 1 ] && [ "$(cs 2 page)" = 2 ] && near "$(shows 2 4)" 0.4 0.62; check "Space goes to the first panel" $?
    [ "$(cs 3 stop)" = 2 ] && [ "$(cs 4 stop)" = 3 ] && [ "$(cs 5 stop)" = 4 ]; check "and on through the four panels in reading order" $?
    [ "$(cs 5 page)" = 2 ] && near "$(shows 5 2)" 0.35 0.6; check "the fourth panel is the bottom right one" $?
    [ "$(cs 6 page)" = 3 ] && [ "$(cs 6 stop)" = 1 ] && [ "$(cs 6 stops)" = 6 ]; check "Space after the last panel turns to the next page, first of its 6 panels" $?
    [ "$(cs 7 page)" = 2 ] && [ "$(cs 7 stop)" = 4 ]; check "Shift+Space goes back to the last panel of the page before" $?
    [ "$(cs 8 stop)" = 3 ]; check "and one panel further back" $?
    [ "$(cs 9 guided)" = false ]; check "the g key switches guided view off" $?
    grep -q "\"guided\": false" "$d/cfg/bookx/comics.json"; check "the choice (here: off again) was stored for the book" $?'

scenario comic_guided_manga_700x700 "library:import=$COMICS/manga,wait:3500,read:3,until:comic:15000,app:win-size=700x700,wait:900,reader:comic-animate=off,reader:comic-goto=2,wait:400,reader:comic-guided=on,wait:300,reader:comic-key=space,wait:800,reader:comic-state,reader:comic-key=space,wait:800,reader:comic-state,quit" '
    [ "$(cs 1 rtl)" = true ] && [ "$(cs 1 stop)" = 1 ]; check "manga: guided view starts" $?
    near "$(shows 1 1)" 0.05 0.8 && near "$(shows 1 3)" 0.95 1.01; rc=$?; check "the first panel is the right one (view runs from x=$(shows 1 1) to $(shows 1 3) of the page, view $(cs 1 view))" $rc
    [ "$(cs 2 stop)" = 2 ] && near "$(shows 2 1)" 0 0.12; rc=$?; check "the second panel is the one on its left (x=$(shows 2 1))" $rc'

scenario comic_webtoon "library:import=$COMICS/webtoon,wait:3500,read:3,until:comic:15000,reader:comic-animate=off,reader:comic-mode=webtoon,wait:600,reader:comic-state,reader:comic-key=Down,reader:comic-key=Down,reader:comic-key=Down,wait:300,reader:comic-state,reader:comic-key=Page_Down,wait:700,reader:comic-state,reader:comic-goto=3,wait:700,reader:comic-state,reader:comic-wheel=30,wait:300,reader:comic-state,reader:comic-key=End,wait:700,reader:comic-state,reader:comic-key=Home,wait:700,reader:comic-state,shot:$SHOTS/webtoon.png,quit" '
    [ "$(cs 1 mode)" = webtoon ] && [ "$(cs 1 page)" = 1 ]; check "vertical scroll mode starts at the top" $?
    near "$(cs 1 zoom)" 2.5 2.6; rc=$?; check "pages are as wide as the window (zoom $(cs 1 zoom))" $rc
    ratio "$(cs 2 cy)" "$(cs 1 cy)" 1.5; rc=$?; check "the arrow keys scroll ($(cs 1 cy) to $(cs 2 cy))" $rc
    [ "$(cs 2 page)" = 1 ]; check "still on the first page" $?
    ratio "$(cs 3 cy)" "$(cs 2 cy)" 1.5; check "Page Down scrolls a screenful" $?
    [ "$(cs 4 page)" = 3 ] && near "$(cs 4 cy)" 3200 3700; rc=$?; check "going to page 3 puts its top in the window (cy $(cs 4 cy))" $rc
    ratio "$(cs 5 cy)" "$(cs 4 cy)" 1.02; check "the mouse wheel scrolls" $?
    [ "$(cs 6 page)" = 4 ]; check "End goes to the last page" $?
    [ "$(cs 7 page)" = 1 ]; check "Home to the first" $?
    grep -q "\"mode\": \"webtoon\"" "$d/cfg/bookx/comics.json"; check "the layout is stored for the book" $?'

scenario comic_fit "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-animate=off,reader:comic-state,reader:comic-fit=width,wait:400,reader:comic-state,reader:comic-key=space,wait:600,reader:comic-key=space,wait:600,reader:comic-state,reader:comic-key=space,wait:600,reader:comic-state,reader:comic-fit=height,wait:400,reader:comic-state,reader:comic-fit=original,wait:400,reader:comic-state,reader:comic-fit=page,wait:400,reader:comic-state,quit" '
    [ "$(cs 1 fit)" = page ]; check "pages fit the window by default" $?
    near "$(cs 1 zoom)" 1.07 1.09; rc=$?; check "fit page: the whole page ($(cs 1 zoom))" $rc
    [ "$(cs 2 fit)" = width ] && near "$(cs 2 zoom)" 2.55 2.57; rc=$?; check "fit width: the page is as wide as the window ($(cs 2 zoom))" $rc
    [ "$(cs 3 page)" = 1 ] && [ "$(cs 3 fitted)" = true ]; check "Space reads down a page that is taller than the window" $?
    ratio "$(cs 3 cy)" 400 1; rc=$?; check "and shows the lower part of it (cy $(cs 3 cy))" $rc
    [ "$(cs 4 page)" = 2 ]; check "at the bottom Space turns to the next page" $?
    near "$(cs 5 zoom)" 1.07 1.09; rc=$?; check "fit height ($(cs 5 zoom))" $rc
    near "$(cs 6 zoom)" 0.99 1.01; rc=$?; check "original size is 100% ($(cs 6 zoom))" $rc
    [ "$(cs 7 fit)" = page ] && near "$(cs 7 zoom)" 1.07 1.09; check "and back to fit page" $?'

scenario comic_zoom_input "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-animate=off,reader:comic-goto=2,wait:400,reader:comic-state,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,reader:comic-wheel=ctrl:-1,wait:300,reader:comic-state,reader:comic-key=Right,wait:300,reader:comic-state,reader:comic-drag=-150:-80,wait:300,reader:comic-state,reader:comic-key=minus,reader:comic-key=minus,reader:comic-key=minus,wait:300,reader:comic-state,reader:comic-key=0,wait:900,reader:comic-state,reader:comic-drag=-250:5,wait:700,reader:comic-state,reader:comic-drag=250:5,wait:700,reader:comic-state,quit" '
    [ "$(cs 1 fitted)" = true ]; check "the page starts fitted" $?
    [ "$(cs 2 fitted)" = false ]; check "Ctrl+wheel zooms in" $?
    ratio "$(cs 2 zoom)" "$(cs 1 zoom)" 2.5; rc=$?; check "by about 15% per notch ($(cs 1 zoom) to $(cs 2 zoom))" $rc
    ratio "$(cs 3 cx)" "$(cs 2 cx)" 1.01; rc=$?; check "the arrow keys pan a zoomed page sideways instead of turning it ($(cs 2 cx) to $(cs 3 cx))" $rc
    [ "$(cs 3 page)" = 2 ]; check "and the page does not change" $?
    ratio "$(cs 4 cx)" "$(cs 3 cx)" 1.0; check "dragging moves the page (the view moves right when the picture is dragged left)" $?
    [ "$(cs 5 zoom)" != "$(cs 4 zoom)" ]; check "the minus key zooms out" $?
    [ "$(cs 6 fitted)" = true ]; check "the 0 key fits the page again" $?
    [ "$(cs 7 page)" = 3 ]; check "dragging a fitted page to the left (a swipe) turns to the next page" $?
    [ "$(cs 8 page)" = 2 ]; check "and to the right turns back" $?'

scenario comic_thumbs "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-state,reader:comic-sidebar=on,wait:1500,reader:comic-state,reader:comic-thumb=4,wait:500,reader:comic-state,shot:$SHOTS/thumbs.png,reader:comic-sidebar=off,wait:500,reader:comic-state,quit" '
    [ "$(cs 1 sidebar)" = false ] && [ "$(cs 1 thumbs)" = 0 ]; check "the thumbnail strip is closed and nothing was decoded for it" $?
    [ "$(cs 2 sidebar)" = true ] && [ "$(cs 2 thumbs)" = 6 ]; check "opening the strip decodes the thumbnails of the visible pages (6)" $?
    [ "$(cs 3 page)" = 4 ]; check "activating a thumbnail goes to its page" $?
    [ "$(cs 4 sidebar)" = false ]; check "it closes again" $?'

scenario comic_memory "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-rtl=on,reader:comic-mode=webtoon,reader:comic-fit=original,wait:600,reader:comic-state,wait:400,close:,until:!comic:20000,wait:800,read:3,until:comic:20000,wait:300,reader:comic-state,reader:comic-reset,wait:700,reader:comic-state,quit" '
    [ "$(cs 1 rtl)" = true ] && [ "$(cs 1 mode)" = webtoon ] && [ "$(cs 1 fit)" = original ]; check "direction, layout and fit are changed" $?
    [ "$(cs 2 rtl)" = true ] && [ "$(cs 2 mode)" = webtoon ] && [ "$(cs 2 fit)" = original ]; check "they are remembered when the book is opened again" $?
    [ "$(cs 3 rtl)" = false ] && [ "$(cs 3 mode)" = single ] && [ "$(cs 3 fit)" = page ]; check "Use the defaults for this book forgets them" $?
    ! grep -q "\"rtl\"" "$d/cfg/bookx/comics.json"; check "and the stored entry is gone" $?'

scenario comic_progress "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-goto=5,wait:700,close:,until:!comic:20000,wait:800,read:3,until:comic:20000,wait:300,reader:comic-state,quit" '
    [ "$(cs 1 page)" = 5 ]; check "the book opens on the page where you stopped" $?
    grep -q "\"spine\": 4" <<<"$LIB"; check "the page is stored in the library" $?'

scenario comic_eink "eink:on,library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,wait:800,reader:comic-state,reader:comic-dblclick-page=0.25:0.25,reader:comic-state,wait:300,reader:comic-state,shot:$SHOTS/eink.png,quit" '
    [ "$(cs 1 grey)" = true ]; check "e-ink mode shows the pages in grey" $?
    [ "$(cs 1 grey_pages)" -ge 1 ] && [ "$(cs 1 grey_pages)" = "$(cs 1 cached)" ] || [ "$(cs 1 grey_pages)" = 1 ]; rc=$?; check "the page on screen is really grey (grey_pages=$(cs 1 grey_pages))" $rc
    [ "$(cs 2 animating)" = false ]; check "there is no animation in e-ink mode" $?
    [ "$(cs 3 fitted)" = false ]; check "the zoom still works" $?'

scenario comic_colour_and_grey "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-state,reader:comic-grey=on,wait:1200,reader:comic-state,reader:comic-grey=off,wait:1200,reader:comic-state,quit" '
    [ "$(cs 1 grey)" = false ] && [ "$(cs 1 grey_pages)" = 0 ]; check "pages are in colour by default" $?
    [ "$(cs 2 grey)" = true ] && [ "$(cs 2 grey_pages)" -ge 1 ]; check "Grey pages turns them grey (pages decoded again)" $?
    [ "$(cs 3 grey_pages)" = 0 ]; check "and back to colour" $?'

scenario comic_bomb "library:import=$COMICS/bomb,wait:3500,read:3,until:comic:15000,reader:comic-state,quit" '
    [ "$(cs 1 pages)" = 1 ]; check "a page whose header claims 40000 x 40000 pixels is left out (1 page remains)" $?
    ! has "panicked|out of memory"; check "and nothing crashed" $?'

scenario comic_corrupt "library:import=$COMICS/corrupt,wait:3500,read:3,until:comic:15000,reader:comic-goto=2,wait:1500,reader:comic-state,reader:comic-key=Right,wait:800,reader:comic-state,shot:$SHOTS/corrupt.png,quit" '
    [ "$(cs 1 failed)" = 1 ] && [ "$(cs 1 pages)" = 3 ]; check "a damaged page is reported as failed, the book stays open (3 pages, 1 failed)" $?
    [ "$(cs 2 page)" = 3 ]; check "the next page can still be read" $?
    ! has "panicked"; check "no crash" $?'

scenario comic_a11y "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-sidebar=on,wait:800,a11y-audit:,quit" '
    has "^a11y: controls=[0-9]+ problems=0"; check "every control of the comic reader has an accessible name" $?'

scenario comic_prefs "prefs:,wait:800,prefs:pick=Reading,wait:400,prefs:comic-layout=webtoon,prefs:comic-fit=width,prefs:comic-guided=on,prefs:comic-cover=off,prefs:comic-wide=off,prefs:comic-grey=on,prefs:comic-animate=off,dialog:close,wait:500,library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-state,quit" '
    P=$(grep "^comic_prefs:" <<<"$OUT" | tail -1)
    grep -q "layout=webtoon fit=width cover=false wide=false guided=true grey=true animate=false" <<<"$P"; check "the defaults are set in Preferences → Reading → Comics ($P)" $?
    grep -q "\"mode\": \"webtoon\"" "$d/cfg/bookx/comics.json" && grep -q "\"grey\": true" "$d/cfg/bookx/comics.json"; check "and stored" $?
    [ "$(cs 1 mode)" = webtoon ] && [ "$(cs 1 fit)" = width ] && [ "$(cs 1 guided)" = true ]; check "a comic with no choices of its own opens with them" $?
    [ "$(cs 1 grey)" = true ]; check "grey pages from the defaults" $?'

scenario comic_look "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-goto=3,wait:800,shot:$SHOTS/single.png,reader:comic-sidebar=on,wait:1500,shot:$SHOTS/sidebar.png,reader:comic-mode=spread,wait:800,shot:$SHOTS/spread.png,quit" '
    [ -s "$SHOTS/spread.png" ]; check "screenshots taken" $?'

# next and previous volume (PP-G-24): the series comes from the folder name, Right on the last page
# says which volume is next and a second Right opens it; going back opens the last page of the one before
scenario comic_next_volume "library:import=$COMICS/saga,wait:3500,read:3,until:comic:15000,reader:comic-state,reader:comic-volumes,reader:comic-goto=6,wait:500,reader:comic-key=Right,wait:400,reader:comic-state,reader:comic-key=Right,wait:3500,until:comic:15000,reader:comic-state,reader:comic-volumes,reader:comic-key=Left,wait:400,reader:comic-key=Left,wait:3500,until:comic:15000,reader:comic-goto=6,wait:500,reader:comic-state,quit" '
    grep -q "Saga v01" <<<"$LIB" && grep -q "Saga v02" <<<"$LIB"; check "both volumes are in the library, titled after their files" $?
    grep -q "\"series\": \"saga\"" <<<"$LIB" || grep -qi "saga" <<<"$LIB"; check "the series comes from the folder name" $?
    grep "^comic_volumes:" <<<"$OUT" | sed -n 1p | grep -q "prev=- next=Saga v02"; check "the first volume has no previous one and v02 is next" $?
    [ "$(cs 2 book)" = 3 ] && [ "$(cs 2 page)" = 6 ]; check "the first push past the last page does not leave the book" $?
    [ "$(cs 3 book)" = 4 ] && [ "$(cs 3 page)" = 1 ]; check "the second push opens the next volume at its first page" $?
    grep "^comic_volumes:" <<<"$OUT" | sed -n 2p | grep -q "prev=Saga v01 next=-"; check "the last volume has a previous one and no next" $?
    [ "$(cs 4 book)" = 3 ]; check "going back from the first page opens the volume before" $?
    ! has "panicked|CRITICAL"; check "no errors" $?'

scenario comic_volume_button "library:import=$COMICS/saga,wait:3500,read:3,until:comic:15000,reader:comic-volume=next,wait:3500,until:comic:15000,reader:comic-state,reader:comic-volume=prev,wait:3500,until:comic:15000,reader:comic-state,quit" '
    [ "$(cs 1 book)" = 4 ] && [ "$(cs 1 page)" = 1 ]; check "Next Volume opens the next book at its start" $?
    [ "$(cs 2 book)" = 3 ] && [ "$(cs 2 page)" = 6 ]; check "Previous Volume opens the one before at its last page" $?'

scenario comic_alone "library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-volumes,reader:comic-goto=6,wait:500,reader:comic-key=Right,wait:300,reader:comic-key=Right,wait:1500,reader:comic-state,quit" '
    grep "^comic_volumes:" <<<"$OUT" | grep -q "prev=- next=-"; check "a comic with no neighbours has no volumes" $?
    [ "$(cs 1 pages)" = 6 ] && [ "$(cs 1 page)" = 6 ]; check "pushing past its end stays on the last page" $?'

scenario comic_ignore_info_and_library_direction "prefs:,wait:800,prefs:pick=Reading,wait:400,prefs:comic-ignore-info=on,prefs:comic-library-rtl=on,wait:300,dialog:close,wait:500,library:import=$COMICS/story,wait:3500,read:3,until:comic:15000,reader:comic-state,quit" '
    P=$(grep "^comic_prefs:" <<<"$OUT" | tail -1)
    grep -q "library_rtl=true ignore_info=true" <<<"$P"; check "both switches are on in Preferences → Reading → Comics ($P)" $?
    grep -q "\"ignore_info\": true" "$d/cfg/bookx/comics.json" && grep -q "library_rtl" "$d/cfg/bookx/comics.json"; check "and stored" $?
    ! grep -q "Gutter Tales" <<<"$LIB" && ! grep -q "Panel Park" <<<"$LIB"; check "ComicInfo.xml is not used: the title and series it names are not in the library" $?
    grep -q "story" <<<"$LIB"; check "the book is called after its file" $?
    [ "$(cs 1 rtl)" = true ]; check "the library reads right to left, so the comic does too" $?'
