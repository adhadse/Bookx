#!/usr/bin/env bash
# Gives every release file the same name:  Bookx-<version>-linux-<arch>.<ext>
#
#   build-aux/release-names.sh DIR VERSION [ARCH]     # ARCH default: x86_64
#
# Renames in DIR: the tarball (Bookx-*-linux-*.tar.gz, or the older lowercase bookx-*), the .deb, the .rpm and the Flatpak bundle (Bookx-*.flatpak),
# and removes the per-file .sha256 files (SHA256SUMS covers everything). Prints "old -> new". Fails if one of the four
# kinds is missing or ambiguous, so a release never goes out with a half-renamed set. docs/releasing.md has the scheme.
set -euo pipefail

dir="${1:?usage: release-names.sh DIR VERSION [ARCH]}"
version="${2:?usage: release-names.sh DIR VERSION [ARCH]}"
arch="${3:-x86_64}"
cd "$dir"

rename_one() { # kind, ext, glob...
    local ext="$1"; shift
    local found=()
    local f
    for f in "$@"; do [ -f "$f" ] && found+=("$f"); done
    if [ "${#found[@]}" -ne 1 ]; then
        echo "release-names: expected exactly one .$ext file in $dir, found ${#found[@]}: ${found[*]:-none}" >&2
        exit 1
    fi
    local new="Bookx-$version-linux-$arch.$ext"
    if [ "${found[0]}" != "$new" ]; then mv -- "${found[0]}" "$new"; fi
    echo "${found[0]} -> $new"
}

rename_one tar.gz bookx-*-linux-*.tar.gz Bookx-*-linux-*.tar.gz
rename_one deb *.deb
rename_one rpm *.rpm
rename_one flatpak *.flatpak
rm -f -- *.sha256
