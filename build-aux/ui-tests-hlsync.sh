#!/usr/bin/env bash
# End-to-end check of the highlight colour/style repair (docs/highlight-sync-rca.md) without GTK or
# Broadway: the real `bookx-cli` against a sandbox library whose 36 highlights are all yellow and plain
# (what every import stored before the device looks were understood) and a device folder with a KOReader
# sidecar that has every colour with every drawer. The plugin -> server leg is the Rust test
# `a_real_shaped_sidecar_keeps_every_look_over_the_wire` (crates/bookx-serve/tests/koreader_annotations.rs).
#
#   build-aux/ui-tests-hlsync.sh            run
#   CARGO_TARGET_DIR=... build-aux/ui-tests-hlsync.sh
#
# Self-contained and parallel safe: its own temporary library, never touches ~/Documents or the user's
# settings (`--config-dir` points at an empty folder). Needs python3 (sqlite3 module). Exit 1 on a failed check.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."

if ! command -v python3 >/dev/null 2>&1; then
    echo "SKIP hlsync: python3 not installed"
    exit 0
fi
TARGET="${CARGO_TARGET_DIR:-target}"
cargo build -q -p bookx-cli || { echo "FAIL hlsync: build"; exit 1; }
CLI="$TARGET/debug/bookx-cli"
FIXTURE="$PWD/crates/bookx-serve/tests/fixtures/koreader_all_looks.lua"
D=$(mktemp -d "${TMPDIR:-/tmp}/bookx-hlsync.XXXXXX")
trap 'rm -rf "$D"' EXIT
LIB="$D/lib"
fail=0
check() { # name, condition result (0 = ok)
    if [ "$2" -eq 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}

mkdir -p "$D/src" "$D/dev/Looks.sdr" "$D/cfg"
printf -- '---\ntitle: "Looks of Highlights"\nauthor: "Ann Author"\n---\n# One\n\nText.\n' >"$D/src/Looks.md"
cp "$FIXTURE" "$D/dev/Looks.sdr/metadata.epub.lua"
"$CLI" --library "$LIB" library add "$D/src/Looks.md" >/dev/null
check "library created" $?

# seed 36 wrong highlights straight into the catalogue (what the old code stored)
python3 - "$LIB/bookx-library.db" "$FIXTURE" <<'PY'
import json, re, sqlite3, sys
db, fixture = sys.argv[1], sys.argv[2]
text = open(fixture).read()
rows = re.findall(r'\["datetime"\] = "([^"]+)".*?\["text"\] = "([^"]+)"', text, re.S)
con = sqlite3.connect(db)
(bid, data), = con.execute("select id, data from books").fetchall()
d = json.loads(data)
d["highlights"] = [
    {"id": f"bx-{i}", "spine": 0, "start": i, "end": i + 3, "color": "yellow", "style": "", "note": "",
     "text": t, "created": dt.replace(" ", "T") + "+00:00"}
    for i, (dt, t) in enumerate(rows)
]
con.execute("update books set data=? where id=?", (json.dumps(d), bid))
con.commit()
print(len(rows), "seeded")
PY
check "36 highlights seeded" $?

looks() {
    python3 - "$LIB/bookx-library.db" <<'PY'
import json, sqlite3, sys
con = sqlite3.connect(sys.argv[1])
(data,), = con.execute("select data from books").fetchall()
hs = json.loads(data)["highlights"]
print(len(hs), len({h["color"] for h in hs}), len({h["style"] or "background" for h in hs}),
      sum(1 for h in hs if h["color"] == "yellow" and not h["style"]))
PY
}

[ "$(looks)" = "36 1 1 36" ]; check "before: 36 highlights, one colour, one style" $?

out=$("$CLI" --library "$LIB" highlights repair --config-dir "$D/cfg" --from "$D/dev")
echo "$out" | grep -q "35 highlight(s) corrected"; check "dry run reports 35" $?
[ "$(looks)" = "36 1 1 36" ]; check "dry run changes nothing" $?

out=$("$CLI" --library "$LIB" highlights repair --config-dir "$D/cfg" --from "$D/dev" --apply)
echo "$out" | grep -q "35 highlight(s) changed"; check "apply changes 35" $?
[ "$(looks)" = "36 9 4 1" ]; check "after: 9 colours, 4 styles, one left at the default" $?

out=$("$CLI" --library "$LIB" highlights repair --config-dir "$D/cfg" --from "$D/dev" --apply)
echo "$out" | grep -q "0 highlight(s) changed"; check "second apply is a no-op" $?

"$CLI" --library "$LIB" library undo >/dev/null
[ "$(looks)" = "36 1 1 36" ]; check "undo brings every look back" $?

exit $fail
