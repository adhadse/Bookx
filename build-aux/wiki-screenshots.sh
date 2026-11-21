#!/usr/bin/env bash
# Real screenshots for the public wiki, taken in sandboxed GNOME Shell sessions (build-aux/session): the library
# (with the Continue Reading banner), details, notes, table, Device window, Preferences, palette, the reader (with
# highlights and the selection popover), read aloud, the book map, the editor, vertical text and the e-ink mode, with
# public-domain test books and a shelf with their real public-domain covers (build-aux/make-wiki-books.py, Standard
# Ebooks). Nothing touches your library or config.
#
# Every picture is made twice, in the Light and in the Dark style (the sandbox's GNOME color-scheme, so libadwaita draws
# the real style), then framed by build-aux/screenshot-frame.py (rounded corners, thin outline, soft drop shadow on a
# transparent background) and written as wiki/docs/assets/<name>-light.png and <name>-dark.png. The pages show the one
# that matches the site's palette (`![..](x-light.png#only-light)` and `![..](x-dark.png#only-dark)`).
#
#   build-aux/wiki-screenshots.sh                       # build, shoot everything in both styles, write the assets
#   SKIP_BUILD=1 build-aux/wiki-screenshots.sh          # use the binaries that are already built
#   SCHEMES=dark build-aux/wiki-screenshots.sh          # one style only
#   SCENARIOS='_wiki_library _wiki_map' build-aux/wiki-screenshots.sh   # a subset
#   KEEP=1 SCENARIOS='_wiki_ed_dialogs2' ...           # add scenes to the previous run instead of starting over
#   FRAME_ONLY=1 KEEP=1 ...                              # only the framing step again, on the captures of the last run
#   DRY=1 build-aux/wiki-screenshots.sh                 # shoot into $RESULTS only, leave wiki/docs/assets alone
#   JOBS=2                                              # sessions at a time (default 2; each uses ~15 inotify instances)
#
# The pictures at the top of README.md and in the AppStream metainfo are NOT separate copies: they point at these same
# files (README: <picture> with the -dark / -light pair; metainfo: raw.githubusercontent URLs), so a run of this script
# refreshes them too. build-aux/check-readme-images.py (run by release-check.sh) fails when README.md uses anything else.
#
# Details and usage steps (a menu, a popover, a dialog, one panel) are crops of the same kind of capture: a scene calls
# wshot(c, name, crop=NODE | [NODES] | (x0, y0, x1, y1), pad=..., scale=..., minsize=(w, h)) or dialog_shot / popover_shot
# (they find the popup by what it changes on the screen). Such a picture gets a `.detail` marker next to the raw capture and is
# framed with a small shadow (screenshot-frame.py --detail). Which picture belongs on which page: build-aux/wiki-pictures.txt.
#
# Needs what session-tests.sh needs (gnome-shell, AT-SPI, python3 with PyGObject and Pillow) and ~/.cache/bookx-testbooks.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
ROOT=$PWD
BASE=${RESULTS:-${CARGO_TARGET_DIR:-$ROOT/target}/wiki-screenshots}
SCHEMES=${SCHEMES:-"light dark"}
# whole windows (scenarios_wiki.py) and details / usage steps (scenarios_wiki_ext.py: pictures made with wshot(crop=...))
ALL_SCENARIOS="_wiki_shots _wiki_hero _wiki_reading _wiki_strip _wiki_library _wiki_favourites _wiki_table _wiki_reader_ux _wiki_map _wiki_tts _wiki_editor _wiki_vertical _wiki_eink
    _wiki_lib_steps _wiki_lib_dialogs _wiki_lib_banner _wiki_lib_banner_strip _wiki_lib_shelves _wiki_lib_jobs _wiki_lib_health _wiki_lib_empty
    _wiki_lib_details _wiki_lib_menus_misc _wiki_lib_device _wiki_prefs_pages _wiki_prefs_details
    _wiki_reader_steps _wiki_reader_ruler _wiki_reader_words _wiki_reader_cards _wiki_reader_theme_light _wiki_reader_theme_sepia _wiki_reader_theme_dark
    _wiki_reader_notes _wiki_reader_aloud _wiki_reader_vocab _wiki_reader_pdf _wiki_reader_comic _wiki_map_detail _wiki_vertical_detail
    _wiki_ed_code _wiki_ed_menus _wiki_ed_dialogs _wiki_ed_dialogs2 _wiki_ed_panels
    _wiki_lib_works _wiki_audiobook _wiki_vocab_own _wiki_reader_comic2"
ALL_SCENARIOS=$(echo $ALL_SCENARIOS)
SCENARIOS=${SCENARIOS:-$ALL_SCENARIOS}
# the pictures the pages use: build-aux/wiki-pictures.txt ("<page.md>: <name> ..."); a missing one fails the run
PICTURES=$(grep -v '^#' build-aux/wiki-pictures.txt | cut -d: -f2- | tr ' ' '\n' | grep -v '^$' | sort -u | tr '\n' ' ')
PY=/usr/bin/python3; [ -x "$PY" ] || PY=$(command -v python3)
# the comic pictures show a real public-domain comic ("Little Nemo in Slumberland", Winsor McCay, 1905; see docs/test-books.md);
# it is downloaded once into the test-book cache. Offline (NET=0) and without a cache the synthetic comic is used instead.
BOOKS=${BOOKX_TESTBOOKS:-${REAL_HOME:-$HOME}/.cache/bookx-testbooks}
mkdir -p "$BOOKS/comics"
if [ -z "${WIKI_REAL_COMIC:-}" ]; then
    if "$PY" build-aux/make-real-comic.py "$BOOKS/comics/little-nemo.cbz" "$BOOKS/real-comic" >/dev/null; then
        export WIKI_REAL_COMIC=$BOOKS/comics/little-nemo.cbz
        echo "==> comic pictures: the real public-domain comic $WIKI_REAL_COMIC"
    else
        echo "==> comic pictures: the real comic is not available (NET=0 or offline); using the synthetic comic. Do not commit these pictures."
    fi
fi
[ "${KEEP:-0}" = 1 ] || rm -rf "$BASE"   # KEEP=1 with SCENARIOS=... adds scenes to an earlier run
for scheme in $SCHEMES; do
    [ "${FRAME_ONLY:-0}" = 1 ] && continue   # FRAME_ONLY=1 KEEP=1: frame the captures of the last run again
    export WIKI_SCHEME=$scheme
    RESULTS=$BASE/$scheme
    mkdir -p "$RESULTS"
    todo=$SCENARIOS
    echo "==> $scheme style: $(echo $todo | wc -w) scenes"
    # other session runs on this machine can use up the inotify instances or file descriptors for a while (a scene then fails
    # without a result): only the scenes that failed are run again, up to three times
    for attempt in 1 2 3; do
        try=$BASE/$scheme.try
        rm -rf "$try"
        out=$(RESULTS=$try ONLY="$todo" JOBS=${JOBS:-2} build-aux/session-tests.sh 2>&1)
        echo "$out" | grep -v '^\.\|^---\|^Ran\|^OK$\|^$'
        cp -f "$try"/* "$RESULTS"/ 2>/dev/null
        todo=$(echo "$out" | sed -n 's/^FAIL  \([^ ]*\).*/\1/p' | tr '\n' ' ')
        [ -z "$todo" ] && break
        [ "$attempt" = 3 ] && { echo "these scenes failed in the $scheme style (see $RESULTS): $todo"; exit 1; }
        echo "trying again in 60 s: $todo (attempt $attempt of 3)"
        sleep 60
    done
    rm -rf "$BASE/$scheme.try"
    SKIP_BUILD=1
    export SKIP_BUILD
done
# a scenario that skips a step still "passes": make sure every picture the pages use was really taken
if [ "$SCENARIOS" = "$ALL_SCENARIOS" ] || [ "${KEEP:-0}" = 1 ]; then
    missing=0
    for scheme in $SCHEMES; do
        for name in $PICTURES; do
            ls "$BASE/$scheme"/_wiki_*-"$name".png >/dev/null 2>&1 || { echo "missing picture: $name ($scheme)"; missing=1; }
        done
    done
    [ "$missing" = 0 ] || { echo "some pictures were not taken (see the scenario logs in $BASE)"; exit 1; }
fi
# no focus ring and no cursor in any picture (build-aux/check-screenshots.py; the scenarios also write a user style sheet that
# turns focus rings off, and take the pictures with cursor=false)
"$PY" build-aux/check-screenshots.py "$BASE" || { echo "a screenshot shows a focus ring or a cursor (see above)"; exit 1; }
[ "${DRY:-0}" = 1 ] && { echo "screenshots are in $BASE"; exit 0; }
n=0
for scheme in $SCHEMES; do
    for f in "$BASE/$scheme"/_wiki_*-*.png; do
        [ -e "$f" ] || continue
        b=$(basename "$f" .png)   # _wiki_shots-library-grid
        name=${b#*-}
        out=$ROOT/wiki/docs/assets/$name-$scheme.png
        detail=""; [ -e "${f%.png}.detail" ] && detail="--detail"   # a crop of a widget, not a whole window
        "$PY" build-aux/screenshot-frame.py --scheme "$scheme" $detail "$f" "$out" || exit 1
        echo "wrote $out"
        n=$((n + 1))
    done
done
echo "$n pictures ($(du -sm "$ROOT/wiki/docs/assets" | cut -f1) MB in wiki/docs/assets)"
