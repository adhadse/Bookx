# Contributing

Issues and pull requests are welcome on [GitHub](https://github.com/adhadse/Bookx). If you are new to GNOME app
design, read the [GNOME HIG](https://developer.gnome.org/hig/) and look at the mockups before changing the UI.

## Build and test

See [Install](install.md) for the build dependencies, then:

```bash
cargo run --release
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
build-aux/ui-tests.sh          # end-to-end scenarios; needs gtk4-broadwayd, skips itself when it is missing
```

All four checks must pass before a change is done. Then `./run.sh --no-run` rebuilds and reinstalls it for you.

## Rules of the repository

The full rules are in `AGENTS.md` in the repository. In short:

- **Every change ships with tests.** Put decisions in pure functions and unit-test them. UI behaviour gets a
  scenario in `build-aux/ui-tests.sh`. A bug fix needs a regression test that fails without the fix.
- **Tests run in parallel.** No shared files, ports or global state. UI scenarios each get their own Broadway display,
  D-Bus session, config, cache and library. Tests that use the network must be skippable with `NET=0`.
- **Workspace layout.** The engine (`bookx-core`) never depends on GTK. Other crates (`bookx-store`, `bookx-formats`,
  `bookx-convert`, `bookx-text`, `bookx-lookup`, `bookx-serve`, `bookx-sources`, `bookx-editing`, `bookx-plugin`,
  `bookx-cli`) meet at the traits in `crates/bookx-core/src/contracts.rs`.
- Rust only for application code. New third-party assets must allow redistribution and be listed in `NOTICE.md`.
- No debug output left in committed code.

## Documentation

There are two kinds, kept apart:

- **This wiki** (`wiki/`) is for users and plugin authors. A user-visible change (feature, menu entry, shortcut,
  setting, CLI option, file or plugin format, device behaviour) updates or adds the matching page in the same change.
- **Internal notes** (`docs/`) hold the roadmap, bug list, research, caveats and performance numbers. They do not go
  on the wiki.

### Adding or editing a wiki page

1. Pages are Markdown files in `wiki/docs/`. Add a new file there and list it in the `nav` of `wiki/zensical.toml`.
2. Write for users: how to do it, what to expect, and a **Limitations** note for what is missing or untested.
3. Put pictures in `wiki/docs/assets/`. Every screenshot comes in two variants, `name-light.png` and `name-dark.png`, and a
   page shows both so that the one matching the site's colour scheme is visible:

    ```markdown
    ![The library](assets/library-details-light.png#only-light)
    ![The library](assets/library-details-dark.png#only-dark)
    ```

    `build-aux/wiki-screenshots.sh` takes all of them in a sandboxed GNOME Shell session (nothing of yours is touched) in
    both styles and adds the window shadow; `build-aux/wiki.sh` fails when a variant is missing. To add a picture, add
    a step to a scenario in `build-aux/session/scenarios_wiki.py`.
4. Build and check:

    ```bash
    build-aux/wiki.sh
    ```

    This installs [Zensical](https://zensical.org/) into `~/.cache/bookx-wiki-venv`, builds the site, and fails on
    build errors and on broken internal links, anchors and images. Open `wiki/site/index.html` to look at the result.
    To preview with live reload: `~/.cache/bookx-wiki-venv/bin/zensical serve -f wiki/zensical.toml`.

5. Every page has an *Edit this page* link that opens the file on GitHub.

The site is deployed to [bookx.adhadse.com](https://bookx.adhadse.com) by GitHub Actions when changes under `wiki/`
reach the `main` branch.

## Licence

Bookx is licensed under the GPL-3.0-or-later.
