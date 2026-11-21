#!/usr/bin/env bash
# Collects Bookx and everything it loads into a folder and a zip, from an MSYS2 UCRT64 shell.
#
#   cargo build --release -p bookx -p bookx-cli
#   build-aux/windows-bundle.sh              # -> dist/Bookx/ and dist/Bookx-<version>-windows-x64.zip
#
# Layout (GLib finds `share` and `lib` relative to the executables, so keep it):
#   Bookx/bin/bookx.exe, bookx-cli.exe, espeak-ng.exe, *.dll
#   Bookx/share/{glib-2.0/schemas,icons,gtksourceview-5,espeak-ng-data,...}
#   Bookx/lib/gdk-pixbuf-2.0/...
#
# STATUS: UNTESTED. It was written without a Windows machine; expect to fix DLL lists on the first run
# (docs/windows-port.md). No code signing: SmartScreen will warn on first start of an unsigned exe.
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(uname -s)" in MINGW*|MSYS*|CLANG*|UCRT*) ;; *) echo "windows-bundle.sh runs in an MSYS2 shell only" >&2; exit 1 ;; esac
[ -n "${MINGW_PREFIX:-}" ] || { echo "start an MSYS2 UCRT64 shell (MINGW_PREFIX is empty)" >&2; exit 1; }

TARGET_DIR="${TARGET_DIR:-target/release}"
APP_ID=com.adhadse.Bookx
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
OUT=dist
DIR="$OUT/Bookx"

rm -rf "$DIR"
mkdir -p "$DIR/bin" "$DIR/share/glib-2.0/schemas" "$DIR/share/icons" "$DIR/lib"

echo "==> programs"
cp "$TARGET_DIR/bookx.exe" "$TARGET_DIR/bookx-cli.exe" "$DIR/bin/"
cp "$MINGW_PREFIX/bin/espeak-ng.exe" "$DIR/bin/"

echo "==> DLLs (everything the programs need from $MINGW_PREFIX)"
copy_dlls() {
    ldd "$1" | awk '/=>/ { print $3 }' | grep -i "^$MINGW_PREFIX" | while read -r dll; do
        [ -e "$DIR/bin/$(basename "$dll")" ] || cp "$dll" "$DIR/bin/"
    done
}
# loop until no new DLL appears (dependencies of dependencies)
for _ in 1 2 3 4 5 6; do
    for f in "$DIR"/bin/*.exe "$DIR"/bin/*.dll; do copy_dlls "$f"; done
done

echo "==> gdk-pixbuf loaders, schemas, icons, GtkSourceView and espeak-ng data"
cp -R "$MINGW_PREFIX/lib/gdk-pixbuf-2.0" "$DIR/lib/"
# the loader cache must name the loaders relative to the install
GDK_PIXBUF_MODULEDIR="$DIR/lib/gdk-pixbuf-2.0/2.10.0/loaders" gdk-pixbuf-query-loaders \
    | sed "s#$(cygpath -m "$DIR")/#@@#g;s#@@#../#g" > "$DIR/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" || true
cp "$MINGW_PREFIX"/share/glib-2.0/schemas/*.gschema.xml "$DIR/share/glib-2.0/schemas/"
glib-compile-schemas "$DIR/share/glib-2.0/schemas"
cp -R "$MINGW_PREFIX/share/icons/Adwaita" "$MINGW_PREFIX/share/icons/hicolor" "$DIR/share/icons/"
mkdir -p "$DIR/share/icons/hicolor/scalable/apps"
cp data/icons/hicolor/scalable/apps/$APP_ID.svg "$DIR/share/icons/hicolor/scalable/apps/"
cp -R "$MINGW_PREFIX/share/gtksourceview-5" "$DIR/share/"
cp -R "$MINGW_PREFIX/share/espeak-ng-data" "$DIR/share/"
cp LICENSE NOTICE.md README.md "$DIR/"

echo "==> zip"
ZIP="$OUT/Bookx-$VERSION-windows-x64.zip"
rm -f "$ZIP"
(cd "$OUT" && powershell -NoProfile -Command "Compress-Archive -Path Bookx -DestinationPath '$(basename "$ZIP")'" 2>/dev/null \
    || zip -qr "$(basename "$ZIP")" Bookx)
(cd "$OUT" && sha256sum "$(basename "$ZIP")" | tee "$(basename "$ZIP").sha256")
du -sh "$DIR"
echo "built $ZIP"
