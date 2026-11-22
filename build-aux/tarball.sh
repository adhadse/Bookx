#!/usr/bin/env bash
# Packs the already built release binaries into bookx-<version>-linux-<arch>.tar.gz plus a .sha256 file.
#
#   cargo build --release --locked -p bookx -p bookx-cli
#   build-aux/tarball.sh [OUTDIR]       # default OUTDIR=dist
#
# Deterministic packing: sorted names, owner 0:0, mtime = SOURCE_DATE_EPOCH (default: the date of the HEAD commit),
# gzip without a name or timestamp. The same binaries therefore give the same archive byte for byte. Whether two
# separate *builds* give the same binary is a different question (see docs/packaging-notes.md).
# TARGET_DIR (default target/release) says where the binaries are.
set -euo pipefail
cd "$(dirname "$0")/.."

out="${1:-dist}"
target="${TARGET_DIR:-target/release}"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=$(uname -m)
name="bookx-$version-linux-$arch"
epoch="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}"

[ -x "$target/bookx" ] || { echo "no $target/bookx: run cargo build --release first" >&2; exit 1; }
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
make -s install-flatpak DESTDIR="$stage/$name" PREFIX=/usr TARGET_DIR="$target"
cp README.md LICENSE NOTICE.md CHANGELOG.md "$stage/$name/"
mkdir -p "$out"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" -C "$stage" -cf - "$name" \
    | gzip -n -9 > "$out/$name.tar.gz"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256" && cat "$name.tar.gz.sha256")
