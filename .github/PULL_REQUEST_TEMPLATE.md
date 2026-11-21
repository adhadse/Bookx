## What and why

<!-- One or two sentences. Link the issue: "Fixes #123". -->

## Checklist

- [ ] Tests added or updated in this change (a bug fix has a regression test that fails without the fix)
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] `build-aux/lint-checks.sh` and, for UI changes, a scenario in `build-aux/ui-tests.sh` (`build-aux/ui-tests.sh`)
- [ ] **Coverage gate**: `build-aux/coverage.sh --quick` does not fall below `build-aux/coverage-baseline.json` (CI check: *Coverage (cargo-llvm-cov, nextest)*)
- [ ] User-visible change: the matching page in `wiki/docs/` is updated (`build-aux/wiki.sh` passes)
- [ ] Strings changed: `build-aux/i18n.sh` run, `po/bookx.pot` refreshed
- [ ] New asset or dependency: listed in `NOTICE.md`, `cargo deny check` passes
- [ ] Touches untrusted input (files, network, paths, processes, secrets): the checklist in `docs/security.md` §8 is answered below
- [ ] AI assistance, if any, is disclosed in this description

## Notes for the reviewer

<!-- Screenshots for UI changes, compatibility, anything surprising. -->
