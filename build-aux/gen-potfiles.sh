#!/usr/bin/env bash
# Writes po/POTFILES.in: the source files that contain a translatable marker (tr("…"), trf("…", …),
# ntr("…", "…", n), tr_c("ctx", "…"), N_("…")), one per line, sorted. gettext tools (xgettext, Weblate's
# "update PO files", Damned Lies, translate-toolkit) read this list. The program's own extractor
# (src/i18n.rs, used by build-aux/i18n.sh) scans every .rs file under src/ and does not need it.
#
#   build-aux/gen-potfiles.sh           rewrite po/POTFILES.in
#   build-aux/gen-potfiles.sh --check   fail when po/POTFILES.in is out of date (CI)
set -euo pipefail
cd "$(dirname "$0")/.."

list() {
    grep -rlE '(^|[^A-Za-z0-9_.])(tr|trf|ntr|tr_c|N_)\([[:space:]]*"' src --include='*.rs' \
        | grep -v '^src/i18n\.rs$' | LC_ALL=C sort
}

if [ "${1:-}" = "--check" ]; then
    if ! list | diff -u po/POTFILES.in - >/dev/null; then
        echo "po/POTFILES.in is out of date: run build-aux/gen-potfiles.sh" >&2
        exit 1
    fi
    exit 0
fi

list >po/POTFILES.in
echo "po/POTFILES.in: $(wc -l <po/POTFILES.in) files"
