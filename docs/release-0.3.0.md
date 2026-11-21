# Release 0.3.0: runbook for the owner

Internal (not for the wiki). Copy-paste top to bottom; tick the boxes. Everything an agent could do is already done
and listed in section 0; the steps below are the ones that need you (a tag, accounts, a decision, a GitHub setting).
Nothing here has ever run on GitHub: the first tag is also the first run of `release.yml`, so read section 6 first.

## 0. State when this was written (2026-11-22)

- Version `0.3.0` in the workspace (`Cargo.toml` and the 18 crates, `Cargo.lock`), `CHANGELOG.md` has `[0.3.0] - 2026-11-22`
  and an empty `[Unreleased]`, the metainfo has `<release version="0.3.0" date="2026-11-22">` with user-facing notes.
  The plugin version (KOReader plugin 0.3.1) is separate and unchanged.
- `build-aux/release-check.sh` passes, `cargo deny check` passes, `cargo audit` has no vulnerability (7 "unmaintained"
  notices, all documented in `deny.toml`). Results and numbers: `docs/remaining-work.md` (Release 0.3.0 readiness) and
  `docs/flathub-submission.md` §5.
- Metainfo screenshots still point at `main`; step 3 pins them to the tag.

## 1. Pre-flight gates (on the release machine, clean checkout of `main`)

```bash
git switch main && git pull --ff-only && git status --short      # must print nothing
```

- [ ] Format, lint, tests
  ```bash
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
- [ ] Scenario lint, coverage (line coverage per crate must not fall below `build-aux/coverage-baseline.json`)
  ```bash
  build-aux/lint-checks.sh
  build-aux/coverage.sh
  ```
- [ ] Translations in step with the code: `build-aux/i18n.sh` (then `git status`: `po/bookx.pot` must not change; if it does, commit it)
- [ ] Wiki builds, links and pictures OK: `build-aux/wiki.sh`
- [ ] Release check: `build-aux/release-check.sh` (install `actionlint` first to turn the one skip into a pass)
- [ ] Supply chain: `cargo deny check` and `cargo audit` (`cargo install --locked cargo-deny cargo-audit`; cargo-deny 0.22 needs rustc 1.95, use 0.21 on older compilers)
- [ ] UI scenarios: `build-aux/ui-tests.sh` (needs `gtk4-broadwayd`)
- [ ] Real-session suite: `build-aux/session-tests.sh` (needs GNOME Shell; skips itself without it)
- [ ] Release build and smoke test (what the workflow builds):
  ```bash
  cargo build --release --locked -p bookx -p bookx-cli
  target/release/bookx-cli --version        # bookx-cli 0.3.0
  ```
- [ ] Optional: build the Flatpak locally on GNOME 51: `BUNDLE=1 build-aux/flatpak.sh` (needs `org.gnome.Sdk//51`, `org.freedesktop.Sdk.Extension.rust-stable//26.08`, `flatpak-builder`)

## 2. Version, changelog, metainfo (already done; re-check if you change anything)

- [ ] `grep -n '^version' Cargo.toml` says `0.3.0`; `git grep -n '"0.2.0"' -- '*.toml'` finds nothing
- [ ] `CHANGELOG.md`: `## [0.3.0] - 2026-11-22` is the first versioned heading. If the release slips a day, change the date here
      **and** in `data/com.adhadse.Bookx.metainfo.xml` (`<release ... date=...>`)
- [ ] `build-aux/flatpak-cargo-sources.py` (regenerates `build-aux/cargo-sources.json`; `release-check.sh` fails when stale)
      after any `Cargo.lock` change

## 3. Pin the screenshots, commit, tag

```bash
build-aux/pin-screenshots.sh v0.3.0          # metainfo screenshot URLs: main -> v0.3.0 (Flathub wants a tag or commit)
git diff --stat                               # only data/com.adhadse.Bookx.metainfo.xml
build-aux/release-check.sh v0.3.0             # the tag now has to match the version
git add -A && git commit -m "Release 0.3.0"
git push origin main
```

- [ ] Wait for CI (`ci.yml`) on that commit to go green. If this is the first run on GitHub, expect to fix workflow
      problems first (they were only parsed locally). **Do not tag a red commit.**
- [ ] Tag (annotated, signed commit not required):
  ```bash
  git tag -a v0.3.0 -m "Bookx 0.3.0"
  git push origin v0.3.0
  ```
  Pushing the tag starts `release.yml`. A wrong tag: delete it on GitHub (Releases and Tags) and locally
  (`git tag -d v0.3.0`) **before** anyone downloads anything; a published signature is in the public Rekor log for ever.

## 4. What `release.yml` does for the tag `v0.3.0`

Trigger: a push of a tag `v*` (or `workflow_dispatch`, which builds but does not publish). Four jobs:

1. **tarball** (`fedora:latest` container): installs GTK 4, libadwaita, GtkSourceView 5 and the toolchain;
   runs `build-aux/release-check.sh v0.3.0` (the tag, `Cargo.toml`, metainfo and `CHANGELOG.md` must agree, the Flatpak
   sources must match `Cargo.lock`, golden plugin files unchanged); `cargo build --release --locked -p bookx -p bookx-cli`;
   `build-aux/tarball.sh dist` writes `bookx-0.3.0-linux-x86_64.tar.gz` (+ `.sha256`; deterministic packing, installs the
   tree under `usr/`: both binaries, desktop file, metainfo, MIME XML, icons, locale `.mo` files, README, LICENSE, NOTICE, CHANGELOG);
   `cargo deb --no-build` writes `bookx_0.3.0-1_amd64.deb`; `cargo generate-rpm` writes `bookx-0.3.0-1.x86_64.rpm`.
2. **flatpak** (`ghcr.io/flathub-infra/flatpak-github-actions:gnome-51`): regenerates `cargo-sources.json`, builds
   `build-aux/com.adhadse.Bookx.json` with `flatpak/flatpak-github-actions/flatpak-builder@v6.8`, writes `bookx.flatpak` and
   `bookx.flatpak.sha256`.
3. **publish** (needs both; only for tags): downloads the artifacts, writes `SHA256SUMS` of every file;
   `actions/attest-build-provenance` (GitHub build-provenance attestation, SLSA style, for every file);
   `cosign sign-blob --yes --bundle FILE.sigstore.json FILE` for **every file including `SHA256SUMS`** (keyless: the
   workflow's OIDC identity `https://github.com/adhadse/Bookx/.github/workflows/release.yml@refs/tags/v0.3.0`, recorded in
   Rekor; no secret exists); then **`cosign verify-blob` of every file** with the same identity regexp users use (the job
   fails if this fails); finally `softprops/action-gh-release` creates the GitHub release with all files and
   *generated* release notes.
4. Release assets after a green run (7 files and one `.sigstore.json` bundle for each): `bookx-0.3.0-linux-x86_64.tar.gz`, `bookx_0.3.0-1_amd64.deb`,
   `bookx-0.3.0-1.x86_64.rpm`, `bookx.flatpak`, the `.sha256` files, `SHA256SUMS`, and one `*.sigstore.json` per file.

Other workflows are independent: `wiki.yml` deploys the wiki on changes under `wiki/`; `package-*.yml` build the
Flatpak/macOS/Windows artifacts without publishing; `coverage-ui.yml` is the nightly UI tier; `i18n.yml` checks strings.

- [ ] Watch it: `gh run watch -R adhadse/Bookx` (the release job log shows the cosign verify lines)
- [ ] If a job fails **before publish**, fix, delete the tag and tag again (nothing was published). If it fails **in** publish
      after signing, delete the release and tag, fix, retag (a new Rekor entry is made; harmless).

## 5. Verify the artifacts (what users do; wiki `install.md`, "Verify your download")

```bash
mkdir /tmp/bookx-verify && cd /tmp/bookx-verify
gh release download v0.3.0 -R adhadse/Bookx
sha256sum -c SHA256SUMS --ignore-missing
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp '^https://github\.com/adhadse/Bookx/\.github/workflows/release\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS      # Verified OK
gh attestation verify bookx-0.3.0-linux-x86_64.tar.gz --repo adhadse/Bookx
git rev-parse v0.3.0^{commit}      # same commit as on the release page
```

- [ ] Checksums, cosign and attestation all pass for the tarball, deb, rpm and Flatpak bundle
- [ ] Install test, at least once each, on a machine that is not the build machine:
      `flatpak install --user bookx.flatpak && flatpak run com.adhadse.Bookx`; `sudo dnf install ./bookx-0.3.0-1.x86_64.rpm` (Fedora 43+: GTK 4.20, libadwaita 1.8);
      `sudo apt install ./bookx_0.3.0-1_amd64.deb` (needs a distribution that ships GTK 4.20 and libadwaita 1.8; **older ones cannot install it**: check `apt policy libgtk-4-1 libadwaita-1-0` first)
- [ ] Open a book, add the library, quit (the first-run path), and `flatpak run --command=bookx-cli com.adhadse.Bookx --version`

## 6. Release notes (GitHub release body)

`generate_release_notes: true` writes a changelog of merged PRs, which is nearly empty for this repository. Edit the release
(`gh release edit v0.3.0 -R adhadse/Bookx --notes-file notes.md`) and put this above the generated part:

```markdown
## Bookx 0.3.0: the first public release

A GNOME e-book library, reader and EPUB editor written in Rust (GTK 4, libadwaita). Free software (GPL-3.0-or-later).

**Highlights**
- Library with custom columns, saved searches, Library Health (duplicates, backups, trash) and Calibre import
- Devices window for Kobo, Kindle and other e-readers; **Bookx sync for KOReader** (highlights, notes, bookmarks) and an OPDS server
- Incremental, encrypted **backups** to a folder, drive, network share or WebDAV; passwords in the system keyring or an encrypted store
- **Read aloud** with natural offline voices, audiobooks, a comic reader, vertical Japanese and Chinese text
- EPUB editor with syntax highlighting, live preview, a Problems panel and a command palette
- Spanish, German, French and Hindi translations (machine-assisted; native review welcome)

**Install**: the Flatpak bundle below, a `.deb`, an `.rpm` or the tarball. Every file is signed
(Sigstore, keyless) and has a build-provenance attestation: [Verify your download](https://bookx.adhadse.com/install/#verify-your-download).
Not on Flathub yet. Needs GTK 4.20 and libadwaita 1.8 (the Flatpak brings its own).

**Please note**: this is a young project. Back up your library before you let Bookx write to an e-reader. Sync with a
Kobo/KOReader has had limited hardware testing. Report problems with *About → Troubleshooting → Copy Diagnostics*:
https://github.com/adhadse/Bookx/issues/new/choose

Full list of changes: [CHANGELOG.md](https://github.com/adhadse/Bookx/blob/v0.3.0/CHANGELOG.md).
```

## 7. Announce (after section 5 passes)

- [ ] GitHub release published (not draft); repository description, homepage and topics are set (done 2026-11-22 with `gh repo edit`)
- [ ] **Social preview image**: Settings → General → Social preview → upload a 1280x640 PNG (GitHub offers no API for this;
      the wiki social cards are built at `wiki/site/assets/images/social/`)
- [ ] Website <https://bookx.adhadse.com> shows 0.3.0 (the wiki deploys on pushes under `wiki/`; the pinned-screenshot commit touches none, so run `gh workflow run wiki.yml` once)
- [ ] Announce where you want to: Mastodon / Fediverse, r/gnome, r/ebooks, r/kobo, MobileRead forum (Kobo and KOReader sub-forums), This Week in GNOME (submit through their matrix room after Flathub), Hacker News "Show HN". Keep it honest: young project, AI-assisted development (see `AGENTS.md`), what is untested (hardware sync)
- [ ] Enable GitHub **Discussions** (optional, off now) or say in the README that issues are the channel

## 8. Flathub

Read `docs/flathub-submission.md` (blockers, permissions, step by step); the AI policy (blocker 1) means **you** write the
manifest and the pull request. The agent prepared: runtime 51 build verified, `packaging/flathub/` reference files with an
official-generator `cargo-sources.json`, the linter results (§5 there), and `packaging/flathub/prepare.sh v0.3.0` which
prints the pinned tag and commit after the tag exists.

- [ ] Flathub account (log in with GitHub; 2FA on GitHub)
- [ ] Decide the open points: AI disclosure text, which permissions to keep (`/run/media`, gvfs), ONNX Runtime download, translations (ship English-only first?), aarch64
- [ ] `packaging/flathub/prepare.sh v0.3.0`, write your manifest by hand from it, `flatpak-builder-lint manifest/repo`, then the PR against `flathub/flathub` branch `new-pr`
- [ ] Tell reviewers about the project history: the repository dates from 2023, 0.2.0 was a rewrite, 0.3.0 is the first tagged release

## 9. After the release

- [ ] **Flathub verification token** (when the PR is accepted): developer portal gives a token; publish it at
      `https://adhadse.com/.well-known/org.flathub.VerifiedApps.txt` (apex domain, HTTPS). Link the app on <https://bookx.adhadse.com> as visible proof
- [ ] **Weblate**: create the project `bookx` on hosted.weblate.org (libre-project request), component `bookx` on `po/*.po`
      with `po/bookx.pot`, add the webhook (`docs/translations-hosting.md`); update the wiki Translating page once it is live
- [ ] **Branch protection with the required `coverage` check**, after the first green CI run (check names first):
      ```bash
      gh api repos/adhadse/Bookx/commits/$(git rev-parse HEAD)/check-runs --jq '.check_runs[].name'
      ```
      then the `gh api -X PUT repos/adhadse/Bookx/branches/main/protection` command in `docs/ci-gates.md` ("Making `coverage` a required check")
- [ ] Tag protection for `v*` (Settings → Rules → Rulesets → tags): only you can create or delete release tags
- [ ] GitHub Pages: Settings → Pages → Source "GitHub Actions"; DNS `CNAME bookx → adhadse.github.io` (`docs/wiki-notes.md`) if not done
- [ ] Pin third-party GitHub Actions by commit SHA (Dependabot will keep them current); `release.yml` uses tags for
      `softprops/action-gh-release`, `sigstore/cosign-installer`, `actions/attest-build-provenance` and `flatpak/flatpak-github-actions`
- [ ] Watch issues for the first week; keep `[Unreleased]` in `CHANGELOG.md` filling; next version: bump, tag, repeat from section 1
- [ ] Hardware checks that are still open (`docs/hardware-test-plan.md`, `docs/remaining-work.md` A): Kobo sync, Kindle, MTP, natural voices inside the Flatpak

## 10. Notes the agent could not settle (decide before the tag)

- **Third-party artwork in `mockups/legacy/` and `_legacy/`** (the 2023 prototype): the two legacy mockup SVGs were started from GNOME
  design mockups (Inkscape export paths in the files name them). They are not built or shipped. To be safe for a public
  release, either keep them and accept the CC BY-SA origin (NOTICE.md says so), or `git rm -r mockups/legacy _legacy`
  (history keeps them). Nothing else in `_legacy` is needed.
- **Binaries are not stripped** (`bookx` is 81 MiB, `bookx-cli` 39 MiB). `cargo deb` strips, the tarball and Flatpak do not.
  A `strip = "symbols"` line in `[profile.release]` would shrink them a lot; it was not added because it changes the
  build for everybody and backtraces in bug reports become useless. Your call.
- **`.po` headers** lack `Last-Translator`, `Language-Team`, `MIME-Version`, `Content-Transfer-Encoding` (`msgfmt -c` warns,
  does not fail). Weblate fills them on the first commit.
