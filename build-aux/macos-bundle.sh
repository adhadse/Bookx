#!/usr/bin/env bash
# Builds Bookx.app and a .dmg from a release build, on macOS with Homebrew's GTK 4 stack.
#
#   cargo build --release -p bookx -p bookx-cli
#   build-aux/macos-bundle.sh                # -> dist/Bookx.app and dist/Bookx-<version>-<arch>.dmg
#
# Environment (all optional):
#   CODESIGN_IDENTITY   "Developer ID Application: Name (TEAMID)"; without it the app is signed ad hoc ("-"),
#                       which runs on the machine that built it and needs right-click > Open elsewhere
#   NOTARY_PROFILE      a `xcrun notarytool store-credentials` profile; with it the dmg is notarized and stapled
#   TARGET_DIR          where cargo put the binaries (default target/release)
#   SKIP_DMG=1          only build the .app
#
# STATUS: UNTESTED. It was written without a Mac; expect to fix paths on the first run (docs/macos-port.md).
# Needs: brew install gtk4 libadwaita gtksourceview5 adwaita-icon-theme librsvg dylibbundler create-dmg espeak-ng
set -euo pipefail
cd "$(dirname "$0")/.."

[ "$(uname -s)" = Darwin ] || { echo "macos-bundle.sh runs on macOS only" >&2; exit 1; }
for t in dylibbundler iconutil rsvg-convert codesign glib-compile-schemas; do
    command -v "$t" >/dev/null || { echo "missing tool: $t (brew install dylibbundler librsvg glib)" >&2; exit 1; }
done

BREW="$(brew --prefix)"
TARGET_DIR="${TARGET_DIR:-target/release}"
APP_ID=com.adhadse.Bookx
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"; [ "$ARCH" = arm64 ] || ARCH=x86_64
OUT=dist
APP="$OUT/Bookx.app"
C="$APP/Contents"

rm -rf "$APP"
mkdir -p "$C/MacOS" "$C/Frameworks" "$C/Resources/share/glib-2.0/schemas" "$C/Resources/share/icons" "$C/Resources/lib"

echo "==> binaries"
cp "$TARGET_DIR/bookx" "$C/MacOS/bookx-bin"
cp "$TARGET_DIR/bookx-cli" "$C/MacOS/bookx-cli"
# the launcher: GTK needs to be told where the bundled data lives
cat > "$C/MacOS/bookx" <<'LAUNCH'
#!/bin/bash
here="$(cd "$(dirname "$0")" && pwd)"
res="$here/../Resources"
export XDG_DATA_DIRS="$res/share"
export GSETTINGS_SCHEMA_DIR="$res/share/glib-2.0/schemas"
export GDK_PIXBUF_MODULE_FILE="$res/lib/gdk-pixbuf-2.0/loaders.cache"
export GTK_PATH="$res/lib/gtk-4.0"
export ESPEAK_DATA_PATH="$res/share/espeak-ng-data"
export PATH="$here:$PATH"
exec "$here/bookx-bin" "$@"
LAUNCH
chmod +x "$C/MacOS/bookx"

echo "==> shared libraries (dylibbundler rewrites the install names)"
dylibbundler -od -b -x "$C/MacOS/bookx-bin" -x "$C/MacOS/bookx-cli" \
    -d "$C/Frameworks" -p @executable_path/../Frameworks/ >/dev/null

echo "==> gdk-pixbuf loaders"
LOADERS="$(find "$BREW/lib/gdk-pixbuf-2.0" -name loaders -type d | head -1)"
mkdir -p "$C/Resources/lib/gdk-pixbuf-2.0/loaders"
cp "$LOADERS"/*.so "$C/Resources/lib/gdk-pixbuf-2.0/loaders/" 2>/dev/null || true
for so in "$C"/Resources/lib/gdk-pixbuf-2.0/loaders/*.so; do
    dylibbundler -od -b -x "$so" -d "$C/Frameworks" -p @executable_path/../Frameworks/ >/dev/null || true
done
GDK_PIXBUF_MODULEDIR="$C/Resources/lib/gdk-pixbuf-2.0/loaders" \
    gdk-pixbuf-query-loaders "$C"/Resources/lib/gdk-pixbuf-2.0/loaders/*.so \
    | sed "s#$C/Resources#@executable_path/../Resources#g" > "$C/Resources/lib/gdk-pixbuf-2.0/loaders.cache"

echo "==> GSettings schemas, icons, espeak-ng data"
# Homebrew links the schemas of every formula into one directory; GTK, libadwaita and GtkSourceView are in it
cp "$BREW"/share/glib-2.0/schemas/*.gschema.xml "$C/Resources/share/glib-2.0/schemas/"
glib-compile-schemas "$C/Resources/share/glib-2.0/schemas"
cp -R "$BREW/share/icons/Adwaita" "$C/Resources/share/icons/"
cp -R "$BREW/share/icons/hicolor" "$C/Resources/share/icons/" 2>/dev/null || true
mkdir -p "$C/Resources/share/icons/hicolor/scalable/apps"
cp data/icons/hicolor/scalable/apps/$APP_ID.svg "$C/Resources/share/icons/hicolor/scalable/apps/"
gtk4-update-icon-cache -f -t "$C/Resources/share/icons/Adwaita" 2>/dev/null || true
cp -R "$BREW/share/gtksourceview-5" "$C/Resources/share/"
cp -R "$BREW/share/espeak-ng-data" "$C/Resources/share/"
cp "$BREW/bin/espeak-ng" "$C/MacOS/espeak-ng"
dylibbundler -od -b -x "$C/MacOS/espeak-ng" -d "$C/Frameworks" -p @executable_path/../Frameworks/ >/dev/null

echo "==> icon"
ICONSET="$(mktemp -d)/Bookx.iconset"
mkdir -p "$ICONSET"
for s in 16 32 64 128 256 512; do
    rsvg-convert -w $s -h $s data/icons/hicolor/scalable/apps/$APP_ID.svg -o "$ICONSET/icon_${s}x${s}.png"
    rsvg-convert -w $((s * 2)) -h $((s * 2)) data/icons/hicolor/scalable/apps/$APP_ID.svg -o "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$C/Resources/Bookx.icns"

echo "==> Info.plist"
cat > "$C/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Bookx</string>
  <key>CFBundleDisplayName</key><string>Bookx</string>
  <key>CFBundleIdentifier</key><string>$APP_ID</string>
  <key>CFBundleExecutable</key><string>bookx</string>
  <key>CFBundleIconFile</key><string>Bookx</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.reference</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Anurag Dhadse and the Bookx contributors, GPL-3.0-or-later</string>
  <key>CFBundleURLTypes</key><array><dict>
    <key>CFBundleURLName</key><string>Bookx link</string>
    <key>CFBundleURLSchemes</key><array><string>bookx</string></array>
  </dict></array>
  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeName</key><string>E-book</string>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSHandlerRank</key><string>Alternate</string>
    <key>LSItemContentTypes</key><array>
      <string>org.idpf.epub-container</string><string>com.amazon.mobi</string>
      <string>com.adobe.pdf</string><string>public.rtf</string>
    </array>
  </dict></array>
</dict></plist>
PLIST

echo "==> signing"
ID="${CODESIGN_IDENTITY:--}"
OPTS=(--force --timestamp)
[ "$ID" = "-" ] || OPTS+=(--options runtime)
# inside out: libraries first, then the executables, then the bundle
find "$C/Frameworks" "$C/Resources/lib" -type f \( -name '*.dylib' -o -name '*.so' \) -exec codesign "${OPTS[@]}" -s "$ID" {} \;
for b in espeak-ng bookx-cli bookx-bin bookx; do codesign "${OPTS[@]}" -s "$ID" "$C/MacOS/$b"; done
codesign "${OPTS[@]}" -s "$ID" "$APP"
codesign --verify --deep --strict "$APP" && echo "signature verifies"

if [ "${SKIP_DMG:-0}" = 1 ]; then echo "built $APP"; exit 0; fi

echo "==> dmg"
DMG="$OUT/Bookx-$VERSION-$ARCH.dmg"
rm -f "$DMG"
create-dmg --volname "Bookx $VERSION" --window-size 560 340 --icon-size 96 \
    --icon Bookx.app 150 160 --app-drop-link 410 160 "$DMG" "$APP"
[ "$ID" = "-" ] || codesign --force --timestamp -s "$ID" "$DMG"
if [ -n "${NOTARY_PROFILE:-}" ] && [ "$ID" != "-" ]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
fi
(cd "$OUT" && shasum -a 256 "$(basename "$DMG")" | tee "$(basename "$DMG").sha256")
echo "built $DMG"
