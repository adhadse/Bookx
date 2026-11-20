#!/usr/bin/env bash
# Checks that the tree is ready to be tagged. Publishes nothing, needs no network.
#
#   build-aux/release-check.sh            # or: make release-check
#   build-aux/release-check.sh v0.3.0     # also: the tag matches the version (the release workflow does this)
#
# Checks: every crate and the metainfo and the CHANGELOG agree on the version; the Flatpak sources match
# Cargo.lock; the plugin golden files are unchanged; desktop file, AppStream metainfo and MIME XML validate;
# the deb/rpm assets and the icons exist; the workflow files parse. A tool that is not installed
# (desktop-file-validate, appstreamcli, xmllint) is reported as skipped, never as passed.
set -uo pipefail
cd "$(dirname "$0")/.."

tag="${1:-}"
fail=0
skipped=()
ok() { printf '  ok    %s\n' "$*"; }
bad() { printf '  FAIL  %s\n' "$*" >&2; fail=1; }
skip() { printf '  skip  %s\n' "$*"; skipped+=("$1"); }
APP_ID=com.adhadse.Bookx

echo "==> versions"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { bad "no version in Cargo.toml"; exit 1; }
ok "Cargo.toml: $version"
for f in crates/*/Cargo.toml; do
    v=$(sed -n 's/^version = "\(.*\)"/\1/p' "$f" | head -1)
    [ "$v" = "$version" ] || bad "$f has version $v"
done
meta=$(sed -n 's/.*<release version="\([^"]*\)".*/\1/p' data/$APP_ID.metainfo.xml | head -1)
[ "$meta" = "$version" ] && ok "metainfo newest release: $meta" || bad "metainfo newest release is '$meta'"
log=$(sed -n 's/^## \[\([0-9][^]]*\)\].*/\1/p' CHANGELOG.md | head -1)
[ "$log" = "$version" ] && ok "CHANGELOG newest version: $log" || bad "CHANGELOG newest version is '$log'"
if [ -n "$tag" ]; then
    [ "${tag#v}" = "$version" ] && ok "tag $tag matches" || bad "tag $tag does not match version $version"
fi

echo "==> Flatpak sources and manifest"
if python3 build-aux/flatpak-cargo-sources.py --check >/dev/null 2>&1; then ok "cargo-sources.json matches Cargo.lock"
else bad "cargo-sources.json is stale: python3 build-aux/flatpak-cargo-sources.py"; fi
if python3 - <<'PY'
import json
m = json.load(open("build-aux/com.adhadse.Bookx.json"))
assert m["id"] == "com.adhadse.Bookx"
for a in m["finish-args"]:
    assert a not in ("--filesystem=host", "--filesystem=home", "--device=all", "--socket=session-bus", "--socket=system-bus"), a
PY
then ok "manifest valid, no broad permissions"; else bad "manifest invalid or too broad"; fi

echo "==> plugin golden files (ABI compatibility)"
if (cd crates/bookx-plugin/tests/golden && sha256sum --quiet -c SHA256SUMS); then ok "golden plugins unchanged"
else bad "golden plugin files changed: this is an ABI break unless intended (docs/plugin-development.md)"; fi

echo "==> desktop integration"
if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate data/$APP_ID.desktop && ok "desktop-file-validate" || bad "desktop-file-validate"
else skip "desktop-file-validate not installed"; fi
if command -v appstreamcli >/dev/null; then
    appstreamcli validate --no-net data/$APP_ID.metainfo.xml >/dev/null && ok "appstreamcli validate" || bad "appstreamcli validate"
else skip "appstreamcli not installed"; fi
if command -v xmllint >/dev/null; then
    xmllint --noout data/$APP_ID.mime.xml && ok "mime xml" || bad "mime xml"
else skip "xmllint not installed"; fi

echo "==> package contents"
for f in $(grep -o '"data/[^"]*"\|"README.md"\|"NOTICE.md"\|"CHANGELOG.md"\|"LICENSE"' Cargo.toml | tr -d '"' | sort -u); do
    [ -e "$f" ] || bad "packaging metadata names a missing file: $f"
done
ok "deb/rpm asset sources exist (data files and docs)"
[ -f data/icons/hicolor/scalable/apps/$APP_ID.svg ] && [ -f data/icons/hicolor/scalable/apps/$APP_ID-symbolic.svg ] \
    && ok "scalable and symbolic icons" || bad "app icons missing"

echo "==> workflows"
if python3 -c 'import yaml' 2>/dev/null; then
    if python3 - <<'PY'
import glob, yaml
for f in sorted(glob.glob(".github/workflows/*.yml")):
    d = yaml.safe_load(open(f))
    assert "jobs" in d and d["jobs"], f
    for n, j in d["jobs"].items():
        assert "runs-on" in j, (f, n)
PY
    then ok "workflow YAML parses (never run on GitHub from here)"; else bad "workflow YAML"; fi
else skip "python yaml not installed"; fi
command -v actionlint >/dev/null && { actionlint && ok "actionlint" || bad "actionlint"; } || skip "actionlint not installed"

echo
if [ "$fail" != 0 ]; then echo "release-check: FAILED" >&2; exit 1; fi
if [ "${#skipped[@]}" != 0 ]; then echo "release-check: passed, but skipped: ${skipped[*]}"; else echo "release-check: passed"; fi
