# CI: layout, gates, caching, coverage

Internal notes (not for the wiki). Everything a pull request has to pass is one workflow, `ci.yml`. The single
entry point for coverage is `build-aux/coverage.sh`; CI runs the same script.

Status: living | Last reviewed: 2026-11-23 | Owner: CI agent, reviewed by @adhadse

## Contents

- [The workflows](#the-workflows)
- [What a job does, and how to run it locally](#what-a-job-does-and-how-to-run-it-locally)
- [Caching, speed, parallelism](#caching-speed-parallelism)
- [Why the first runs were red (2026-11-22)](#why-the-first-runs-were-red-2026-11-22)
- [Code coverage](#code-coverage)
- [Scenarios that measure a speed](#scenarios-that-measure-a-speed)

## The workflows

Before (2026-11-21): 9 workflows, 14 jobs, nothing ever green. After: 7 workflows (`ci`, `_flatpak`, `wiki-deploy`, `release`,
`nightly`, `package-native`, `flathub-update`) plus `dependabot-sources`, 13 job definitions in `ci.yml` and the reusable
ones (the UI job is a 10-way matrix), one required check.

| Workflow | Runs on | Jobs |
|---|---|---|
| `ci.yml` (name **CI**) | pull requests, manual, called by `nightly.yml`; a push to `main` only runs the cache warm-up (below) | `changes`, `quick`, `rust`, `build-ui`, `ui` (10 shards), `wiki` (build and check only), `flatpak` (calls `_flatpak.yml`), **`required`**, `report` |
| `_flatpak.yml` | called by `ci.yml`, `release.yml`, `nightly.yml` | `bundle`: the Flatpak bundle, smoke-tested, Flathub linter optional |
| `dependabot-sources.yml` | Dependabot pull requests that change Cargo files | `regenerate`: commits `build-aux/cargo-sources.json`, dispatches CI on the new commit |
| `wiki-deploy.yml` (name **Wiki deploy**) | called by `release.yml`, manual (`ref`) | `deploy`: builds the wiki of a ref and publishes it to GitHub Pages |
| `release.yml` | tags `v*`, manual | `gates`, `version`, `tarball`, `flatpak`, `publish`, `wiki` (after `publish`, tags only) |
| `nightly.yml` | 03:17 UTC, manual | `ci` (the whole of `ci.yml` on main), `coverage-ui` (UI tier), `flatpak` with cold caches, `notify` (the issue "Nightly failed") |
| `package-native.yml` | manual (`os`: macos-arm64, macos-x64, windows) | `macos`, `windows` |
| `flathub-update.yml` | unchanged | dry-run/PR against the Flathub repository |

Old to new:

| Old workflow / job | Now |
|---|---|
| `ci.yml` `lint` (actionlint, lint-checks, `bash -n`, Python tests) | `quick`, separate steps |
| `ci.yml` `koreader-plugin` (Lua tests, zip) | `quick` |
| `ci.yml` `deny` (cargo-deny) | `quick` |
| `ci.yml` `check` (fmt, clippy, tests, desktop files, release-check, mockups) | `rust` |
| `ci.yml` `coverage` | `rust` (the tests ARE the coverage run: `cargo llvm-cov nextest`, no second `cargo test`) |
| `ci.yml` `ui` | `build-ui` once + `ui` shards |
| `pr-report.yml` (never merged as a workflow of its own) | the `report` job of `ci.yml` |
| `ci.yml` `flatpak`, `package-flatpak.yml` | `flatpak` via `_flatpak.yml` |
| `i18n.yml` | steps of `rust` (pot, POTFILES, LINGUAS, msgfmt, status table, reviewed-locales tests) |
| `wiki.yml` `build` | `wiki` (build and check; it no longer deploys) |
| `wiki.yml` `deploy` | `wiki-deploy.yml`, run by `release.yml` after `publish` or by hand |
| `coverage-ui.yml` | `nightly.yml` `coverage-ui` |
| `package-macos.yml`, `package-windows.yml` | `package-native.yml` |
| `release.yml` `tarball`, `flatpak`, `publish` | same, plus `gates` and `version` |

### Branch protection: require exactly one check

**`required`** (job name in `ci.yml`). It needs every other job, fails when any of them failed or was cancelled, and is
green when jobs were skipped on purpose: a pull request that only touches `docs/`, `wiki/` or Markdown skips `rust`,
`build-ui` and `ui`; one that does not touch packaging skips `flatpak`. A required check that is skipped counts as
"pending" for ever, which is why the aggregate job exists and the others are not required one by one.

The check names a run reports (`gh api repos/adhadse/Bookx/commits/<sha>/check-runs --jq '.check_runs[].name'`):

```
Changed paths
Quick checks
Rust (format, lint, tests, coverage)
Build the app for the UI scenarios
UI scenarios (Broadway, shard N/10)          N = 1..10
Wiki (build and check)
Flatpak / Flatpak bundle (x86_64)            only when packaging changed
required                                     <- the only one to require
Report                                       informational, never required
```

Set it (needs admin rights; at the time of writing `main` has no protection):

```bash
gh api -X PUT repos/adhadse/Bookx/branches/main/protection --input - <<'JSON'
{
  "required_status_checks": { "strict": true, "checks": [ { "context": "required" } ] },
  "enforce_admins": false,
  "required_pull_request_reviews": null,
  "restrictions": null
}
JSON
```

### The public wiki is deployed by releases, not by pushes

`ci.yml` builds and checks the wiki (job `wiki`, also feeding `required`) on pull requests and pushes, and never
deploys it: the live site must not describe a feature that no download has yet. `wiki-deploy.yml` publishes it:

* `release.yml` calls it as the last job (`wiki`, `needs: publish`) with the tag, so the docs go live after the release
  assets are out, for pre-release tags too. It is skipped for manual runs without a tag, and a Pages failure leaves the
  release as it is (red job, assets already published).
* by hand (Actions, Wiki deploy, Run workflow) is the owner's override: a manual run deploys whatever ref you pick
  (default `main`; any branch or tag); a release deploys its tag.

The environment `github-pages` restricts deployments to refs it allows. Its policies are `main`, `master` and the tag
pattern `v*` (added 2026-11-22 with `gh api -X POST repos/adhadse/Bookx/environments/github-pages/deployment-branch-policies -f name='v*' -f type=tag`); without the tag policy a tag-triggered deploy is refused.

## What a job does, and how to run it locally

| Job | What | Locally |
|---|---|---|
| `changes` | `build-aux/ci-changes.sh`: which of rust / wiki / flatpak / ui the changed files need. Everything but a pull request runs everything | `FILES="docs/a.md" build-aux/ci-changes.sh` |
| `quick` | `lint-checks.sh`, actionlint, `bash -n`, `python3 -m unittest discover -s build-aux/tests`, KOReader plugin tests (LuaJIT, Lua 5.4), plugin zip, sync endpoint check, cargo-deny. Every step has `if: !cancelled()`, so all report even after a failure; the job is red if any failed | `build-aux/lint-checks.sh`, `actionlint`, `koreader-plugin/tests/run.sh` |
| `rust` | rustfmt, clippy `-D warnings`, i18n gates, `build-aux/coverage.sh --html --base origin/main` (all tests under nextest + the coverage gate + patch coverage), desktop file/metainfo, `release-check.sh`, Flatpak sources vs `Cargo.lock`, mockups | `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `build-aux/coverage.sh` |
| `build-ui`, `ui` | one debug build of the app, then 10 shards of the Broadway scenarios | `build-aux/ui-tests.sh` (all) or `SHARD=3/10 SKIP_BUILD=1 build-aux/ui-tests.sh` |
| `wiki` | `build-aux/wiki.sh` | the same |
| `flatpak` | `build-aux/flatpak.sh` with `BUNDLE=1` on the runner | `BUNDLE=1 build-aux/flatpak.sh` |

### What runs where

| Event | What runs | Why |
|---|---|---|
| pull request | the full pipeline of `ci.yml` (quick, Rust + coverage, UI shards unless the classifier skips them, wiki, Flatpak when packaging changed), `required`, `Report` | branch protection forces the branch to be up to date with `main`, so the merge ref is the tree that `main` will get |
| push to `main` (a merge) | only `warm-rust` and `build-ui` (build, no tests; `build-ui` builds the app and does not pack it): they refill the caches that pull requests restore | the same tree was tested on the pull request; about 110 of the 125 runner-minutes of a merge are saved |
| nightly (03:17 UTC) and manual `nightly.yml` | the whole of `ci.yml` as for any non-PR event (everything on, UI forced), the UI-tier coverage, the cold-cache Flatpak build, and the issue "Nightly failed" | the full suite on `main`; a red run is visible and opens or updates one issue, a green run closes it |
| manual `ci.yml` | everything, or with `warm_only` only the warm-up (to try it on a branch) | |
| tag `v*` | `release.yml`: `gates` first | see below |

A push to `main` has **no `required` check on purpose**: with every job skipped it would be green with nothing tested,
and anything that looks at `required` on a commit (including `release.yml`) would be fooled.

`release.yml` `gates` (`build-aux/release-gate.py`, tested offline in `build-aux/tests/test_release_gate.py`) accepts a commit when, in this order:
`skip_gates` is set (emergency); or its `required` check succeeded (a rerun, a manual CI run on it; a red one refuses at once);
or the latest Nightly run on it succeeded; or it is the merge commit of a pull request whose head has a green `required` check AND
whose head tree equals the tree of the commit (a squash merge of an up-to-date branch). A different tree means the branch was
not up to date when it was merged, nothing proved the result, and the gate says so. A direct push without any of these is
refused. Dry run without building anything: run `release.yml` by hand with `gates_only` on `main`.

### When the UI scenarios run

`build-ui` and the 10 `ui` shards are the expensive part (about 85 runner-minutes, 8 minutes of wall time after the
build). `build-aux/ci-changes.sh` calls `build-aux/ci-ui-scope.py`, which decides `ui=true|false` from the changed files
(`git diff base...head` of a pull request). Nightly and manual runs always run them: the nightly full run on `main` is the
safety net for anything a pull request skipped.

| Changed path | UI scenarios |
|---|---|
| `src/**`, `build.rs`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain*`, `data/**`, `po/**` (strings are on screen) | run |
| any file (not `*.md`) of a workspace crate the app depends on, tests and fixtures included | run |
| `build-aux/ui-tests*`, `ui-shard*`, `session/`, `coverage-ui*`, `.github/workflows/ci.yml` | run |
| `docs/`, `wiki/`, `*.md`, `packaging/`, `koreader-plugin/`, `.github/` templates, other workflows, release/Flatpak/wiki/i18n/CI scripts, tool settings | skipped |
| a workspace crate the app does not depend on | skipped |
| anything else, an empty change | run (fail safe: a path nobody classified, or a change that cannot be shown harmless) |

"Crates the app depends on" is read from the `Cargo.toml` files, not from a list: the roots are `bookx` (the app) and the
programs the scenarios call (`bookx-cli`, and the example programs of `bookx-tts`, `bookx-plugin`, `bookx-comic`,
`bookx-pdf`), followed through normal, build and dev dependencies. A bug in `bookx-core`, `bookx-text` or `bookx-formats`
shows up in the reader and editor scenarios because the app links them, so a short `src/ui/**` list would be wrong. Today
every crate of the workspace is in that set; the rule only skips a crate that nothing the scenarios use depends on.

The decision, with the first matching file, is printed in the `changes` job log and summary and shown in the PR report as
`UI scenarios: skipped: ...` or `UI scenarios: ran because <file> (<area>)`. To force the scenarios on a pull request that
would skip them, add the label **`run-ui-tests`** (adding it starts CI again) or run CI by hand with `force_ui`. `required`
treats the skipped jobs as fine, and it is code that decides, never a label: a label can only add a run.

Verified on real pull requests (2026-11-22): a docs-only pull request reports `UI scenarios: skipped: no UI-relevant change: ...`,
runs no `build-ui` or `ui` job and `required` is green; adding the label `run-ui-tests` to it runs them.

Rust (format, lint, tests, coverage) has its own, unchanged path rule (`ci-changes.sh`, `rust=`). Cases the classifier is
tested on (`build-aux/tests/test_ci_ui_scope.py`): docs only, wiki only, `po/` only, `src/ui`, a library crate, `Cargo.lock`
only, other workflows, unknown paths, an empty change, a crate outside the graph (synthetic workspace), forcing.
Saving: a pull request that only touches `packaging/`, `build-aux/flatpak.sh`, `release.yml` or similar skips about 85
runner-minutes of UI work (the Rust job, about 10 minutes, still runs: those paths are not text-only); docs/wiki/Markdown
pull requests already skipped Rust as well.

One Rust toolchain (`RUST_TOOLCHAIN` in `ci.yml`, installed with rustup) is used for fmt, clippy, tests, coverage and the
UI build. The coverage baseline is a number of that compiler's LLVM; with the distribution's newer rustc every crate read
1 to 3 points lower, and a new stable clippy would turn the checks red between two commits. Bump it on purpose, together
with the baseline (`build-aux/coverage.sh --update-baseline` raises it; lowering is done by hand with a reason).

### The pull request report

The last job of `ci.yml`, `report` (check name `Report`), runs after `required` whatever happened (`if: !cancelled()`) and
builds ONE Markdown report with `build-aux/ci-report.py`. It always goes to the job summary. For a pull request from this
repository it is also posted as ONE comment, found again by the marker `<!-- bookx-ci-report -->` and updated in place
on every push. Content: all jobs (result, time, link), the tests (nextest counts and failed test names, Broadway checks
per shard with the first failing check of each failed scenario, flaky scenarios that passed on the retry, Lua,
translations), the slowest steps of each job, the coverage per crate against the baseline with arrows, the logic-tier
total against 90%, patch coverage with the uncovered changed lines, the gate verdict, and links to the artifacts (HTML
coverage, lcov, JUnit).

It is informational: not a required check, `required` does not wait for it, and the comment step may fail without
failing CI (`continue-on-error`; the summary step cannot be skipped by it).

Security: no `pull_request_target`, no `workflow_run`. The job-level token has `pull-requests: write`, `contents: read`
and `actions: read`; for a pull request from a fork or from Dependabot GitHub makes it read-only, and the comment step is
skipped (`head.repo.full_name != github.repository` or actor `dependabot[bot]`): those get the summary only, as do pushes and
manual runs. The report is built in a step that has no token; only the next step, which runs `gh api` with the finished
Markdown file as data, gets it. The CI artifacts are data in any case: size-limited, parsed with the standard library, escaped
(no HTML, no links, no mentions), capped at 60 000 characters.

The builder is `build-aux/ci-report.py` (standard library), tested by `build-aux/tests/test_ci_report.py` against
fixtures in `build-aux/tests/fixtures/ci-report/`:

```bash
python3 build-aux/ci-report.py --jobs jobs.json --artifacts DIR --baseline build-aux/coverage-baseline.json --pr-comment
python3 -m unittest build-aux/tests/test_ci_report.py
```

## Caching, speed, parallelism

Hosted runners of a public repository have 4 vCPU and 16 GB; up to 20 jobs run at once (the matrix is 10 shards + 5 other
jobs, so everything starts together). Nothing paid is used. `build-aux/ci-timing.sh` prints the hardware at the start of the
heavy jobs; the PR comment's "Where the time goes" table comes from the step times of the jobs API.

| What | How |
|---|---|
| Cargo registry, git, `target` | `Swatinem/rust-cache` (pinned), key `rust` (clippy + the instrumented coverage build) and `ui` (the debug app), keyed on `Cargo.lock`, toolchain and job. **Only `main` saves** (a pull request restores main's cache and does not use the 10 GB quota); `cache-on-failure` so a red run still saves |
| `cargo-llvm-cov`, `cargo-nextest` | `taiki-e/install-action` downloads released binaries (seconds) |
| `diff-cover` | `~/.cache/pip` with `actions/cache` |
| Wiki | the Zensical virtualenv, keyed on `build-aux/wiki.sh` |
| Flatpak | `~/.local/share/flatpak` (GNOME runtimes) keyed on the manifest; `.flatpak-work/state` (downloads and build cache) keyed on manifest + `cargo-sources.json` + `Cargo.lock` |
| System packages | `dnf install` in the Fedora container takes 40 to 50 s; a prebuilt image would save less than the 2 minutes it would cost to maintain (a workflow, a registry, `packages: write`), so there is none |
| UI binaries | built once in `build-ui`, shared as the artifact `ui-bin` (a tarball of `bookx`, `bookx-cli` and the example programs); the 10 shards need no compiler: `SKIP_BUILD=1` |

Speed settings (env of `ci.yml`): `CARGO_PROFILE_DEV_DEBUG` and `CARGO_PROFILE_TEST_DEBUG` = `line-tables-only` (smaller
objects and binaries, backtraces still have lines), `RUSTFLAGS=-C link-arg=-fuse-ld=mold` (cargo-llvm-cov adds its own
flags to it), `CARGO_INCREMENTAL=0`, cargo and nextest use all 4 CPUs by default (nothing lowers them: `ci-timing.sh` prints it).
The UI build uses `CARGO_PROFILE_DEV_OPT_LEVEL=1`: three scenarios that time the app (PDF search, live annotations, big
books) failed on the 4-core runner with an unoptimised build.

UI shards: `build-aux/ui-shard.py` splits the scenarios greedily by the seconds each took
(`build-aux/ui-shard-weights.json`; a scenario without a weight counts as the median). Refresh the weights after adding
scenarios: download the `ui-times.txt` of the `report-ui-N` artifacts of a CI run and run
`python3 build-aux/ui-shard.py --update times1.txt times2.txt ...`. `SHARD=i/n` without the file falls back to a hash of the
name. `TIMES_FILE=file` makes `ui-tests.sh` write `scenario seconds`.

A scenario that fails in a shard is run up to two more times alone; if it passes the shard stays green, the job log has a warning and the
PR comment lists it as flaky. A scenario that fails three times is a failure (`koreader_annotations_live_reader` fails often on the runner: its first live state already shows the second push; it needs a fix in the scenario or the app). This is not a way to ignore a flake: the list in the
comment is the to-do list.

Not done, with the reason: nextest partitions across runners with a merged coverage profile (the instrumented build would
be repeated per partition and the profile merge is not exact: the test phase is about 4 minutes, the build 5);
sccache (rust-cache restores the whole `target` from main, which beats a per-crate cache for the warm case; for the cold
case the dependency build is the same either way).

### Timings (4 vCPU runners, wall time; measured on the pull request 2026-11-22)

| | Before (first run on `main`) | Cold cache | Warm cache (3 runs) |
|---|---|---|---|
| Whole CI | red; the UI job alone ran 60 min and failed 3514 of 3770 checks | 21 min 40 s | 13 min 30 s, 15 min |
| `quick` | 1 min (lint) + 1 min | 0 min 50 s | 1 min 20 s |
| `rust` (fmt, clippy, i18n, tests, coverage, checks) | 14 min, build failed | 21 min 20 s | 9 min 40 s to 12 min 30 s |
| `build-ui` | (built inside every UI job) | 12 min 20 s | 5 min 10 s to 6 min 30 s |
| `ui`, each of 10 shards | 60 min, one job | 7 min 30 s to 8 min 30 s | 7 min 30 s to 8 min 30 s |
| `wiki` | red | 8 min 50 s | 9 min to 10 min 40 s (picture optimisation, not cached) |
| `flatpak` | red (infrastructure) | 13 min 50 s | 11 min 20 s to 12 min 50 s |

Where the time goes (warm): `rust` = system packages 1 min, cache restore 0.5 min, clippy 1 min, reviewed-locales tests 1 min,
instrumented build + 5626 tests + gate 7 min (about 4 min of that is the tests: 5626 tests on 4 cores), mockups 1.2 min.
`build-ui` = packages 0.5 min, build 3.5 min, pack 20 s. A UI shard = packages 40 s, then 7 min of scenarios at 2 at a time
(they spend most of their time waiting for the GUI; a higher `JOBS` made timing scenarios flaky). The cold run is
dominated by compiling about 600 crates twice (instrumented and debug) plus once more for the UI build.
CPU-bound: compile, tests. Waiting: UI scenarios (sleeps), Flatpak (downloads and a release build of the app), wiki
(WebP conversion of 260 screenshots, single-threaded).

Remaining slowest steps and ideas: the instrumented build (`rust`, about 3 min warm) could be shared with `nextest archive`
partitions if the 4-minute test phase grows; the wiki picture conversion could be cached by file hash; `JOBS=3` per shard
with the weights file would take the UI wall time to about 6 minutes if the flaky scenarios named in the PR comment are fixed
first.

## Why the first runs were red (2026-11-22)

Nothing in the first runs was flaky; every red job had a cause.

| Check | Why it was red | What changed |
|---|---|---|
| Workflow lint | actionlint runs shellcheck on every `run:` block and failed on `ls` piped into `grep`, an unquoted `$tag`, reading and writing `SHA256SUMS` in one pipeline (`release.yml`, `i18n.yml`). | The scripts were rewritten (globs, quoting, a variable). |
| Format, lint, test | The runner's rust (Fedora, 1.100) has clippy lints that 1.93 does not: `unnecessary_sort_by`, `while_let_loop`, `manual_checked_div`, `collapsible_match`, `manual_as_chunks` and others. The code had only been linted with 1.93.1. | The code was fixed (no `allow`); CI now pins one toolchain so a new stable cannot break a merge. |
| Coverage | (1) three tests failed on the runner: `rar` tests used a `7z` that was found but cannot open RAR (7-Zip 26 builds without the RAR format); an HTTPS test connected only to the first address of `localhost`, which is `::1` in the container while the server listens on IPv4; the fake SMTP server of an attachment test rescanned its whole buffer on every read (quadratic), 15 s in a debug build, over the 10 s timeout on a loaded runner. (2) every crate read 1 to 3 points under the baseline: the baseline was measured with another compiler and with calibre, unrar, espeak-ng, chmlib and fonts installed. | `find_tool` asks `7z i` whether it knows RAR (real fix, with a test); `connect` tries every address (real fix; the existing test failed without it); the fake server scans only new bytes; the job pins the toolchain, installs the tools the tests use (calibre, chmlib, original `unrar` with a pinned sha256, espeak-ng, hunspell, fonts) and three baselines were set to what the CI environment measures (backup 90.19, serve 94.66, store 93.06). `calibredb` 9 prints "Upgrading database..." after its JSON, the test now reads the first JSON value (assertions unchanged). |
| UI scenarios | `zip` (and `xxd`) were not installed, so fixtures were not built; and `gtk4-broadwayd` puts its sockets in `$XDG_RUNTIME_DIR`, which does not exist in a root container (`/run/user/0`): GTK failed to initialise in 3514 of 3770 checks. After that: the container had no UTF-8 locale (grep compared curly quotes byte by byte), no espeak-ng, no fonts that match the ones the layout scenarios were written for (Adwaita Sans, Noto Serif, CJK/Arabic/Devanagari) and no hunspell dictionary. | `ui-tests.sh` checks `zip unzip python3` first and creates a runtime dir; the job sets `LANG=C.UTF-8` and installs the tools and fonts. Timing-sensitive scenarios failed on the unoptimised build: the UI build is `opt-level=1`, JOBS=2 per shard and a failed scenario gets one more run. |
| Translations | `actions/checkout` ran before git was installed in the container, so the checkout was not a repository and `git diff po/bookx.pot` printed its usage; `po/POTFILES.in` was out of date. | git is installed first; `POTFILES.in` regenerated. |
| Wiki | `check-wiki-frames.py` and `check-wiki-centred.py` import Pillow but were run with the system Python; Pillow lives in the wiki virtualenv. | They run with the virtualenv's Python. |
| Flatpak | `flatpak-builder` inside the `flatpak-github-actions` container died with "Invalid cross-device link" while updating the GNOME SDK (ostree hard links across the container's overlay mounts), then `xvfb-run` crashed (exit 139). | The bundle is built on the runner itself (apt `flatpak`, `flatpak-builder`), with `build-aux/flatpak.sh`, which also runs the smoke tests. |
| Wiki (once, later) | The colour-scheme check starts headless Firefox and waited 20 s for it; one run on a loaded runner needed longer (`could not start Firefox`). | It waits up to 60 s. |
| `release-check` | It asserted that every job has `runs-on`; the reusable-workflow job `flatpak` has `uses` instead. | It accepts either. |
| Dependabot pull requests | The same failures (they ran the same broken CI), plus code changes in the cargo bumps (see the pull requests). Their token is read-only and has no secrets; no job needs either now (the report job only comments on pull requests of this repository, see above). | Rebase onto the fixed `main`. |

## Code coverage

### Tools (nothing is home-made except the table)

| What | Tool |
|---|---|
| Measuring | `cargo-llvm-cov` 0.8+ (LLVM source-based coverage, `-C instrument-coverage`; the Rust counterpart of pytest-cov) |
| Running the tests | `cargo-nextest` (one process per test, all cores, no doc-tests; there are none) |
| Patch coverage (lines changed in a PR) | `diff-cover` (pip) on the filtered lcov file |
| Table, baseline, gate | `build-aux/coverage-gate.py` (reads the lcov file: grouping, comparison, exit code; writes `gate.json`, `summary.md`) |

Install once: `cargo install cargo-llvm-cov cargo-nextest --locked`, `rustup component add llvm-tools-preview`.

### Commands

```bash
build-aux/coverage.sh                    # the gate: whole workspace
build-aux/coverage.sh --quick            # GTK-free crates only (does not build the app crate)
build-aux/coverage.sh --html             # HTML report: target/llvm-cov/html/index.html
build-aux/coverage.sh --update-baseline  # after improving coverage: raise build-aux/coverage-baseline.json
build-aux/coverage.sh --base origin/main # also: lines changed since that ref must be >= 80% covered
build-aux/coverage.sh --no-run           # report again from the last run
build-aux/coverage-ui.sh                 # UI tier through the Broadway scenarios (slow; nightly.yml)
```

Outputs are in `$CARGO_TARGET_DIR/llvm-cov/`: `lcov.info`, `lcov.filtered.info`, `summary.json`, `summary.md`, `gate.json`,
`junit.xml` (nextest), `patch.md` and `patch.json` (diff-cover), `html/`.

### What is measured, and what is left out

The number is **line coverage of application code**. Left out, because counting it would inflate the figure:

* inline `#[cfg(test)]` modules, `tests/`, `examples/`, `benches/`, `fuzz/`, and test support compiled into libraries
  (`testutil`, `testgen`, `test_support`, `tests_extra`);
* in the app crate `bookx`, the GTK glue: `src/ui/**`, `src/app.rs`, `src/main.rs`, `src/*_ui.rs`, `src/devtools.rs`,
  `src/kobo_real_check.rs` and `src/voice_download.rs`. They form the **UI tier**, exercised by the Broadway scenarios.

Groups: every `crates/bookx-*` crate, `bookx-logic` (the GTK-free modules of `src/`), `bookx-ui` (the glue above).

### The policy

**Logic tier (per PR)**

1. The whole logic tier must be at or above 90% (`target` in the baseline file).
2. A ratchet per group: no crate may fall more than 0.2 points below its number in `build-aux/coverage-baseline.json`.
   The baseline only goes up (`--update-baseline` keeps the higher value); lower it by hand only with a reason in the
   change description. The environment of record is the CI job (toolchain, installed tools): a number from a machine
   with other tools is not comparable.
3. A new crate must start at 90% or more.
4. Patch coverage: lines a PR adds or changes in logic code must be covered at least 80% (`diff-cover`).
5. Any failing test fails the job, and a broken run is not trusted for coverage.

**UI tier (nightly or on demand)**: the app is built with instrumentation, every scenario process writes its own
`.profraw` when it quits, the profiles are merged. Ratcheted in the same baseline file (`bookx-ui`). Not a per-PR check.

Below the 90% goal today: bookx-audio (real audio device code), bookx-cli (device and backup commands), bookx-secrets
(needs a keyring daemon), bookx-tts (ONNX engines need model files), bookx-logic. Each should reach 90% with fakes.

### Known limits

* Line coverage counts a line as covered when it ran, not that anything was asserted about it. Review tests too.
* Region and function coverage are in `summary.json` and the HTML report, not gated.
* Inline test modules are found by their `#[cfg(test)]` attribute at column 0 up to the closing `}` at column 0.
* `NET=0` skips the loopback server tests, so the gate runs without it; the offline figure is a little lower.

## Scenarios that measure a speed

A scenario that checks "drawn within 1.5 seconds" (`koreader_annotations_live_reader`) waits for the app's own signal
(`until:sync-repaint>=N`, counters from `devtools::bump`) and then checks the measured time (`until_ms:` line) against
1500 ms times `UI_LATENCY_SLACK` (1 on a developer machine, 3 when `CI=true`, set in `build-aux/ui-tests.sh`). The state
is never read after a fixed sleep: that read the page before the app had finished on a slow or busy machine and failed
every time there (found when the scenario failed on a developer machine and flaked in CI). Do not use `wait:` where a
`until:` counter exists; add a `devtools::bump` where the app does the thing you wait for.
