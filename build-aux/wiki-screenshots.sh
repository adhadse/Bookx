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
#   DRY=1 build-aux/wiki-screenshots.sh                 # shoot into $RESULTS only, leave wiki/docs/assets alone
#   JOBS=2                                              # sessions at a time (default 2; each uses ~15 inotify instances)
#
# Needs what session-tests.sh needs (gnome-shell, AT-SPI, python3 with PyGObject and Pillow) and ~/.cache/bookx-testbooks.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
ROOT=$PWD
BASE=${RESULTS:-${CARGO_TARGET_DIR:-$ROOT/target}/wiki-screenshots}
SCHEMES=${SCHEMES:-"light dark"}
ALL_SCENARIOS="_wiki_shots _wiki_hero _wiki_reading _wiki_strip _wiki_library _wiki_table _wiki_reader_ux _wiki_map _wiki_tts _wiki_editor _wiki_vertical _wiki_eink"
SCENARIOS=${SCENARIOS:-$ALL_SCENARIOS}
PICTURES="library-device device-window preferences preferences-search command-palette reader reader-contents editor editor-find editor-palette
    library-banner-hero library-banner-strip library-details library-notes library-table reader-sidebar-contents reader-sidebar-notes reader-handles
    reader-wikipedia reader-footer-settings book-map read-aloud-popover read-aloud-reading read-aloud-settings editor-preview editor-split-menu
    reader-vertical reader-eink"
PY=/usr/bin/python3; [ -x "$PY" ] || PY=$(command -v python3)
rm -rf "$BASE"
for scheme in $SCHEMES; do
    export RESULTS=$BASE/$scheme WIKI_SCHEME=$scheme
    mkdir -p "$RESULTS"
    echo "==> $scheme style: $SCENARIOS"
    # other session runs on this machine can use up the inotify instances for a while: try again a few times
    for attempt in 1 2 3; do
        ONLY="$SCENARIOS" JOBS=${JOBS:-2} build-aux/session-tests.sh && break
        [ "$attempt" = 3 ] && { echo "the $scheme screenshot session failed (see $RESULTS)"; exit 1; }
        echo "the $scheme session failed, trying again in 90 s (attempt $attempt of 3)"
        sleep 90
        rm -rf "$RESULTS"; mkdir -p "$RESULTS"
    done
    SKIP_BUILD=1
    export SKIP_BUILD
done
# a scenario that skips a step still "passes": make sure every picture the pages use was really taken
if [ "$SCENARIOS" = "$ALL_SCENARIOS" ]; then
    missing=0
    for scheme in $SCHEMES; do
        for name in $PICTURES; do
            ls "$BASE/$scheme"/_wiki_*-"$name".png >/dev/null 2>&1 || { echo "missing picture: $name ($scheme)"; missing=1; }
        done
    done
    [ "$missing" = 0 ] || { echo "some pictures were not taken (see the scenario logs in $BASE)"; exit 1; }
fi
[ "${DRY:-0}" = 1 ] && { echo "screenshots are in $BASE"; exit 0; }
n=0
for scheme in $SCHEMES; do
    for f in "$BASE/$scheme"/_wiki_*-*.png; do
        [ -e "$f" ] || continue
        b=$(basename "$f" .png)   # _wiki_shots-library-grid
        name=${b#*-}
        out=$ROOT/wiki/docs/assets/$name-$scheme.png
        "$PY" build-aux/screenshot-frame.py --scheme "$scheme" "$f" "$out" || exit 1
        echo "wrote $out"
        n=$((n + 1))
    done
done
echo "$n pictures"
