#!/usr/bin/env bash
# Real-session UI tests: Bookx inside a sandboxed GNOME Shell (Mutter, Wayland), driven with REAL
# pointer and keyboard events and inspected through AT-SPI and screenshots.
#
#   build-aux/session-tests.sh                 # build (debug) and run all scenarios concurrently
#   NET=0 build-aux/session-tests.sh           # skip scenarios that need internet (and the book download)
#   JOBS=2 build-aux/session-tests.sh          # sessions at once (capped by the free inotify instances) (default: a quarter of the CPU cores; 0 = all)
#   ONLY='lookup_dictionary lookup_wikipedia' build-aux/session-tests.sh
#   SKIP_BUILD=1 build-aux/session-tests.sh    # use the binaries that are already built
#   KEEP=1 build-aux/session-tests.sh          # keep the work folders (logs, screenshots)
#   RESULTS=/some/dir build-aux/session-tests.sh   # where screenshots and reports go (default: target/session-results)
#
# Why this exists next to build-aux/ui-tests.sh (Broadway): Broadway has no real compositor. Popovers
# are drawn inside the window, there is no focus handling, no xdg_popup grab, no real pointer
# events, no clipboard or portals. This suite starts gnome-shell --headless --wayland --virtual-monitor,
# so the app is a real Wayland client of a real Mutter. See docs/test-notes.md.
#
# Each scenario = its own `dbus-run-session`, XDG_RUNTIME_DIR, config, cache, HOME, gsettings (keyfile),
# gnome-shell and library. Nothing touches your session, your library or your config.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
ROOT=$PWD
HERE=$ROOT/build-aux/session
PY=/usr/bin/python3
[ -x "$PY" ] || PY=$(command -v python3)

for t in gnome-shell dbus-run-session; do
    command -v "$t" >/dev/null || { echo "$t not found — skipping real-session tests"; exit 0; }
done
"$PY" -c "import gi, PIL; gi.require_version('Atspi','2.0'); from gi.repository import Atspi" 2>/dev/null || {
    echo "python3 with PyGObject, Atspi and Pillow not found — skipping real-session tests"; exit 0; }
[ -x /usr/libexec/at-spi2-registryd ] || { echo "at-spi2-registryd not found — skipping real-session tests"; exit 0; }

"$PY" "$HERE/selftest.py" || { echo "session harness self-test failed"; exit 1; }

[ "${SKIP_BUILD:-0}" = 1 ] || cargo build --quiet -p bookx -p bookx-cli || exit 1
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
export BOOKX_BIN=$TARGET/debug/bookx BOOKX_CLI=$TARGET/debug/bookx-cli

# ---- test books (public domain, downloaded, never committed; see docs/test-books.md) ----------
BOOKS=${BOOKX_TESTBOOKS:-${REAL_HOME:-$HOME}/.cache/bookx-testbooks}
mkdir -p "$BOOKS"
fetch() { # name url
    [ -s "$BOOKS/$1" ] && return 0
    [ "${NET:-1}" = 0 ] && return 1
    curl -fsSL -m 90 -o "$BOOKS/$1.part" "$2" && mv "$BOOKS/$1.part" "$BOOKS/$1"
}
fetch alice.epub "https://www.gutenberg.org/ebooks/11.epub3.images" || true
fetch tiny.epub "https://www.gutenberg.org/ebooks/1064.epub.noimages" || true
if [ ! -s "$BOOKS/alice.epub" ] || [ ! -s "$BOOKS/tiny.epub" ]; then
    echo "test books not cached in $BOOKS and not downloadable (NET=0?) — skipping real-session tests"
    exit 0
fi
export BOOK_ALICE=$BOOKS/alice.epub BOOK_TINY=$BOOKS/tiny.epub
# vertical writing: a synthetic Japanese book (generated) and, when the Aozora Bunko text can be had, a real one
# (夏目漱石「吾輩は猫である」, public domain; the scenarios skip themselves without it)
"$PY" "$ROOT/build-aux/make-vertical-epubs.py" synthetic "$BOOKS/vertical-synthetic.epub" && export BOOK_VSYNTH=$BOOKS/vertical-synthetic.epub
fetch aozora-neko-789_14547.html "https://www.aozora.gr.jp/cards/000148/files/789_14547.html" || true
if [ -s "$BOOKS/aozora-neko-789_14547.html" ]; then
    "$PY" "$ROOT/build-aux/make-vertical-epubs.py" aozora "$BOOKS/aozora-neko-789_14547.html" "$BOOKS/vertical-neko.epub" && export BOOK_VNEKO=$BOOKS/vertical-neko.epub
fi

# a generated PDF (8 pages: outline, page labels, links, a rare word on page 5); nothing is downloaded
if BIG=0 cargo run --quiet -p bookx-pdf --example make_test_pdfs "$BOOKS/pdf" >/dev/null 2>&1 && [ -s "$BOOKS/pdf/story.pdf" ]; then
    export BOOK_PDF=$BOOKS/pdf/story.pdf
fi

# generated comics (a story with panels, a manga, a webtoon); nothing is downloaded
if cargo run --quiet -p bookx-comic --example make_test_comics "$BOOKS/comics" >/dev/null 2>&1 && [ -s "$BOOKS/comics/story.cbz" ]; then
    export BOOK_COMIC=$BOOKS/comics/story.cbz BOOK_MANGA=$BOOKS/comics/manga.cbz
fi
# the wiki screenshots show a real public-domain comic (build-aux/make-real-comic.py, made by wiki-screenshots.sh);
# every other run keeps the synthetic one, so the tests stay offline and deterministic
if [ -n "${WIKI_REAL_COMIC:-}" ]; then
    if [ -s "$WIKI_REAL_COMIC" ]; then export BOOK_COMIC=$WIKI_REAL_COMIC
    else echo "WIKI_REAL_COMIC=$WIKI_REAL_COMIC does not exist; the synthetic comic is used"; fi
fi

# GNOME Shell + the app + helper daemons are heavy: default to a quarter of the cores (at least 1)
JOBS=${JOBS:-$(( $(nproc 2>/dev/null || echo 4) / 4 ))}
[ "$JOBS" = 0 ] && JOBS=100000
# inotify instances are limited per USER (often 128) and shared with your real desktop; every session
# needs about 15 (GNOME Shell, ibus, portals, the bus). Never take more than the free ones.
if [ -z "${JOBS_FORCE:-}" ] && [ -r /proc/sys/fs/inotify/max_user_instances ]; then
    used=$(find /proc/[0-9]*/fd -lname 'anon_inode:inotify' 2>/dev/null | wc -l)
    free=$(( $(cat /proc/sys/fs/inotify/max_user_instances) - used - 20 ))
    cap=$(( free / 16 ))
    [ "$cap" -lt 1 ] && cap=1
    [ "$JOBS" -gt "$cap" ] && { echo "(inotify instances free: $((free + 20)); running $cap sessions at once, set JOBS_FORCE=1 to override)"; JOBS=$cap; }
fi
[ "$JOBS" -lt 1 ] && JOBS=1
RESULTS=${RESULTS:-$TARGET/session-results}
rm -rf "$RESULTS"; mkdir -p "$RESULTS"
export SESSION_RESULTS=$RESULTS
TMPROOT=$(mktemp -d /tmp/bookx-session.XXXXXX)
[ "${KEEP:-0}" = 1 ] || trap 'rm -rf "$TMPROOT"' EXIT

ALL=$("$PY" - <<PY
import sys
sys.path.insert(0, "$HERE")
import scenarios, harness
print(" ".join(harness.SCENARIOS))
PY
)
LIST=${ONLY:-$(echo $ALL | tr " " "\n" | grep -v "^_" | tr "\n" " ")}

run_one() { # name: run, and once more when the app was killed from outside (exit 75)
    run_once "$1"
    local rc=$?
    # 75: the app was killed from outside; 76: the AT-SPI registry did not start. Both are infrastructure, not the app.
    if [ "$rc" = 75 ] || [ "$rc" = 76 ]; then rm -rf "$TMPROOT/$1"; run_once "$1"; rc=$?; fi
    return $rc
}

run_once() { # name
    local name=$1 d=$TMPROOT/$1
    rm -rf "$d"
    mkdir -p "$d"/{home,cfg,data,cache,state,run}
    chmod 700 "$d/run"
    (
        export SESSION_TEST_DIR=$d HOME=$d/home XDG_CONFIG_HOME=$d/cfg XDG_DATA_HOME=$d/data \
            XDG_CACHE_HOME=$d/cache XDG_STATE_HOME=$d/state XDG_RUNTIME_DIR=$d/run \
            GSETTINGS_BACKEND=keyfile BOOKX_SECRETS=memory BOOKX_LIBRARY=$d/lib
        unset WAYLAND_DISPLAY DISPLAY GDK_BACKEND BROADWAY_DISPLAY SSH_AUTH_SOCK
        # A private session bus that can only auto-start what GNOME Shell and the app need: the stock
        # bus would also start Tracker, Evolution, GOA, gvfs volume monitors ... per session, which use
        # inotify instances (limit 128 per USER, shared with your real desktop) and a lot of CPU.
        mkdir -p "$d/dbus/services"
        for svc in org.a11y.Bus org.freedesktop.portal.Desktop org.freedesktop.portal.Documents \
            org.freedesktop.impl.portal.desktop.gnome org.freedesktop.impl.portal.desktop.gtk \
            org.freedesktop.impl.portal.PermissionStore org.gnome.Shell.Notifications; do
            ln -sf "/usr/share/dbus-1/services/$svc.service" "$d/dbus/services/$svc.service"
        done
        cat >"$d/dbus/session.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:tmpdir=$d/run</listen>
  <auth>EXTERNAL</auth>
  <servicedir>$d/dbus/services</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
  <limit name="max_incoming_bytes">1000000000</limit>
  <limit name="max_outgoing_bytes">1000000000</limit>
</busconfig>
CONF
        timeout "${SCENARIO_TIMEOUT:-240}" dbus-run-session --config-file="$d/dbus/session.conf" -- "$PY" "$HERE/run_scenario.py" "$name"
    ) >"$RESULTS/$name.log" 2>&1
    local rc=$?
    # keep the compositor and app logs next to the screenshots
    cp "$d/shell.log" "$RESULTS/$name.shell.log" 2>/dev/null
    cp "$d/app.log" "$RESULTS/$name.app.log" 2>/dev/null
    return $rc
}

START=$(date +%s)
pids=(); names=()
for n in $LIST; do
    while [ "$(jobs -rp | wc -l)" -ge "$JOBS" ]; do sleep 0.5; done
    run_one "$n" &
    pids+=($!); names+=("$n")
done
fail=0
for i in "${!pids[@]}"; do
    wait "${pids[$i]}" || true
done
for n in "${names[@]}"; do
    f=$RESULTS/$n.txt
    if [ ! -s "$f" ]; then echo "FAIL  $n (no result; see $RESULTS/$n.log)"; fail=$((fail+1)); continue; fi
    if grep -q '^FAIL' "$f"; then
        echo "FAIL  $n"; grep '^FAIL' "$f" | sed 's/^/        /'; fail=$((fail+1))
    elif grep -q '^SKIP' "$f"; then echo "SKIP  $n ($(grep '^SKIP' "$f" | head -1 | cut -d: -f2-))"
    else echo "ok    $n ($(grep -c '^ok' "$f") checks)"; fi
done
echo "(ran up to $JOBS sessions at once in $(( $(date +%s) - START )) s; screenshots and logs: $RESULTS)"
[ "$fail" = 0 ]
