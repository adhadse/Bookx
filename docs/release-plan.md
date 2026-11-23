# Release plan: build history first, stores later

Internal (not for the wiki). Owner decision, 2026-11-22: *"We need to build up history, to show Flathub maintainers we've
built a credible application. Let's focus on the non-Flathub way of releasing first: from the GitHub releases themselves,
which anybody can download. Once we build a credible reputation we'll think about package-based releases."*

Status: living | Last reviewed: 2026-11-23 | Owner: @adhadse

So: **GitHub Releases is the only official channel.** Flathub, Homebrew, winget, Scoop and MSYS2 are *later*, documented
here and in `docs/flathub-submission.md`, and are **not promised publicly** (the wiki install page says "may come later,
no date"). The 0.3.0 runbook is `docs/release-0.3.0.md`.

## 1. Release process (every release)

1. Gates of the runbook §1 on a clean `main` (fmt, clippy, tests, coverage, lint-checks, i18n, wiki, release-check).
2. Move `[Unreleased]` in `CHANGELOG.md` to `[X.Y.Z] - date`, bump versions, add `<release>` to the metainfo,
   `build-aux/pin-screenshots.sh vX.Y.Z`, commit, wait for green CI, tag `vX.Y.Z`.
3. `release.yml` builds and signs; verify the assets as a user would (runbook §5).
4. Edit the release notes from the CHANGELOG (highlights, known limits, how to report, link to the install page).
5. Announce (runbook §7) only for minor releases; patch releases get the release page and the changelog.

Every release must have: tarball, `.deb`, `.rpm`, Flatpak bundle, `SHA256SUMS`, a cosign `.sigstore.json` for each file,
the GitHub build-provenance attestation, and (proposal below) an SBOM.

## 2. Cadence and versioning

- **Semantic versioning** while 0.x: `0.MINOR.PATCH`. A patch (0.3.1) has only fixes and tiny improvements, no new
  setting and no file-format change. A minor (0.4.0) adds features and may change settings or the catalogue schema; it must
  say so in the notes and migrate data (backup first). 1.0.0 only when the Flathub criteria (section 5) have been met
  for some time and the catalogue/plugin ABI are considered stable.
- **0.3.0 now** (first tagged release). **Patch releases 0.3.x about every 2 weeks** while there are fixes (target: first
  one within 2 weeks of 0.3.0, driven by early bug reports). **Minor releases about monthly** (0.4.0, 0.5.0 …).
  Skip a cycle rather than release something unverified; but do not go quiet for more than about a month.
- **Changelog discipline**: every user-visible change gets a line in `[Unreleased]` in the same pull request (the release
  check refuses a tag whose CHANGELOG heading is missing). Release notes are the CHANGELOG section plus a short intro.
- Security fixes: patch release as soon as ready, with an advisory (`docs/security.md`), not bundled with features.

### SBOM proposal for `release.yml` (for the CI agent)

Add a step to the `tarball` job after the build, before `upload-artifact`, and let `publish` sign it like every file:

```yaml
      - name: SBOM (CycloneDX)
        run: |
          cargo install --locked cargo-cyclonedx
          cargo cyclonedx --format json --all-features --override-filename bookx-${GITHUB_REF_NAME#v}.cdx
          mv *.cdx.json dist/ 2>/dev/null || true     # one file per workspace crate: ship only the `bookx` one,
                                                      # or merge them; check the file names on the first run
```

Notes: `cargo cyclonedx` writes one `.cdx.json` per crate; ship only the `bookx` crate's (it lists the whole dependency
tree through `Cargo.lock`), named `bookx-X.Y.Z.cdx.json`. Pin the tool version (`cargo install --locked cargo-cyclonedx@VERSION`).
`cargo sbom` (SPDX) is the alternative if CycloneDX output proves awkward. Also attach it to the attestation with
`actions/attest-sbom` if wanted. Mark `docs/security.md` G13 "SBOM" as done afterwards. The workflow was never run on GitHub: first run on a tag candidate (`workflow_dispatch` builds but does not publish).

## 3. Signals reviewers (Flathub and users) look for

Checkable list; tick when true and note the date.

**Activity and history**

- [ ] Tagged releases: 0.3.0 plus a steady series (section 5 sets the minimum)
- [ ] Issue tracker in use: every issue answered or triaged within about 3 days; labels (`bug`, `enhancement`, `good first issue`, `help wanted`, `needs-diagnostics`, `translation`) exist and are used; stale issues closed with a reason
- [ ] 5 or more `good first issue` items open at any time
- [ ] GitHub Discussions switched on (Q&A, ideas, show-and-tell) or a stated reason why issues are the channel
- [ ] Public roadmap: a pinned issue, a GitHub Project board or a wiki page "Roadmap" derived from `docs/bookx-feature-roadmap.md` (user-facing items only, no internal status)
- [ ] Contributor docs: CONTRIBUTING, CODE_OF_CONDUCT, PR template, issue forms (done), plus a "first contribution" walkthrough
- [ ] Security policy `SECURITY.md` (done) and, once a vulnerability is fixed, a published GitHub Security Advisory
- [ ] Download counts visible on the release assets (`gh api repos/adhadse/Bookx/releases --jq '.[].assets[]|[.name,.download_count]'`): record them per release in the table at the end
- [ ] A working feedback loop: *About → Troubleshooting → Copy Diagnostics* in every issue form (done), and bug reports turning into fixes in the CHANGELOG
- [ ] Project site up to date (`bookx.adhadse.com`), screenshots current, install page matching the last release

**Quality and trust**

- [ ] CI badge in the README (needs a green `ci.yml` run; none has run on GitHub yet), coverage gate required on `main` (`docs/ci-gates.md`), branch and tag protection set
- [ ] Signed artifacts and provenance on every release (in `release.yml`), SBOM (section 2)
- [ ] Reproducibility notes: the tarball packing is deterministic; the binaries are not. Write what is and is not reproducible in `docs/security.md` instead of claiming more
- [ ] Dependencies audited per release (`cargo deny check`, `cargo audit` in CI) and third-party actions pinned by SHA (18 unpinned on 2026-11-22)
- [ ] Translations: a native speaker has reviewed at least one language, or the build is English-only (`reviewed-locales-only`)
- [ ] Hardware claims honest: the hardware test plan (`docs/hardware-test-plan.md`) results are published, with limits stated in the wiki

## 4. Distribution milestones, in order

| # | Milestone | Needs first |
|---|---|---|
| 1 | **GitHub Releases + bundles** (now): tarball, `.deb`, `.rpm`, Flatpak bundle, signed | 0.3.0 tagged and verified (runbook) |
| 2 | **Own Flatpak repository on GitHub Pages**: users run `flatpak remote-add --user bookx https://bookx.adhadse.com/bookx.flatpakrepo` and get updates through Flatpak. Needs a `.flatpakrepo` file, an OSTree repo built by `flatpak build-bundle`/`flatpak build-export` in CI and a **GPG signing key**: decision on key custody (CI secret vs offline key; note that it replaces the "no key anyone holds" property of the keyless release signing, so document the trade-off and rotation) | milestone 1 stable for a few releases; the owner's key-custody decision |
| 3 | **Flathub submission** | criteria in section 5; `docs/flathub-submission.md`; hand-written manifest and PR by the owner (AI policy) |
| 4 | **Homebrew tap/cask and winget/Scoop** (and MSYS2) | macOS and Windows builds exist and are tested on real machines, and a **native audio sink** replaces PulseAudio/PipeWire for read aloud and audiobooks on those systems (`docs/macos-port.md`, `docs/windows-port.md`) |

The wiki must keep saying only: GitHub Releases is the channel; other routes may come later (no dates).

## 5. Exit criteria: "ready to approach Flathub"

All of these, checked on the day (and the Flathub requirements re-read: `docs/flathub-submission.md` §0):

- [ ] At least **3 tagged releases** published over at least **2 to 3 months** (the first on 0.3.0, the date is in section 7)
- [ ] At least **N = 200 total downloads** across releases (a rough bar for "real people use it"; adjust and say why), and at least a handful of issues from people other than the owner
- [ ] Fewer than **X = 3 open critical bugs** (data loss, crash at start, security) and none older than one release
- [ ] Issue response time under about 3 days over the last month; first outside contributions merged or at least answered
- [ ] CI green for the last 20 runs; coverage gate required; the installs of the previous release verified on at least two distributions (Fedora, Debian or Ubuntu) and the Flatpak bundle started by hand
- [ ] **Translations**: reviewed by a native speaker for at least one non-English language, or the Flathub build stays English only (already supported: `reviewed-locales-only`)
- [ ] Permissions decision made (`/run/media`, gvfs, Documents) and the ONNX Runtime download decision made (`docs/flathub-submission.md` blockers 4 and 6)
- [ ] The AI-disclosure text below reviewed and edited by the owner
- [ ] The GNOME runtime in the manifest is the newest on Flathub on the day

## 6. Flathub disclosure draft (to be reviewed by the owner)

Draft only. The owner must read it, correct anything untrue and write the final text and the pull request personally
(Flathub does not allow AI tools to write or open the submission).

> **How Bookx was written.** Bookx is developed by Anurag Dhadse with the help of AI coding agents (large language
> models used through an agent harness). A large part of the Rust code, the unit and end-to-end tests, the documentation
> (the user wiki and internal notes), the build and packaging scripts (the Flatpak and distribution packages, CI
> workflows) and the translations (German, Spanish, French and Hindi, machine-translated and not yet reviewed by native
> speakers) was produced by agents working under the owner's direction: the owner sets the goals and design decisions
> (the roadmap records them), and reviews and runs the results. The release manifest for Flathub and this submission were
> written by the owner by hand, not by an AI tool.
>
> **What guards the quality of that code.** The repository requires a test with every change (`AGENTS.md`); there are
> unit tests per crate, a suite of end-to-end UI scenarios run on a Broadway display and a real GNOME Shell session
> suite, fuzz targets and tests with hostile files for every parser, and performance budgets. CI runs formatting,
> clippy with warnings denied, the tests, a coverage gate per crate that may not fall, dependency checks (`cargo deny`,
> `cargo audit`) and a release check that the tag, version, metainfo and changelog agree. There is a written security
> model and a per-change checklist for code that handles untrusted input (`docs/security.md`), and the library crates
> forbid `unsafe` code. Releases are signed with Sigstore and carry a build-provenance attestation; the source is public
> and licensed GPL-3.0-or-later. Bug reports include a diagnostics text so problems can be reproduced.
>
> **What this does not mean.** The tests and gates reduce, but do not remove, the risk of defects in AI-written code. Parts
> of Bookx (device sync, natural voices inside the sandbox) have had limited testing on real hardware; the project says so
> in its documentation.

Check the claims before submitting (each one must be true that day): the test counts and CI gates are in
`docs/remaining-work.md` and `docs/ci-gates.md`; fuzz targets in `fuzz/`; translations status in `po/STATUS.md`.

## 7. Release log (fill in)

| Version | Date | Assets verified | Downloads after 2 weeks | Open critical bugs | Notes |
|---|---|---|---|---|---|
| 0.3.0 | (tag date) | | | | first tagged release |
