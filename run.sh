#!/usr/bin/env bash
# Builds Bookx (showing cargo's progress), installs it for the current user and launches it.
#
#   ./run.sh            build in release mode, install to ~/.local, launch
#   ./run.sh --debug    faster build, no install, run straight from target/
#   ./run.sh --no-run   build and install only
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")"

PREFIX="${PREFIX:-$HOME/.local}"
APP_ID="com.adhadse.Bookx"
MODE=release
RUN=1
for a in "$@"; do
    case "$a" in
        --debug) MODE=debug ;;
        --no-run) RUN=0 ;;
        -h|--help) sed -n '2,7p' "$0"; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 1 ;;
    esac
done

step() { printf '\n\033[1;34m==>\033[0m \033[1m%s\033[0m\n' "$*"; }

step "Building Bookx ($MODE)"
if [ "$MODE" = release ]; then
    cargo build --release --color always
    BIN=target/release/bookx
else
    cargo build --color always
    BIN=target/debug/bookx
fi

if [ "$MODE" = release ]; then
    step "Installing to $PREFIX"
    install -Dm755 "$BIN" "$PREFIX/bin/bookx"
    install -Dm644 "data/$APP_ID.desktop" "$PREFIX/share/applications/$APP_ID.desktop"
    sed -i "s|^Exec=.*|Exec=$PREFIX/bin/bookx %U|" "$PREFIX/share/applications/$APP_ID.desktop"
    install -Dm644 "data/$APP_ID.metainfo.xml" "$PREFIX/share/metainfo/$APP_ID.metainfo.xml"
    install -Dm644 "data/icons/hicolor/scalable/apps/$APP_ID.svg" "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    install -Dm644 "data/icons/hicolor/scalable/apps/$APP_ID-symbolic.svg" "$PREFIX/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
    install -Dm644 "data/$APP_ID.mime.xml" "$PREFIX/share/mime/packages/$APP_ID.xml"
    update-mime-database "$PREFIX/share/mime" 2>/dev/null || true
    update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
    gtk4-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

if [ "$RUN" = 1 ]; then
    step "Launching Bookx (Ctrl+C to quit)"
    exec "$BIN"
fi
step "Done"
