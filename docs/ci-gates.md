# CI gates: code coverage

Internal notes (not for the wiki). The single entry point is `build-aux/coverage.sh`; CI runs the same script.

## Tools (nothing is home-made except the table)

| What | Tool |
|---|---|
| Measuring | `cargo-llvm-cov` 0.8+ (LLVM source-based coverage, `-C instrument-coverage`; the Rust counterpart of pytest-cov) |
| Running the tests | `cargo-nextest` (one process per test, all cores, no doc-tests) |
| Patch coverage (lines changed in a PR) | `diff-cover` (pip) on the filtered lcov file |
| Table, baseline, gate | `build-aux/coverage-gate.py` (reads the lcov file: grouping, comparison, exit code) |

Install once: `cargo install cargo-llvm-cov cargo-nextest --locked`, `rustup component add llvm-tools-preview`
(a distribution rustc without rustup: set `LLVM_COV` and `LLVM_PROFDATA` to the tools of the same LLVM version as
`rustc -vV`). CI installs the two cargo tools with `taiki-e/install-action` (pinned by SHA).

## Commands

```bash
build-aux/coverage.sh                    # the gate: whole workspace, ~2 minutes on 32 cores
build-aux/coverage.sh --quick            # GTK-free crates only (does not build the app crate)
build-aux/coverage.sh --html             # HTML report: target/llvm-cov/html/index.html
build-aux/coverage.sh --update-baseline  # after improving coverage: raise build-aux/coverage-baseline.json
build-aux/coverage.sh --base origin/main # also: lines changed since that ref must be >= 80% covered
build-aux/coverage.sh --no-run           # report again from the last run
NET=0 build-aux/coverage.sh              # offline figure (loopback tests skipped: lower)
build-aux/coverage-ui.sh                 # UI tier through the Broadway scenarios (slow)
```

Outputs are in `$CARGO_TARGET_DIR/llvm-cov/` (`lcov.info`, `lcov.filtered.info`, `summary.json` with region and
function numbers, `summary.md`, `html/`). The instrumented build lives in `llvm-cov-target/` next to it (about 30 GB
for the whole workspace; CI caches it, a developer can delete it).

## What is measured, and what is left out

The number is **line coverage of application code**. Left out, because counting it would inflate the figure:

* inline `#[cfg(test)]` modules (their lines are always "covered"; in the app crate they were more than half of the logic lines),
  `tests/`, `examples/`, `benches/`, `fuzz/`, and test support compiled into libraries (`testutil`, `testgen`,
  `test_support`, `tests_extra`);
* in the app crate `bookx`, the GTK glue: `src/ui/**`, `src/app.rs`, `src/main.rs`, `src/*_ui.rs`,
  `src/devtools.rs`, `src/kobo_real_check.rs` and `src/voice_download.rs` (a GLib main-loop job). They form the
  **UI tier**, exercised by the Broadway scenarios, not by `cargo test`.

Groups: every `crates/bookx-*` crate, `bookx-logic` (the GTK-free modules of `src/`), `bookx-ui` (the glue above).
Unit tests cannot reach `bookx-ui` (about 5% with `cargo test`), that is expected.

## The policy

**Logic tier (per PR, `build-aux/coverage.sh`)**

1. The whole logic tier must be at or above 90% (`target` in the baseline file). Today it is 92.9%.
2. A ratchet per group: no crate may fall more than 0.2 points below its number in `build-aux/coverage-baseline.json`.
   The baseline only goes up (`--update-baseline` keeps the higher value); lower it by hand only with a reason in the
   change description.
3. A new crate must start at 90% or more (or be added to the baseline by `--update-baseline` on purpose).
4. Patch coverage: lines a PR adds or changes in logic code must be covered at least 80% (`diff-cover`). UI glue is
   not counted there (it is not in `lcov.filtered.info`).
5. Any failing test fails the job, and a broken run is not trusted for coverage.

**UI tier (nightly or on demand, `build-aux/coverage-ui.sh`)**: the app is built with instrumentation, every scenario
process writes its own `.profraw` when it quits (`quit` dev command), the profiles are merged. The number is ratcheted
in the same baseline file (`bookx-ui`). It is not a per-PR check: it needs Broadway and takes as long as the whole
UI suite. A scenario that the harness kills writes no profile, so the figure is a lower bound.

**The 90% goal per crate**, below it today (see the table printed by the script): bookx-audio (real audio device
code), bookx-cli (device and backup commands), bookx-secrets (the Secret Service/`oo7` store needs a keyring daemon),
bookx-tts (ONNX engines need model files), bookx-logic (device sync, devices, config load/save, i18n tools).
Each should reach 90% by tests with fakes (a fake Secret Service on the session bus, tiny ONNX models, a fake
audio sink); do that crate by crate and run `--update-baseline`.

## Making `coverage` a required check

The job is `coverage` in `.github/workflows/ci.yml` (name "Coverage (cargo-llvm-cov, nextest)"). It only blocks a merge
when GitHub is told it is required. In the web UI: Settings → Branches (or Rules → Rulesets) → Add rule for `main` →
tick "Require status checks to pass before merging" → search for `Coverage (cargo-llvm-cov, nextest)` → also tick
"Require branches to be up to date before merging" → Save.

The same with the API (needs admin rights; at the time of writing `main` has **no** protection rule, so this creates one):

```bash
gh api -X PUT repos/adhadse/Bookx/branches/main/protection --input - <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "checks": [
      { "context": "Coverage (cargo-llvm-cov, nextest)" },
      { "context": "Format, lint, test (stable Rust)" }
    ]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": null,
  "restrictions": null
}
JSON
```

Check the exact context strings first: `gh api repos/adhadse/Bookx/commits/<sha>/check-runs --jq '.check_runs[].name'`
after the first CI run (a context that never reports blocks every PR for ever).

## Known limits

* Line coverage counts a line as covered when it ran, not that anything was asserted about it. Review tests too.
* Region and function coverage are in `summary.json` and the HTML report, not gated.
* Inline test modules are found by their `#[cfg(test)]` attribute at column 0 up to the closing `}` at column 0
  (rustfmt layout). A `#[cfg(test)]` item that is indented is counted as code.
* `NET=0` skips the loopback server tests, so the gate runs without it; the offline figure is a little lower.
* The first CI run has no cache for the instrumented build (a few minutes more).
