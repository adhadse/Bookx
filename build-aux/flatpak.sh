#!/usr/bin/env bash
# Builds the Flatpak (offline: crates come from build-aux/cargo-sources.json) and smoke-tests it.
#
#   build-aux/flatpak.sh              static checks, build, smoke tests
#   BUNDLE=1 build-aux/flatpak.sh     also write bookx.flatpak next to the repository
#   CHECK_ONLY=1 build-aux/flatpak.sh only the static checks (manifest, desktop file, metainfo, sources)
#
#   REVIEWED_LOCALES=1 build-aux/flatpak.sh   build like Flathub: English plus reviewed languages only
#
# Needs flatpak-builder plus the GNOME 51 Platform and SDK and the rust-stable extension (26.08)
# installed; nothing is installed for you. Without flatpak-builder only the static checks run and
# the script says so. Build files live in $FLATPAK_BUILD_DIR (default ~/.cache/bookx-flatpak).
set -euo pipefail
cd "$(dirname "$0")/.."

MANIFEST=build-aux/com.adhadse.Bookx.json
APP_ID=com.adhadse.Bookx
WORK="${FLATPAK_BUILD_DIR:-$HOME/.cache/bookx-flatpak}"
fail=0
step() { printf '\n==> %s\n' "$*"; }

step "Cargo sources match Cargo.lock"
python3 build-aux/flatpak-cargo-sources.py >/dev/null
python3 build-aux/flatpak-cargo-sources.py --check

step "Manifest is valid JSON with the pieces a build needs"
python3 - "$MANIFEST" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
assert m["id"] == "com.adhadse.Bookx", "id"
assert m["runtime"] == "org.gnome.Platform" and m["sdk"] == "org.gnome.Sdk"
assert any("rust-stable" in e for e in m["sdk-extensions"]), "rust extension"
mods = {x["name"]: x for x in m["modules"]}
assert "espeak-ng" in mods and "bookx" in mods, "modules"
assert "--share=network" in m["finish-args"], "network permission"
assert not any("--share=network" in a for a in m.get("build-options", {}).get("build-args", [])), "build must be offline"
assert "cargo-sources.json" in mods["bookx"]["sources"], "vendored sources"
for a in m["finish-args"]:
    assert a not in ("--filesystem=host", "--filesystem=home", "--device=all"), "too broad: " + a
print("manifest ok:", m["runtime"], m["runtime-version"])
PY

step "Desktop file, metainfo and MIME definitions"
desktop-file-validate data/$APP_ID.desktop || fail=1
if command -v appstreamcli >/dev/null; then appstreamcli validate --no-net data/$APP_ID.metainfo.xml || fail=1; fi
xmllint --noout data/$APP_ID.mime.xml || fail=1
[ "$fail" = 0 ] || { echo "static checks failed" >&2; exit 1; }

if [ "${CHECK_ONLY:-0}" = 1 ]; then echo "checks passed (CHECK_ONLY)"; exit 0; fi
if ! command -v flatpak-builder >/dev/null; then
    echo "flatpak-builder is not installed: only the static checks ran, nothing was built." >&2
    exit 0
fi

# REVIEWED_LOCALES=1: build the way the Flathub manifest does, without the machine-translated languages
# (cargo feature reviewed-locales-only, po/reviewed-locales.txt). The manifest stays untouched; a copy is used.
if [ "${REVIEWED_LOCALES:-0}" = 1 ]; then
    REVIEWED_MANIFEST=build-aux/.reviewed-locales.com.adhadse.Bookx.json
    trap 'rm -f "$REVIEWED_MANIFEST"' EXIT
    sed -e 's|-p bookx -p bookx-cli"|-p bookx -p bookx-cli --features bookx/reviewed-locales-only"|' \
        -e 's|TARGET_DIR=target/release"|TARGET_DIR=target/release CARGO_FEATURES=bookx/reviewed-locales-only"|' \
        "$MANIFEST" > "$REVIEWED_MANIFEST"
    grep -q reviewed-locales-only "$REVIEWED_MANIFEST" || { echo "could not patch the manifest" >&2; exit 1; }
    MANIFEST=$REVIEWED_MANIFEST
fi

step "Building (this compiles the whole workspace in release mode)"
mkdir -p "$WORK"
flatpak-builder --force-clean --disable-rofiles-fuse \
    --state-dir="$WORK/state" --repo="$WORK/repo" \
    "$WORK/build" "$MANIFEST"

run() { flatpak-builder --run "$WORK/build" "$MANIFEST" "$@"; }

step "Smoke tests inside the sandbox"
run bookx-cli --help >/dev/null && echo "bookx-cli runs"
run espeak-ng --ipa -q -v en "hello" | grep -q . && echo "espeak-ng speaks"
run test -f /app/share/applications/$APP_ID.desktop && echo "desktop file installed"
run test -f /app/share/metainfo/$APP_ID.metainfo.xml && echo "metainfo installed"
run test -f /app/share/mime/packages/$APP_ID.xml && echo "MIME definitions installed"
run test -f /app/share/icons/hicolor/scalable/apps/$APP_ID.svg && echo "icon installed"
if command -v gtk4-broadwayd >/dev/null || run sh -c 'command -v gtk4-broadwayd' >/dev/null 2>&1; then
    # A scratch library, settings, cache and D-Bus session: the smoke test must never open the real library
    # or talk to a Bookx the user is running.
    run sh -c 'S=$(mktemp -d); export XDG_CONFIG_HOME=$S/config XDG_DATA_HOME=$S/data XDG_CACHE_HOME=$S/cache \
        BOOKX_LIBRARY=$S/library BOOKX_SECRETS=memory; D=$((300 + $(date +%N | sed s/^0*//) % 600)); rm -f $XDG_RUNTIME_DIR/broadway$((D + 1)).socket; \
        gtk4-broadwayd :$D >/dev/null 2>&1 & B=$!; trap "kill $B 2>/dev/null; rm -f $XDG_RUNTIME_DIR/broadway$((D + 1)).socket" EXIT; sleep 1; \
        GDK_BACKEND=broadway BROADWAY_DISPLAY=:$D BOOKX_DEV_SCRIPT=wait:1500,diag:x,quit \
        dbus-run-session -- timeout 60 bookx 2>&1 \
        | grep "diag:" | grep -q "(0 books)"' \
        && echo "the app starts in a scratch library and reports diagnostics" \
        || { echo "the app did not start inside the sandbox" >&2; exit 1; }
else
    echo "no Broadway in the runtime: the window was not started"
fi

if [ "${BUNDLE:-0}" = 1 ]; then
    step "Bundle"
    flatpak build-bundle "$WORK/repo" bookx.flatpak $APP_ID
    sha256sum bookx.flatpak > bookx.flatpak.sha256
    ls -l bookx.flatpak
fi
step "Flatpak build finished"
