#!/usr/bin/env bash
# Builds the public wiki (wiki/, Zensical) into wiki/site and checks it.
#
#   build-aux/wiki.sh            build, make pictures lighter, check links, anchors, light/dark pictures and the
#                                colour-scheme toggle (headless Firefox, skipped when it is missing)
#   WIKI_SHOTS=DIR build-aux/wiki.sh   also saves screenshots of the built pages (light/dark, desktop/phone) in DIR
#   build-aux/wiki.sh serve      live preview at http://127.0.0.1:8000
#
# Zensical is installed into a user-local virtual environment (nothing system-wide):
#   VENV=~/.cache/bookx-wiki-venv   where it goes
#   ZENSICAL_VERSION=0.1.1          the pinned version (the GitHub workflow uses this script too)
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")/.."

ZENSICAL_VERSION="${ZENSICAL_VERSION:-0.1.1}"
VENV="${VENV:-$HOME/.cache/bookx-wiki-venv}"
CONFIG=wiki/zensical.toml
SITE=wiki/site

if [ ! -x "$VENV/bin/zensical" ] || [ "$("$VENV/bin/zensical" --version 2>/dev/null)" != "$ZENSICAL_VERSION" ]; then
    echo "==> Installing Zensical $ZENSICAL_VERSION into $VENV"
    if command -v uv >/dev/null 2>&1; then
        uv venv --quiet --allow-existing "$VENV"
        uv pip install --quiet --python "$VENV/bin/python" "zensical==$ZENSICAL_VERSION" pillow
    else
        python3 -m venv "$VENV"
        "$VENV/bin/python" -m pip install --quiet --disable-pip-version-check "zensical==$ZENSICAL_VERSION" pillow
    fi
fi
"$VENV/bin/python" -c "import PIL" 2>/dev/null || {
    if command -v uv >/dev/null 2>&1; then uv pip install --quiet --python "$VENV/bin/python" pillow
    else "$VENV/bin/python" -m pip install --quiet --disable-pip-version-check pillow; fi
}

if [ "${1:-}" = serve ]; then
    exec "$VENV/bin/zensical" serve -f "$CONFIG" --dev-addr "127.0.0.1:8000"
fi

build-aux/check-sync-endpoint.sh || exit 1
echo "==> Building the wiki"
rm -rf "$SITE"
"$VENV/bin/zensical" build -f "$CONFIG" --clean

for f in index.html CNAME; do
    [ -f "$SITE/$f" ] || { echo "missing $SITE/$f in the built site" >&2; exit 1; }
done
[ "$(tr -d '[:space:]' < "$SITE/CNAME")" = "bookx.adhadse.com" ] || { echo "CNAME is not bookx.adhadse.com" >&2; exit 1; }

echo "==> Optimising pictures (WebP, lazy loading)"
"$VENV/bin/python" build-aux/wiki-optimize.py "$SITE"

echo "==> Checking the pictures against build-aux/wiki-pictures.txt (every picture exists, is on its page in both styles, has a scene; no orphans; size budget)"
python3 build-aux/check-wiki-pictures.py || exit 1
python3 build-aux/wiki_video.py check wiki/docs/assets || exit 1   # the videos: size, length and width budget
"$VENV/bin/python" build-aux/check-wiki-frames.py || exit 1
"$VENV/bin/python" build-aux/check-wiki-centred.py || exit 1
python3 build-aux/check-wiki-contrast.py || exit 1

echo "==> Checking the screenshots for focus rings"
"$VENV/bin/python" build-aux/check-screenshots.py wiki/docs/assets || exit 1

echo "==> Checking links and light/dark pictures"
python3 build-aux/check-site-links.py "$SITE"

echo "==> Checking the social cards and their Open Graph tags"
python3 build-aux/check-social-cards.py "$SITE"

if [ "${WIKI_BROWSER:-1}" != 0 ]; then
    echo "==> Checking the colour-scheme toggle in a browser (WIKI_BROWSER=0 skips; needs firefox)"
    python3 build-aux/wiki-theme-check.py "$SITE" ${WIKI_SHOTS:+--shots "$WIKI_SHOTS"} || exit 1
    echo "==> Checking that the demo videos stay paused with prefers-reduced-motion: reduce"
    python3 build-aux/wiki-theme-check.py "$SITE" --reduced-motion || exit 1
fi
echo "==> Wiki OK: $SITE"
