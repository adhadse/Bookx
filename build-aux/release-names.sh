#!/usr/bin/env bash
# Names the release files the way each ecosystem names its own (ripgrep, bat, KOReader do the same):
#
#   bookx-<version>-linux-<arch>.tar.gz     tarball
#   bookx_<version>_<debarch>.deb           Debian/Ubuntu   (amd64, arm64)
#   bookx-<version>.<arch>.rpm              Fedora/openSUSE (x86_64, aarch64)
#   bookx-<version>-<arch>.flatpak          Flatpak bundle
#
#   build-aux/release-names.sh DIR VERSION [ARCH]     # ARCH default: x86_64
#
# <version> is the tag without the "v" (0.3.0-beta.1). The tools write their own names, and GitHub rewrites "~" in an
# asset name to ".", so the file names carry the plain version (the packages inside still say 0.3.0~beta.1 so that the
# final release sorts higher). Renames in DIR, removes the per-file .sha256 files (SHA256SUMS covers everything),
# prints "old -> new". Fails if one of the four kinds is missing or ambiguous, so a release never goes out
# half-renamed. docs/releasing.md has the scheme.
set -euo pipefail

dir="${1:?usage: release-names.sh DIR VERSION [ARCH]}"
version="${2:?usage: release-names.sh DIR VERSION [ARCH]}"
arch="${3:-x86_64}"
case "$arch" in
    x86_64) debarch=amd64 ;;
    aarch64) debarch=arm64 ;;
    *) debarch="$arch" ;;
esac
cd "$dir"

rename_one() { # new-name, glob...
    local new="$1"; shift
    local found=() f
    for f in "$@"; do [ -f "$f" ] && found+=("$f"); done
    if [ "${#found[@]}" -ne 1 ]; then
        echo "release-names: expected exactly one file for $new in $dir, found ${#found[@]}: ${found[*]:-none}" >&2
        exit 1
    fi
    if [ "${found[0]}" != "$new" ]; then mv -- "${found[0]}" "$new"; fi
    echo "${found[0]} -> $new"
}

rename_one "bookx-$version-linux-$arch.tar.gz" [Bb]ookx-*-linux-*.tar.gz
rename_one "bookx_${version}_$debarch.deb" *.deb
rename_one "bookx-$version.$arch.rpm" *.rpm
rename_one "bookx-$version-$arch.flatpak" *.flatpak
rm -f -- *.sha256
