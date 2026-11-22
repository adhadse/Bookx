#!/usr/bin/env bash
# Which parts of CI a change needs. Prints `rust=true|false`, `wiki=...`, `flatpak=...`, `ui=...` and `ui_reason=...`
# (CI appends it to $GITHUB_OUTPUT). `ui` (the Broadway scenarios) is decided by build-aux/ci-ui-scope.py from the crate
# graph; FORCE_UI=true (the manual `force_ui` input, the `run-ui-tests` label) turns it on. Everything runs for anything but a pull request, so main keeps its caches and a release
# tag always has a complete result. A skipped job still lets the `required` check pass (docs/ci-gates.md).
#
#   EVENT=pull_request BASE=<sha> build-aux/ci-changes.sh     # the files changed since BASE
#   FILES="docs/a.md wiki/docs/b.md" build-aux/ci-changes.sh   # the same decision for a list (tests)
set -euo pipefail

files=${FILES:-}
if [ -z "$files" ] && [ "${EVENT:-}" = pull_request ] && [ -n "${BASE:-}" ]; then
    files=$(git diff --name-only "$BASE"...HEAD)
elif [ -z "$files" ]; then
    printf 'rust=true\nwiki=true\nflatpak=true\nui=true\nui_reason=ran because this is not a pull request (main, tags and manual runs always run everything)\n'
    exit 0
fi

any() { # any REGEX: some changed file matches
    printf '%s\n' $files | grep -Eq "$1"
}
# text only: documentation, the wiki, licences (nothing the compiler or the packaging reads)
if printf '%s\n' $files | grep -Evq '^(docs/|wiki/|_developer/|mockups/|[A-Za-z_-]+\.md$|LICENSE|NOTICE\.md$)'; then rust=true; else rust=false; fi
if any '^(wiki/|build-aux/(wiki|check-)|build-aux/wiki-pictures\.txt|\.github/workflows/ci\.yml)'; then wiki=true; else wiki=false; fi
if any '^(packaging/|data/|Makefile|Cargo\.lock|build-aux/(com\.adhadse\.Bookx\.json|cargo-sources\.json|flatpak)|\.github/workflows/(_flatpak|ci)\.yml)'; then flatpak=true; else flatpak=false; fi
if [ "${FORCE_UI:-}" = true ]; then
    ui_out=$(printf 'ui=true\nui_reason=ran because it was forced (the run-ui-tests label or force_ui)\n')
else
    ui_out=$(cd "$(dirname "$0")/.." && python3 build-aux/ci-ui-scope.py $files)
fi
printf 'rust=%s\nwiki=%s\nflatpak=%s\n%s\n' "$rust" "$wiki" "$flatpak" "$ui_out"
