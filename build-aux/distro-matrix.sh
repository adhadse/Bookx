#!/usr/bin/env bash
# Installs the release packages on real distributions (containers) and starts the app, so a package that cannot run on a
# supported distribution (glibc, GTK or libadwaita too old, a missing dependency) is found BEFORE a release.
#
#   build-aux/distro-matrix.sh DIR                        # DIR holds the .deb, .rpm and linux .tar.gz
#   ONLY="ubuntu:25.10 fedora:43" build-aux/distro-matrix.sh DIR   # a subset
#   JOBS=4 build-aux/distro-matrix.sh DIR                 # containers at a time (default 4)
#
# Needs podman or docker. Per image: install the package the distribution uses (.deb with apt, .rpm with dnf/zypper,
# the tarball on Arch), run `bookx-cli --version`, start `bookx` on a Broadway display for a few seconds and require the
# line "library opened". Prints one line per image: PASS or FAIL with the reason, and a table at the end.
# Exit status: 1 when an image in SUPPORTED fails; images in BEST_EFFORT may fail (they are reported, not fatal).
# docs/releasing.md ("Supported distributions") says what is promised.
set -uo pipefail

dir="${1:?usage: distro-matrix.sh DIR}"
cd "$(dirname "$0")/.."
dir=$(cd "$dir" && pwd)
engine=$(command -v podman || command -v docker || true)
[ -n "$engine" ] || { echo "distro-matrix: needs podman or docker" >&2; exit 2; }
jobs="${JOBS:-4}"

# image  kind  (kind: deb | rpm-dnf | rpm-zypper | tar-arch)
SUPPORTED=${SUPPORTED:-"
fedora:43 rpm-dnf
fedora:44 rpm-dnf
fedora:latest rpm-dnf
ubuntu:25.10 deb
ubuntu:26.04 deb
debian:testing deb
archlinux:latest tar-arch
"}
# Not promised: openSUSE Tumbleweed moves daily. Never listed: distributions older than GTK 4.20 / libadwaita 1.8
# (Debian 12 and 13, Ubuntu 24.04, Fedora 42): the .deb/.rpm cannot install there, use the Flatpak.
BEST_EFFORT=${BEST_EFFORT:-"
opensuse/tumbleweed:latest rpm-zypper
"}

# the files are found by type, so the script works on the tools' own names (before release-names.sh) and on the release names
deb=$(ls "$dir"/*.deb 2>/dev/null | head -1)
rpm=$(ls "$dir"/*.rpm 2>/dev/null | head -1)
tar=$(ls "$dir"/*linux*.tar.gz 2>/dev/null | head -1)
[ -n "$deb" ] && [ -n "$rpm" ] && [ -n "$tar" ] || { echo "distro-matrix: need a .deb, an .rpm and a linux .tar.gz in $dir" >&2; exit 2; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# the script that runs inside the container; $KIND and $FILE come from the environment
cat > "$work/inner.sh" <<'INNER'
export DEBIAN_FRONTEND=noninteractive
step() { echo "STEP $(date +%T) $*"; }
fail() { echo "RESULT FAIL $*"; exit 0; }
step install
case "$KIND" in
  deb)
    apt-get update -qq >/dev/null 2>&1 || fail "apt-get update"
    out=$(apt-get install -y -qq "/rel/$FILE" 2>&1) || fail "install: $(echo "$out" | grep -E 'Depends|Unable|E:' | head -2 | tr '\n' ' ' | cut -c1-200)"
    apt-get install -y -qq dbus-daemon procps libgtk-4-bin >/dev/null 2>&1 ;;
  rpm-dnf)
    out=$(dnf install -y -q "/rel/$FILE" 2>&1) || fail "install: $(echo "$out" | grep -iE 'nothing provides|requires|conflict|error' | head -2 | tr '\n' ' ' | cut -c1-200)"
    dnf install -y -q dbus-daemon procps-ng >/dev/null 2>&1 ;;
  rpm-zypper)
    out=$(zypper --non-interactive --no-gpg-checks install -y "/rel/$FILE" 2>&1) || fail "install: $(echo "$out" | grep -iE 'nothing provides|requires|conflict|error' | head -2 | tr '\n' ' ' | cut -c1-200)"
    zypper --non-interactive install -y dbus-1-daemon procps gtk4-tools >/dev/null 2>&1 ;;
  tar-arch)
    pacman -Syu --noconfirm gtk4 libadwaita gtksourceview5 espeak-ng dbus procps-ng >/dev/null 2>&1 || fail "pacman dependencies"
    tar -xzf "/rel/$FILE" -C /opt && cp -r /opt/bookx-*/usr/* /usr/ ;;
esac
step installed
command -v bookx >/dev/null || fail "bookx is not on the PATH after install"
v=$(bookx-cli --version 2>&1) || fail "bookx-cli: $(echo "$v" | head -1 | cut -c1-160)"
missing=$(ldd "$(command -v bookx)" 2>&1 | grep -E 'not found|version .* not found' | head -2 | tr '\n' ' ')
[ -z "$missing" ] || fail "libraries: $missing"
export XDG_RUNTIME_DIR=/tmp/xdg HOME=/tmp/home GDK_BACKEND=broadway BROADWAY_DISPLAY=:5
mkdir -p "$XDG_RUNTIME_DIR" "$HOME"; chmod 700 "$XDG_RUNTIME_DIR"
command -v gtk4-broadwayd >/dev/null || fail "gtk4-broadwayd missing in the image (test setup)"
step start
(gtk4-broadwayd :5 >/dev/null 2>&1 &); sleep 2
(timeout 30 dbus-run-session -- bookx >/tmp/bookx.log 2>&1 &)
for i in $(seq 1 25); do grep -q 'library opened' /tmp/bookx.log 2>/dev/null && break; sleep 1; done
grep -q 'library opened' /tmp/bookx.log || fail "app did not start: $(grep -E 'panicked|error|Gtk-WARNING|cannot' /tmp/bookx.log | head -2 | tr '\n' ' ' | cut -c1-200)"
echo "RESULT PASS $v"
INNER

ref() { # short name -> full image reference
    case "$1" in
        fedora:*) echo "registry.fedoraproject.org/$1" ;;
        opensuse/*) echo "registry.opensuse.org/$1" ;;
        */*) echo "docker.io/$1" ;;
        *) echo "docker.io/library/$1" ;;
    esac
}

run_one() { # image kind tier
    local image="$1" kind="$2" tier="$3" file log
    case "$kind" in deb) file=$(basename "$deb") ;; rpm-*) file=$(basename "$rpm") ;; tar-*) file=$(basename "$tar") ;; esac
    log="$work/$(echo "$image" | tr '/:' '__').log"
    timeout 900 "$engine" run --rm -v "$dir:/rel:ro" -v "$work/inner.sh:/inner.sh:ro" -e KIND="$kind" -e FILE="$file" \
        "$(ref "$image")" bash /inner.sh > "$log" 2>&1
    local rc=$?
    local res
    res=$(grep -a '^RESULT ' "$log" | tail -1)
    [ -n "$res" ] || res="RESULT FAIL container exit $rc: $(tail -n 2 "$log" | tr '\n' ' ' | cut -c1-160)"
    printf '%s\t%s\t%s\t%s\n' "$tier" "$image" "$kind" "${res#RESULT }" > "$log.res"
    echo "$(cut -f2-4 "$log.res" | tr '\t' ' ')"
}

list=$( { echo "$SUPPORTED" | awk 'NF{print $1, $2, "supported"}'; echo "$BEST_EFFORT" | awk 'NF{print $1, $2, "best-effort"}'; } )
if [ -n "${ONLY:-}" ]; then list=$(echo "$list" | awk -v only=" $ONLY " 'index(only, " " $1 " ")'); fi

running=0
while read -r image kind tier; do
    [ -n "$image" ] || continue
    ( run_one "$image" "$kind" "$tier" ) &
    running=$((running + 1))
    if [ "$running" -ge "$jobs" ]; then wait -n; running=$((running - 1)); fi
done <<< "$list"
wait

echo
echo "tier         image                          package     result"
cat "$work"/*.res 2>/dev/null | sort | awk -F'\t' '{printf "%-12s %-30s %-11s %s\n", $1, $2, $3, $4}'
bad=$(cat "$work"/*.res 2>/dev/null | awk -F'\t' '$1=="supported" && $4 !~ /^PASS/' | wc -l)
if [ "$bad" -gt 0 ]; then echo "distro-matrix: $bad supported image(s) FAILED"; exit 1; fi
echo "distro-matrix: every supported image passed"
