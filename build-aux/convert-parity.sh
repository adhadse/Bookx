#!/usr/bin/env bash
# Parity of Bookx's converters with Calibre's ebook-convert.
#
#   build-aux/convert-parity.sh            full matrix (about 2-3 minutes with 30 cores)
#   build-aux/convert-parity.sh --smoke    a few cells (seconds)
#   build-aux/convert-parity.sh --report   list differences instead of failing on unlisted ones
#   build-aux/convert-parity.sh --fixtures regenerate the committed Calibre-made fixtures
#   build-aux/convert-parity.sh --update-doc  also rewrite the matrix in docs/convert-parity.md
#
# Skips cleanly (exit 0) when ebook-convert is missing. JOBS=n sets the concurrency.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v ebook-convert >/dev/null 2>&1; then
  echo "convert-parity: ebook-convert not found, skipping (the committed fixtures still run in cargo test)"
  exit 0
fi

mode=full; report=0; update=0
for a in "$@"; do
  case "$a" in
    --smoke) mode=smoke ;;
    --report) report=1 ;;
    --fixtures) mode=fixtures ;;
    --update-doc) update=1 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

out="${PARITY_OUT:-${TMPDIR:-/tmp}/bookx-parity-out}"
mkdir -p "$out"
export CALIBRE_CONFIG_DIRECTORY="$out/calibre-config"

if [ "$mode" = fixtures ]; then
  src="$out/src"; dest=crates/bookx-convert/tests/fixtures/calibre-made
  mkdir -p "$src" "$dest"
  BOOKX_DUMP_CORPUS="$src" cargo test -q -p bookx-convert --test dump_corpus
  for f in mobi azw3 pdb pmlz rb snb fb2 docx rtf txtz htmlz; do
    ebook-convert "$src/epub3-features.epub" "$dest/features.$f" >/dev/null
  done
  ebook-convert "$src/epub-long-toc.epub" "$dest/long-toc.azw3" >/dev/null
  ebook-convert "$src/epub-rtl.epub" "$dest/rtl.azw3" >/dev/null
  ebook-convert "$src/epub-ruby-cjk.epub" "$dest/cjk.mobi" >/dev/null
  echo "fixtures written to $dest"
  exit 0
fi

[ "$mode" = full ] && export BOOKX_PARITY=1
[ "$report" = 1 ] && export BOOKX_PARITY_REPORT=1
export BOOKX_PARITY_OUT="$out"
echo "convert-parity: $(ebook-convert --version | head -1), mode $mode"
cargo test --release -p bookx-convert --test parity -- --nocapture
echo "matrix and details: $out/parity.md  (json: $out/parity.json)"
if [ "$update" = 1 ] && [ "$mode" = full ]; then
  python3 - "$out/parity.md" <<'PY'
import re, sys
doc = open("docs/convert-parity.md").read()
matrix = open(sys.argv[1]).read().split("\n\n- ")[0].strip()
doc = re.sub(r"(<!-- matrix:begin -->\n).*?(\n<!-- matrix:end -->)", lambda m: m.group(1) + matrix + m.group(2), doc, flags=re.S)
open("docs/convert-parity.md", "w").write(doc)
PY
fi
