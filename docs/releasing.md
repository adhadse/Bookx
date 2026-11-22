# Releasing Bookx

For maintainers. This is the exact procedure for a release: which version string, which tag, which GitHub Action runs
in which order, what ends up on the release page and how to check it. Users read `wiki/docs/install.md` instead.

Policy first: **GitHub Releases is the only official channel.** Flathub, Homebrew, winget, Scoop and MSYS2 come later
(`docs/release-plan.md`, `docs/flathub-submission.md`). macOS and Windows are scaffolds and are not part of a release.

## What a release contains

| File | Built by | Notes |
|---|---|---|
| `bookx-<version>-linux-x86_64.tar.gz` (+ `.sha256`) | `release.yml` job `tarball` (`build-aux/tarball.sh`) | deterministic packing |
| `bookx_*_amd64.deb` | `tarball` (`cargo deb`) | Debian, Ubuntu. cargo-deb rewrites `0.3.0-beta.1` as `0.3.0~beta.1` so the final release sorts higher; check the real name on the first beta |
| `bookx-*.x86_64.rpm` | `tarball` (`cargo generate-rpm`) | Fedora, openSUSE. Pre-release version handling in RPM is untested: confirm on the first beta (`rpm -qip`) |
| `Bookx-<version>.flatpak` | `release.yml` job `flatpak` (reusable `_flatpak.yml`) | single-file bundle, GNOME runtime from Flathub |
| `SHA256SUMS` | job `publish` | covers every file above |
| `<file>.sigstore.json` | job `publish` | cosign keyless signature per file |
| build provenance attestation | job `publish` | verify with `gh attestation verify` |

**There is no AppImage.** Nothing in the repository builds one and we do not promise one: the Flatpak bundle is the
portable, sandboxed single file. An AppImage would be a new packaging task (GTK 4 + libadwaita bundling, GSettings
schemas, pixbuf loaders), tracked in `docs/enhancements.md` if wanted. Do not mention AppImage in release notes.

## Tags and versions

- One tag per release, **annotated**, named `v` + the version in `Cargo.toml`: `v0.3.0-beta.1`, `v0.3.0-beta.2`,
  `v0.3.0-rc.1`, `v0.3.0`.
- A tag with `-alpha`, `-beta` or `-rc` in it becomes a **GitHub pre-release** automatically (job `version` in
  `release.yml`). A plain `vX.Y.Z` is a normal release and becomes "Latest".
- The tag must match the version in `Cargo.toml` exactly. `build-aux/release-check.sh vX.Y.Z` enforces it, and the
  workflow runs it, so a wrong tag fails before anything is built.
- Never move or delete a pushed tag that has a published release. Fix forward with the next number
  (`-beta.2`). Signatures and attestations name the tag, so a moved tag breaks verification for everybody.
- The version appears in: every `crates/*/Cargo.toml` and the workspace `Cargo.toml`, `Cargo.lock`,
  `data/com.adhadse.Bookx.metainfo.xml` (`<release version="...">` with notes: the About dialog reads them),
  `CHANGELOG.md`, and `build-aux/cargo-sources.json` (regenerated from `Cargo.lock`). `release-check.sh` fails if
  they disagree.

## Workflows, and what runs when

| Workflow | Trigger | What it does | Publishes? |
|---|---|---|---|
| `ci.yml` | every push and pull request | quick checks, Rust (fmt, clippy, i18n, tests, coverage gate), UI shards, wiki, Flatpak (only when packaging changed), final `required` check | no |
| `_flatpak.yml` | called by `ci.yml`, `release.yml`, `nightly.yml` | builds `Bookx-<version>.flatpak` once, smoke-tests it | no |
| `nightly.yml` | schedule, manual | UI coverage tier, cold-cache Flatpak build | no |
| `release.yml` | **push of a `v*` tag**, or manual (`skip_gates`) | the release (below) | **yes** |
| `package-native.yml` | manual only | macOS `.app/.dmg` or Windows zip; scaffold, never part of a release | no (artifact only) |
| `flathub-update.yml` | deferred | opens the Flathub update PR after Flathub exists | no |

You never start the build jobs by hand: **pushing the tag is the one action that starts a release.**

### Order inside `release.yml`

```
tag v0.3.0-beta.1 pushed
   │
   ├── gates      the `required` check of ci.yml must be green on the tagged commit, else stop
   │      │
   │      ├── tarball   release-check.sh <tag> → cargo build --release → tarball.sh → .deb → .rpm
   │      └── version   derives "0.3.0-beta.1" and prerelease=true from the tag
   │             └── flatpak   _flatpak.yml → Bookx-0.3.0-beta.1.flatpak
   │
   └── publish    (after tarball + flatpak) SHA256SUMS → attestation → cosign sign + verify
                  → GitHub release, generated notes, flagged pre-release when the tag says so
```

`gates` is why a tag on a red commit does nothing. Emergency only: run the workflow manually with `skip_gates`.

## Procedure

Do the steps in this order. Everything before step 7 is reversible; the tag is not.

1. **Start from a green `main`.** The `required` check of `ci.yml` is green on the commit you will tag (check the PR
   or the commit's checks page).
2. **Bump the version** everywhere listed above. Regenerate the Flatpak crate list and the lock file:
   `cargo update -w` (workspace only), then `python3 build-aux/flatpak-cargo-sources.py`.
3. **Write the notes:** a `<release version="X" date="YYYY-MM-DD">` block in the metainfo (user-visible changes in
   plain words, the beta limits) and a `CHANGELOG.md` entry.
4. **Pin the screenshots to the tag:** `build-aux/pin-screenshots.sh v0.3.0-beta.1`. If the UI changed, retake them
   first (`build-aux/wiki-screenshots.sh`).
5. **Run the local gate:** `build-aux/release-check.sh v0.3.0-beta.1` (or `make release-check`), then the usual
   `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `build-aux/lint-checks.sh`, `build-aux/ui-tests.sh`, `build-aux/wiki.sh` (AGENTS.md §4).
6. **Commit and push to `main` through a PR** (or direct push while there is no branch protection). Wait for CI to be
   green on the final commit. This is the commit that gets tagged.
7. **Tag and push** (the only irreversible step; ask the owner first):
   ```bash
   git switch main && git pull --ff-only
   git tag -a v0.3.0-beta.1 -m "Bookx 0.3.0-beta.1"
   git push origin v0.3.0-beta.1
   ```
8. **Watch `release.yml`** (Actions → Release): `gates` → `tarball` and `flatpak` → `publish`. Expect roughly
   30 to 60 minutes cold. If a job fails, fix forward: do not move the tag; make a new commit and tag
   `-beta.2`. A release that failed before `publish` created nothing public, so you may delete that tag and push it
   again only if no release page exists for it.
9. **Verify the artifacts** from the release page, on a machine that is not the build machine:
   ```bash
   sha256sum -c SHA256SUMS
   gh attestation verify Bookx-0.3.0-beta.1.flatpak --repo adhadse/Bookx
   cosign verify-blob --bundle Bookx-0.3.0-beta.1.flatpak.sigstore.json \
     --certificate-identity-regexp '^https://github\.com/adhadse/Bookx/\.github/workflows/release\.yml@refs/tags/v' \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com Bookx-0.3.0-beta.1.flatpak
   flatpak install --user ./Bookx-0.3.0-beta.1.flatpak      # starts, About says "Beta"
   sudo apt install ./bookx_*_amd64.deb                      # or: sudo dnf install ./bookx-*.rpm
   ```
10. **Edit the release page:** replace the generated notes with the plain-words notes, add the beta warning and the
    "Report a Problem" steps, keep it marked **pre-release** for any `-alpha`/`-beta`/`-rc`.
11. **Announce** only after step 9 passed.

## After the release

- Open a new `[Unreleased]` section in `CHANGELOG.md`; bump `main` to the next pre-release version when the next work
  starts, not before.
- Triage reports (GitHub issue forms; the in-app Report a Problem button fills them).
- A stable `vX.Y.Z` follows the exit criteria in `docs/release-plan.md` (enough beta releases, downloads and no open
  critical bugs). `-rc.N` is the last step before it.

## Run the pieces locally

```bash
build-aux/release-check.sh v0.3.0-beta.1        # versions, changelog, metainfo, Flatpak sources, plugin golden files
cargo build --release --locked -p bookx -p bookx-cli && build-aux/tarball.sh dist
cargo deb --no-build -p bookx --output dist/ && cargo generate-rpm -p . -o dist/
build-aux/flatpak.sh                              # Flatpak build with smoke tests
```

## If something goes wrong

| Symptom | Cause and fix |
|---|---|
| `gates` fails with "CI has not passed" | the tagged commit has no green `required` check: wait for CI or tag a green commit |
| `release-check` fails on the tag | tag and `Cargo.toml` version differ, or a crate/metainfo/changelog was not bumped |
| `flatpak` fails on crates | `build-aux/cargo-sources.json` is stale: regenerate it after any `Cargo.lock` change |
| `publish` fails at cosign verify | the workflow file was renamed: the identity regex in `release.yml` and `wiki/docs/install.md` must match |
| the release is not marked pre-release | the tag has no `-alpha`/`-beta`/`-rc` part: edit the release page by hand, then fix the tag name rule |
