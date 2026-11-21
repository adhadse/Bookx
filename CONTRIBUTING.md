# Contributing to Bookx

Thank you for helping. Bookx is a GNOME e-book library, reader and editor written in Rust (GPL-3.0-or-later).

- **Guide for contributors** (build, test, wiki, translations, plugins): <https://bookx.adhadse.com/contributing/>
- **Translations**: <https://bookx.adhadse.com/translating/> (Weblate, or a pull request on `po/<language>.po`).
- **Plugins and patches**: <https://bookx.adhadse.com/plugin-development/>.
- **Security problems**: do *not* open a public issue; follow [SECURITY.md](SECURITY.md).

## Before you open an issue

Search the [existing issues](https://github.com/adhadse/Bookx/issues). For a bug, use the *Bug report* form and paste the
output of **Main menu → About Bookx → Troubleshooting → Copy Diagnostics** (it contains no book titles, accounts or
passwords; read it once before posting). For an idea, use *Feature request* and describe what you are trying to do.

## Before you open a pull request

The rules every change follows are in [AGENTS.md](AGENTS.md) (they apply to people and AI assistants alike). In short:

1. **Every change ships with tests** (a bug fix has a regression test that fails without the fix; UI behaviour gets a
   scenario in `build-aux/ui-tests.sh`).
2. Tests run in parallel and touch no shared files, ports or global state.
3. A user-visible change updates the matching page in `wiki/docs/`; internal notes go in `docs/`.
4. Run, and keep green:

   ```bash
   cargo fmt
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   build-aux/lint-checks.sh
   build-aux/coverage.sh --quick      # line coverage per crate must not fall below build-aux/coverage-baseline.json
   build-aux/ui-tests.sh              # needs gtk4-broadwayd; skips itself when it is missing
   ```

5. New third-party assets must allow redistribution and be listed in [NOTICE.md](NOTICE.md). New dependencies must pass
   `cargo deny check`.
6. If you used an AI assistant, say so in the pull request description. You are responsible for what you submit.

By contributing you agree that your work is licensed under GPL-3.0-or-later, like the rest of the project.

## Conduct

Be kind and assume good faith; see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
