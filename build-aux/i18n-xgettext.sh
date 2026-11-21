#!/usr/bin/env bash
# Extracts the marked strings with GNU xgettext (the way Weblate/Damned Lies/any gettext tool would) and
# compares them with po/bookx.pot, which the program's own scanner writes. Informational: prints how many
# messages each side has that the other lacks. Known, accepted differences are listed in
# docs/translations-hosting.md ("Extraction parity").
#
#   build-aux/i18n-xgettext.sh [out.pot]
set -euo pipefail
cd "$(dirname "$0")/.."
command -v xgettext >/dev/null || { echo "xgettext not installed: skipping"; exit 0; }
out="${1:-$(mktemp -d)/xgettext.pot}"
build-aux/gen-potfiles.sh --check || build-aux/gen-potfiles.sh
xgettext -L Rust --from-code=UTF-8 --no-location --add-comments=TRANSLATORS \
    --package-name=Bookx \
    --keyword=tr:1 --keyword=trf:1 --keyword=N_:1 --keyword=ntr:1,2 --keyword=tr_c:1c,2 \
    --files-from=po/POTFILES.in -o "$out" 2>&1 | grep -v 'embedded URL\|vc-mtime' || true
echo "xgettext: $(grep -c '^msgid ' "$out") messages (po/bookx.pot: $(grep -c '^msgid ' po/bookx.pot))"
echo "in po/bookx.pot but not found by xgettext:"
msgcmp --use-untranslated "$out" po/bookx.pot 2>&1 | grep -c 'this message is used but not defined' || true
echo "found by xgettext but not in po/bookx.pot:"
msgcmp --use-untranslated po/bookx.pot "$out" 2>&1 | grep -c 'this message is used but not defined' || true
echo "wrote $out"
