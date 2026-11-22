#!/usr/bin/env bash
# Code coverage gate (cargo-llvm-cov: LLVM source-based coverage, run by cargo-nextest). One entry point for
# developers and CI.
#
#   build-aux/coverage.sh                  the gate: every test of the workspace, lines per crate vs the baseline
#   build-aux/coverage.sh --quick          the GTK-free crates only (no bookx app crate): fast local check
#   build-aux/coverage.sh --html           also write an HTML report to <target>/llvm-cov/html
#   build-aux/coverage.sh --update-baseline  raise build-aux/coverage-baseline.json to the measured numbers
#   build-aux/coverage.sh --base REF       also require >= 80% coverage of the lines changed since REF (diff-cover)
#   build-aux/coverage.sh --no-run         report again from the last run (no tests)
#
# Needs: cargo install cargo-llvm-cov cargo-nextest --locked ; rustup component add llvm-tools-preview
# (a distribution rustc without llvm-tools: set LLVM_COV and LLVM_PROFDATA to the tools of the same LLVM).
# The 90% policy, what is excluded and why: docs/ci-gates.md. The UI tier (src/ui) is measured by
# build-aux/coverage-ui.sh.
# Environment: CARGO_TARGET_DIR (default target), JOBS / NEXTEST_TEST_THREADS (test concurrency).
set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")/.." || exit 2

QUICK=0 HTML=0 UPDATE=0 RUN=1 BASE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --quick) QUICK=1 ;;
        --html) HTML=1 ;;
        --update-baseline) UPDATE=1 ;;
        --no-run) RUN=0 ;;
        --base) BASE="${2:?--base needs a git ref}"; shift ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "coverage: $1 is missing: $2" >&2; exit 2; }; }
need cargo-llvm-cov "cargo install cargo-llvm-cov --locked"
need cargo-nextest "cargo install cargo-nextest --locked"
need python3 "python3 is needed for the table"

OUT="${CARGO_TARGET_DIR:-target}/llvm-cov"
mkdir -p "$OUT"
# NET is left alone on purpose: the "network" tests talk to loopback only, and skipping them (NET=0) would hide
# the HTTP client and server code from the numbers. NET=0 build-aux/coverage.sh gives the lower, offline figure.
export BOOKX_SECRETS="${BOOKX_SECRETS:-memory}"
[ -n "${JOBS:-}" ] && export NEXTEST_TEST_THREADS="$JOBS"
# integration tests, examples and fuzz targets are not application code
IGNORE='(^|/)(tests|examples|benches|fuzz)/'
SCOPE=(--workspace)
[ "$QUICK" = 1 ] && SCOPE+=(--exclude bookx)

t0=$(date +%s)
rc_tests=0
if [ "$RUN" = 1 ]; then
    cargo llvm-cov clean --workspace >/dev/null 2>&1
    # perf budgets are timing tests (meaningless when instrumented); doc-tests are not run by nextest
    cargo llvm-cov nextest "${SCOPE[@]}" --locked --no-report --no-fail-fast \
        -E 'not binary(perf_budgets)' || rc_tests=$?
fi
t1=$(date +%s)
# the JUnit file of the run, for the PR report (the profile in .config/nextest.toml writes it)
junit=$(find "${CARGO_TARGET_DIR:-target}" -path '*nextest/default/junit.xml' -print -quit 2>/dev/null)
[ -n "$junit" ] && cp "$junit" "$OUT/junit.xml"

cargo llvm-cov report --workspace --lcov --output-path "$OUT/lcov.info" --ignore-filename-regex "$IGNORE" >/dev/null || exit 2
if [ "$HTML" = 1 ]; then
    cargo llvm-cov report --workspace --html --output-dir "$OUT" --ignore-filename-regex "$IGNORE" >/dev/null || exit 2
    echo "coverage: HTML report in $OUT/html/index.html"
fi
cargo llvm-cov report --workspace --summary-only --json --output-path "$OUT/summary.json" --ignore-filename-regex "$IGNORE" >/dev/null || exit 2

args=("$OUT/lcov.info" --out "$OUT")
[ "$QUICK" = 1 ] && args+=(--partial)
[ "$UPDATE" = 1 ] && args+=(--update-baseline)
python3 build-aux/coverage-gate.py "${args[@]}"
rc_gate=$?
t2=$(date +%s)

rc_patch=0
if [ -n "$BASE" ]; then
    if command -v diff-cover >/dev/null 2>&1; then
        echo
        echo "coverage: patch coverage against $BASE (lines changed in logic code must be >= 80% covered)"
        diff-cover "$OUT/lcov.filtered.info" --compare-branch "$BASE" --fail-under 80 \
            --format "markdown:$OUT/patch.md,json:$OUT/patch.json" || rc_patch=$?
        [ -n "${GITHUB_STEP_SUMMARY:-}" ] && [ -f "$OUT/patch.md" ] && cat "$OUT/patch.md" >>"$GITHUB_STEP_SUMMARY"
    else
        echo "coverage: diff-cover is not installed (pip install diff-cover): patch coverage skipped" >&2
    fi
fi

printf 'coverage: tests+instrumented build %ss, report and gate %ss\n' "$((t1 - t0))" "$((t2 - t1))"
if [ "$rc_tests" != 0 ]; then
    echo "coverage: some tests FAILED (exit $rc_tests); coverage of a broken run is not trustworthy" >&2
    exit 1
fi
[ "$rc_gate" = 0 ] && [ "$rc_patch" = 0 ]
