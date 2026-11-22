#!/usr/bin/env bash
# Points the metainfo screenshots at a release tag instead of the main branch (Flathub wants "a tag or a
# commit and not a branch"). Run before tagging, commit the change, then create the tag.
#
#   build-aux/pin-screenshots.sh v0.3.0
set -euo pipefail
cd "$(dirname "$0")/.."
tag="${1:?usage: pin-screenshots.sh vX.Y.Z}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "tag must look like v1.2.3 or v1.2.3-beta.1" >&2; exit 1; }
f=data/com.adhadse.Bookx.metainfo.xml
sed -i -E "s#(raw\.githubusercontent\.com/adhadse/Bookx/)(main|master|v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?)/#\1$tag/#" "$f"
echo "screenshots in $f now use $tag:"
grep -c "Bookx/$tag/" "$f"
