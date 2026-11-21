#!/usr/bin/env bash
# Builds dist/bookx.koplugin.zip: the folder `bookx.koplugin` ready to unzip into KOReader's `plugins` folder.
# Runs the plugin tests first (unless SKIP_TESTS=1).
set -euo pipefail
cd "$(dirname "$0")/.."
[ "${SKIP_TESTS:-0}" = 1 ] || koreader-plugin/tests/run.sh
version=$(sed -n 's/^M.VERSION = "\(.*\)".*/\1/p' koreader-plugin/bookx.koplugin/bookxsync.lua)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -r koreader-plugin/bookx.koplugin "$stage/bookx.koplugin"
mkdir -p dist
out="$PWD/dist/bookx.koplugin.zip"
rm -f "$out"
if command -v zip >/dev/null 2>&1; then
  (cd "$stage" && zip -qr -X "$out" bookx.koplugin)
else
  (cd "$stage" && python3 -c "
import sys, zipfile, os
z = zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_DEFLATED)
for root, _, files in os.walk('bookx.koplugin'):
    for f in sorted(files):
        z.write(os.path.join(root, f))
z.close()" "$out")
fi
echo "dist/bookx.koplugin.zip (Bookx Sync $version): unzip it so that the folder bookx.koplugin sits inside KOReader's plugins folder"
unzip -l "$out" 2>/dev/null | tail -n +4 | head -12 || true
