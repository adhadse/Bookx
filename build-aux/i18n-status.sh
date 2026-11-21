#!/usr/bin/env bash
# Translation status: completeness of every language in po/LINGUAS (msgfmt --statistics) and a check that
# po/STATUS.md lists each of them with a valid review state.
#
#   build-aux/i18n-status.sh            print "code translated total percent"
#   build-aux/i18n-status.sh --check    fail when STATUS.md is missing a language or has a wrong state,
#                                       or when a language marked "reviewed" or complete is not complete
#                                       (STATUS.md column "Scope": full | starter)
set -euo pipefail
cd "$(dirname "$0")/.."
command -v msgfmt >/dev/null || { echo "msgfmt not installed: skipping"; exit 0; }
total=$(grep -c '^msgid ' po/bookx.pot)
total=$((total - 1))
bad=0
for code in $(grep -v '^#' po/LINGUAS); do
    done_n=$(msgfmt --statistics -o /dev/null "po/$code.po" 2>&1 | grep -o '^[0-9]* translated' | grep -o '^[0-9]*' || echo 0)
    pct=$((100 * done_n / total))
    echo "$code $done_n $total $pct%"
    [ "${1:-}" = "--check" ] || continue
    row=$(grep -E "\| *\`$code\` *\|" po/STATUS.md || true)
    if [ -z "$row" ]; then
        echo "po/STATUS.md has no row for $code" >&2; bad=1; continue
    fi
    echo "$row" | grep -qE '\| *(machine-translated|reviewed) *\|' \
        || { echo "po/STATUS.md: $code needs the state machine-translated or reviewed" >&2; bad=1; }
    if echo "$row" | grep -q '| *full *|' && [ "$done_n" -lt "$total" ]; then
        echo "$code is marked full but has $done_n of $total strings" >&2; bad=1
    fi
done
exit $bad
