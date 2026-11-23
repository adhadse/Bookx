# Releasing Bookx

For maintainers. This is the exact procedure for a release: which version string, which tag, which GitHub Action runs
in which order, what ends up on the release page and how to check it. Users read `wiki/docs/install.md` instead.

Status: living | Last reviewed: 2026-11-23 | Owner: @adhadse

## Contents

- [What a release contains](#what-a-release-contains)
- [Tags and versions](#tags-and-versions)
- [Workflows, and what runs when](#workflows-and-what-runs-when)
- [Procedure](#procedure)
- [After the release](#after-the-release)
- [Run the pieces locally](#run-the-pieces-locally)
- [If something goes wrong](#if-something-goes-wrong)

Policy first: **GitHub Releases is the only official channel.** Flathub, Homebrew, winget, Scoop and MSYS2 come later
(`docs/release-plan.md`, `docs/flathub-submission.md`). macOS and Windows are scaffolds and are not part of a release.

## What a release contains

| File | Built by | Notes |
|---|---|---|
| `bookx-<version>-linux-x86_64.tar.gz` | `release.yml` job `tarball` (`build-aux/tarball.sh`) | deterministic packing; unpacks to a folder of the same name |
| `bookx_<version>_amd64.deb` | `tarball` (`cargo deb`) | Debian, Ubuntu. Inside, the package version is `0.3.0~beta.1` (cargo-deb rewrites `-` so the final release sorts higher): check on the first beta |
| `bookx-<version>.x86_64.rpm` | `tarball` (`cargo generate-rpm`) | Fedora, openSUSE. The release job passes `0.3.0~beta.1` to cargo-generate-rpm (a hyphen is invalid in an RPM version; found on the first tag run): check with `rpm -qip` |
| `bookx-<version>-x86_64.flatpak` | `flatpak` (reusable `_flatpak.yml`) | single-file bundle, GNOME 51 runtime from Flathub |
| `SHA256SUMS` | job `publish` | covers every file above |
| `<file>.sigstore.json` | job `publish` | cosign keyless signature per file |
| build provenance attestation | job `publish` | verify with `gh attestation verify` |

### Supported distributions (and how it is proved)

The packages are built on **Fedora 43** (glibc 2.42, GTK 4.20, libadwaita 1.8, the minimum Bookx needs): a binary runs on
its build machine's glibc or newer, never older, so the build image is the floor. Never build the release on
`fedora:latest`: the first beta did, needed glibc 2.44 and OpenSSL 4 and ran on Fedora 45 only. The `.rpm` has
`auto-req = "no"` (otherwise it requires the exact sonames of the build machine) and the `.deb` requires `libc6 (>= 2.42)`.
The `distros` job of `release.yml` runs `build-aux/distro-matrix.sh dist`: it installs the package of each supported
image (Fedora 43/44/45, Ubuntu 25.10/26.04, Debian testing, Arch) in a container, runs `bookx-cli --version` and starts
`bookx` on Broadway until it logs `library opened`. `publish` needs it, so a package that does not run blocks the release.
Run it yourself: `build-aux/distro-matrix.sh DIR` (podman or docker; `ONLY="fedora:43"` for one image).
Not supported by the native packages (GTK too old): Debian 12/13, Ubuntu 24.04, Fedora 42: they use the Flatpak. When a
new distribution release comes out, add it to the list in the script and to the install page.

### Naming scheme

Each file is named the way its ecosystem names its own (ripgrep, bat and KOReader do the same), always with the version
exactly as in the tag without the `v` (`0.3.0-beta.2`):

| Kind | Name |
|---|---|
| tarball | `bookx-<version>-linux-<arch>.tar.gz` |
| Debian/Ubuntu | `bookx_<version>_<amd64\|arm64>.deb` |
| Fedora/openSUSE | `bookx-<version>.<x86_64\|aarch64>.rpm` |
| Flatpak bundle | `bookx-<version>-<arch>.flatpak` |

The tools write their own names (`bookx_0.3.0~beta.1-1_amd64.deb`); `build-aux/release-names.sh` renames them in the
`publish` job before checksums and signatures are made, fails if a kind is missing or doubled, and has a test
(`build-aux/tests/test_release_names.py`). **No `~` in a file name:** GitHub rewrites it to `.` on upload, so the name in
`SHA256SUMS` would not match the download. The packages inside still carry `0.3.0~beta.1` (so the final 0.3.0 sorts
higher). A future macOS or Windows file would be `bookx-<version>-<arch>.dmg` / `bookx-<version>-windows-x64.zip`.

**There is no AppImage.** Nothing in the repository builds one and we do not promise one: the Flatpak bundle is the
portable, sandboxed single file. An AppImage would be a new packaging task (GTK 4 + libadwaita bundling, GSettings
schemas, pixbuf loaders), tracked in `docs/enhancements.md` if wanted. Do not mention AppImage in release notes.

## Tags and versions

- One tag per release, **annotated**, named `v` + the version in `Cargo.toml`: `v0.3.0-beta.2`, `v0.3.0-beta.2`,
  `v0.3.0-rc.1`, `v0.3.0`.
- Every tag, also one with `-alpha`, `-beta` or `-rc`, is published as a normal release and marked **Latest** (owner decision
  2026-11-23): a GitHub pre-release does not appear in the repository's Releases box. The beta status is in the name, the
  notes and the app (About), not in GitHub's flag.
- The tag must match the version in `Cargo.toml` exactly. `build-aux/release-check.sh vX.Y.Z` enforces it, and the
  workflow runs it, so a wrong tag fails before anything is built.
- Never move or delete a pushed tag that has a published release. Fix forward with the next number
  (`-beta.2`). Signatures and attestations name the tag, so a moved tag breaks verification for everybody.
- The version appears in: every `crates/*/Cargo.toml` and the workspace `Cargo.toml`, `Cargo.lock`,
  `data/com.adhadse.Bookx.metainfo.xml` (`<release version="...">` with notes: the About dialog reads them),
  `CHANGELOG.md`, `build-aux/cargo-sources.json` (regenerated from `Cargo.lock`) and `_pkgver` in
  `packaging/aur/*/PKGBUILD` (checksums and `.SRCINFO` after the release: `packaging/aur/README.md`). `release-check.sh`
  fails if they disagree.

## Workflows, and what runs when

| Workflow | Trigger | What it does | Publishes? |
|---|---|---|---|
| `ci.yml` | pull requests (a push to `main` only warms the caches), manual, and every night through `nightly.yml` | quick checks, Rust (fmt, clippy, i18n, tests, coverage gate), UI shards, wiki build and check (never a deploy), Flatpak (only when packaging changed), final `required` check | no |
| `_flatpak.yml` | called by `ci.yml`, `release.yml`, `nightly.yml` | builds the bundle once (`Bookx-<version>.flatpak`, renamed by `publish`), smoke-tests it | no |
| `wiki-deploy.yml` | called by `release.yml` after `publish`, or manual (`ref`) | builds the wiki of one ref and publishes it to GitHub Pages (https://bookx.adhadse.com) | **the live docs** |
| `nightly.yml` | schedule, manual | UI coverage tier, cold-cache Flatpak build | no |
| `release.yml` | **push of a `v*` tag**, or manual (`skip_gates`) | the release (below) | **yes** |
| `package-native.yml` | manual only | macOS `.app/.dmg` or Windows zip; scaffold, never part of a release | no (artifact only) |
| `flathub-update.yml` | deferred | opens the Flathub update PR after Flathub exists | no |

You never start the build jobs by hand: **pushing the tag is the one action that starts a release.**

### Order inside `release.yml`

```
tag v0.3.0-beta.2 pushed
   │
   ├── gates      the tagged commit passed CI: its `required` check, a green Nightly on it, or the merged pull request
   │              with a green `required` and an identical tree (build-aux/release-gate.py), else stop
   │      │
   │      ├── tarball   release-check.sh <tag> → cargo build --release → tarball.sh → .deb → .rpm
   │      └── version   derives "0.3.0-beta.2" from the tag
   │             └── flatpak   _flatpak.yml → bookx-0.3.0-beta.2-x86_64.flatpak
   │
   └── publish    (after tarball + flatpak) SHA256SUMS → attestation → cosign sign + verify
          │       → GitHub release, generated notes, marked Latest
          └── wiki      (after publish, tags only) wiki-deploy.yml builds the TAGGED commit and publishes it
```

The docs go live after the release assets are out, also for betas (beta users need them). A Pages failure turns
`wiki` red but changes nothing about the release. `gates` is why a tag on a red commit does nothing. Emergency only: run the workflow manually with `skip_gates`.

## Procedure

Do the steps in this order. Everything before step 7 is reversible; the tag is not.

1. **Start from a green `main`.** Pushes to `main` do not run the full CI (the pull request did, on an up-to-date branch).
   The commit you tag is the squash merge of a pull request whose `required` check is green, and the last Nightly run
   (Actions, Nightly) is green. `release.yml` `gates` checks the first for you (below); if a Nightly is red, fix that first.
   Dry run: Actions, Release, Run workflow, `gates_only`.
2. **Bump the version** everywhere listed above. Regenerate the Flatpak crate list and the lock file:
   `cargo update -w` (workspace only), then `python3 build-aux/flatpak-cargo-sources.py`.
3. **Write the notes:** a `<release version="X" date="YYYY-MM-DD">` block in the metainfo (user-visible changes in
   plain words, the beta limits) and a `CHANGELOG.md` entry.
4. **Screenshots:** retake them first if the UI changed (`build-aux/wiki-screenshots.sh`). Do **not** run
   `build-aux/pin-screenshots.sh` before the tag exists: the Flatpak job validates the metainfo screenshot URLs and
   they 404 until the tag is pushed, so CI goes red (found on the first beta). Pin only for a Flathub submission, as a
   follow-up commit after the tag.
5. **Run the local gate:** `build-aux/release-check.sh v0.3.0-beta.2` (or `make release-check`), then the usual
   `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
   `build-aux/lint-checks.sh`, `build-aux/ui-tests.sh`, `build-aux/wiki.sh` (AGENTS.md §4).
6. **Commit and push to `main` through a PR** (or direct push while there is no branch protection). Wait for CI to be
   green on the final commit. This is the commit that gets tagged.
7. **Tag and push** (the only irreversible step; ask the owner first):
   ```bash
   git switch main && git pull --ff-only
   git tag -a v0.3.0-beta.2 -m "Bookx 0.3.0-beta.2"
   git push origin v0.3.0-beta.2
   ```
8. **Watch `release.yml`** (Actions → Release): `gates` → `tarball` and `flatpak` → `publish`. Expect roughly
   30 to 60 minutes cold. If a job fails, fix forward: do not move the tag; make a new commit and tag
   `-beta.2`. A release that failed before `publish` created nothing public, so you may delete that tag and push it
   again only if no release page exists for it.
9. **Verify the artifacts** from the release page, on a machine that is not the build machine:
   ```bash
   sha256sum -c SHA256SUMS
   gh attestation verify bookx-0.3.0-beta.2-x86_64.flatpak --repo adhadse/Bookx
   cosign verify-blob --bundle bookx-0.3.0-beta.2-x86_64.flatpak.sigstore.json \
     --certificate-identity-regexp '^https://github\.com/adhadse/Bookx/\.github/workflows/release\.yml@refs/tags/v' \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com bookx-0.3.0-beta.2-x86_64.flatpak
   flatpak install --user ./bookx-0.3.0-beta.2-x86_64.flatpak      # starts, About says "Beta"
   sudo apt install ./bookx_*_amd64.deb                      # or: sudo dnf install ./bookx-*.x86_64.rpm
   ```
10. **Edit the release page:** replace the generated notes with the plain-words notes, add the beta warning and the
    "Report a Problem" steps, keep it marked **Latest** (not pre-release).
11. **Announce** only after step 9 passed.

## After the release

- **The wiki is already live** (the `wiki` job built it from the tag). Pushes to `main` never deploy it: the site only
  describes what a download contains. Documentation fixes between releases reach the site by a manual **Wiki deploy** run
  (Actions, Wiki deploy, Run workflow): leave `ref` empty to publish the latest stable release tag (use it only when the tagged
  content is right and only the deploy was missed), or type a tag. A docs hotfix needs a new tag; publishing `main` by hand
  (`ref` = `main`) puts unreleased features on the public site, so do it only knowingly.

- Open a new `[Unreleased]` section in `CHANGELOG.md`; bump `main` to the next pre-release version when the next work
  starts, not before.
- Triage reports (GitHub issue forms; the in-app Report a Problem button fills them).
- A stable `vX.Y.Z` follows the exit criteria in `docs/release-plan.md` (enough beta releases, downloads and no open
  critical bugs). `-rc.N` is the last step before it.

## Run the pieces locally

```bash
build-aux/release-check.sh v0.3.0-beta.2        # versions, changelog, metainfo, Flatpak sources, plugin golden files
cargo build --release --locked -p bookx -p bookx-cli && build-aux/tarball.sh dist
cargo deb --no-build -p bookx --output dist/ && cargo generate-rpm -p . -o dist/
build-aux/flatpak.sh                              # Flatpak build with smoke tests
```

## If something goes wrong

| Symptom | Cause and fix |
|---|---|
| `gates` is refused | the message says which: no pull request (a direct push), the pull request's tree differs from the tagged tree (it was not up to date when merged), or its `required` check is not green. Tag a squash-merge commit of a green, up-to-date pull request, wait for a green Nightly on the commit, or use `skip_gates` in an emergency |
| `release-check` fails on the tag | tag and `Cargo.toml` version differ, or a crate/metainfo/changelog was not bumped |
| `flatpak` fails on crates | `build-aux/cargo-sources.json` is stale: regenerate it after any `Cargo.lock` change |
| `publish` fails at cosign verify | the workflow file was renamed: the identity regex in `release.yml` and `wiki/docs/install.md` must match |
| the release is not marked Latest | a newer release exists, or the flag was changed by hand: `gh release edit <tag> --latest --prerelease=false` |
