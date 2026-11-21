#!/usr/bin/env bash
# Writes the directory that becomes the flathub/com.adhadse.Bookx repository (or a pull request
# updating it): manifest pinned to a release tag and commit, cargo-sources.json for that tag, flathub.json.
#
#   packaging/flathub/prepare.sh v0.3.0 [OUT_DIR]     OUT_DIR defaults to /tmp/opencode/flathub-bookx
#
# The tag must exist on https://github.com/adhadse/Bookx (it is cloned from there), so nothing here
# can run before the owner has pushed a tag. The script changes no file in this repository and
# publishes nothing. Needs git, python3 (3.11+), network.
set -euo pipefail
cd "$(dirname "$0")/../.."

tag="${1:?usage: prepare.sh vX.Y.Z [out-dir]}"
out="${2:-/tmp/opencode/flathub-bookx}"
repo="${BOOKX_REPO_URL:-https://github.com/adhadse/Bookx.git}"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag must look like v1.2.3" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" "$repo" "$work/src"
# an annotated tag points at a tag object; Flathub wants the commit
commit="$(git -C "$work/src" rev-parse "$tag^{commit}")"

mkdir -p "$out"
python3 "$work/src/build-aux/flatpak-cargo-sources.py" "$work/src/Cargo.lock" -o "$out/cargo-sources.json" >/dev/null
sed -e "s|^        tag: .*|        tag: $tag|" \
    -e "s|^        commit: .*|        commit: $commit|" \
    packaging/flathub/com.adhadse.Bookx.yml > "$out/com.adhadse.Bookx.yml"
cp packaging/flathub/flathub.json "$out/flathub.json"

# screenshots in the metainfo are fetched from the tag, not from a branch that moves
if grep -q "raw.githubusercontent.com/adhadse/Bookx/main/" "$work/src/data/com.adhadse.Bookx.metainfo.xml"; then
    echo "note: the metainfo at $tag still points its screenshots at the main branch." >&2
    echo "      Flathub mirrors them at build time, so this works, but they can change under a release." >&2
fi

echo "wrote $out:"
ls -l "$out"
echo
echo "next: cd $out && flatpak-builder --user --install --force-clean build com.adhadse.Bookx.yml"
echo "      flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest com.adhadse.Bookx.yml"
