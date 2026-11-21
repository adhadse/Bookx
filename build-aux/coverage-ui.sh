#!/usr/bin/env bash
# UI-tier coverage: how much of src/ui/** (and the other GTK glue) the Broadway scenarios reach. The app and
# bookx-cli are built with LLVM coverage instrumentation (cargo llvm-cov show-env), every scenario process writes
# its own .profraw when it quits, cargo llvm-cov merges them. Slow (minutes to an hour): a nightly or on-demand check,
# not a per-PR one.
#
#   build-aux/coverage-ui.sh                       all scenarios
#   ONLY='notes import' build-aux/coverage-ui.sh   some scenarios (see ui-tests.sh)
#   JOBS=8 build-aux/coverage-ui.sh                scenarios at once
#   build-aux/coverage-ui.sh --update-baseline     raise the bookx-ui number in build-aux/coverage-baseline.json
#   build-aux/coverage-ui.sh --html                also an HTML report in <target>/llvm-cov/html-ui
#
# A scenario killed by the harness (timeout, crash) writes no profile: its coverage is lost, so the number is a lower bound.
# Needs gtk4-broadwayd and dbus-run-session like ui-tests.sh. When ONLY is set the gate is not applied.
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.." || exit 2

HTML=0 UPDATE=0
for a in "$@"; do
    case "$a" in
        --html) HTML=1 ;;
        --update-baseline) UPDATE=1 ;;
        -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 2 ;;
    esac
done
for t in cargo-llvm-cov gtk4-broadwayd dbus-run-session python3; do
    command -v "$t" >/dev/null 2>&1 || { echo "coverage-ui: $t is missing" >&2; exit 2; }
done

OUT="${CARGO_TARGET_DIR:-target}/llvm-cov"
mkdir -p "$OUT"
# the instrumented binaries live in cargo-llvm-cov's own target directory, shared with build-aux/coverage.sh
eval "$(cargo llvm-cov show-env --export-prefix)"
cargo llvm-cov clean --workspace >/dev/null 2>&1

t0=$(date +%s)
NET="${NET:-0}" BOOKX_SECRETS="${BOOKX_SECRETS:-memory}" build-aux/ui-tests.sh
rc_ui=$?
t1=$(date +%s)
echo "coverage-ui: scenarios took $((t1 - t0))s (exit $rc_ui); profiles: $(find "${CARGO_LLVM_COV_TARGET_DIR:-$CARGO_TARGET_DIR}" -maxdepth 1 -name '*.profraw' | wc -l)"

IGNORE='(^|/)(tests|examples|benches|fuzz)/'
cargo llvm-cov report --workspace --lcov --output-path "$OUT/lcov-ui.info" --ignore-filename-regex "$IGNORE" >/dev/null || exit 2
[ "$HTML" = 1 ] && cargo llvm-cov report --workspace --html --output-dir "$OUT/ui" --ignore-filename-regex "$IGNORE" >/dev/null
args=("$OUT/lcov-ui.info" --out "$OUT/ui" --tier ui)
[ -n "${ONLY:-}" ] && args+=(--partial)
[ "$UPDATE" = 1 ] && args+=(--update-baseline)
python3 build-aux/coverage-gate.py "${args[@]}"
rc_gate=$?
[ "$rc_ui" = 0 ] && [ "$rc_gate" = 0 ]
