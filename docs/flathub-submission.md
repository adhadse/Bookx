# Flathub submission

Owner decision (2026-11): **Flathub is the first-class distribution channel.** This page is the internal checklist
(not for the public wiki). Rules come from <https://docs.flathub.org/docs/for-app-authors/requirements> and
`/submission`, read on 2026-11-21. Re-read them on the day you submit; they change.

## 0. Read this first: blockers and risks

| # | Blocker or risk | What to do | Who |
|---|---|---|---|
| 1 | **Generative AI policy.** Flathub requires you to *disclose* AI-generated code, documentation or packaging, says **manifests must not contain AI-generated or AI-assisted content**, and forbids AI tools to open or automate the submission pull request or to write its commit messages, description and replies. Bookx is developed with AI assistants (`AGENTS.md`). | Write the Flathub manifest **yourself, by hand**. `packaging/flathub/com.adhadse.Bookx.yml` was drafted by an AI assistant: use it only as a list of values to check against, not as the file you submit. Write the PR text yourself. Prepare an honest disclosure (which parts of the app were written with AI help and how much). Reviewers may reject based on extent. If unsure, ask first in the Flathub Matrix room or a `flathub/flathub` issue (the rules invite that). | owner |
| 2 | **"Latest runtime at submission."** The manifest must use the newest runtime on Flathub. GNOME **51** is out (`org.gnome.Platform//51`, freedesktop 26.08, `rust-stable//26.08` exist). The manifests use 51 and the build was verified (section 5). | Done: built on 51 (section 5). Submit with `runtime-version: '51'` unless a newer runtime is out on the day. | owner |
| 3 | **Insufficient development history.** "Applications that have only existed for a very short period of time will generally not be accepted"; tagged releases and real-world use are expected. Version 0.2.0 is described as a complete rewrite and there is **no git tag yet**. | Tag releases (`v0.2.0` or `v0.3.0`) on GitHub with release notes; let it be used for a while; mention the history of the project in the PR. | owner |
| 4 | **Static permissions must be "an absolute minimum"; a portal is mandatory when one covers the use.** Bookx asks for `xdg-documents`, `/run/media`, `xdg-run/gvfs(d)`, `org.gtk.vfs.*`. Reviewers review permissions separately and can refuse any of them. | Section 3 has the justification for each and a fallback (drop it, use the file chooser portal). Be ready to lose `/run/media` and the Documents default. | owner decision |
| 5 | **Machine translated locales.** "Low quality, automated or machine-generated or mixed translations cannot be used." de, es, fr and hi were all written by an AI agent and no native speaker has reviewed them (`po/STATUS.md`). English is complete. | **Handled in the build, nothing to decide before submitting.** The Flathub manifest builds with `--features bookx/reviewed-locales-only` and `CARGO_FEATURES=bookx/reviewed-locales-only` for `make install-flatpak`: only English and the codes in `po/reviewed-locales.txt` (empty today) are embedded or installed; the language picker shows English and the system default. The desktop file and metainfo have no translated text (a test checks it). Other channels (tarball, Windows, macOS, bundles) keep all four languages. To ship a language on Flathub, a native speaker reviews it (Weblate *approved*), then a pull request sets `reviewed` in `po/STATUS.md` and adds the code to `po/reviewed-locales.txt` (`docs/translations-hosting.md`, "Promoting a language"). Say in the PR that Flathub ships English only for now. | owner (when a reviewer is found) |
| 6 | **Runtime download of ONNX Runtime.** The natural voices fetch `libonnxruntime.so` (hash pinned, `crates/bookx-tts/src/models.rs`) into the user's data folder on first use. "Binary or precompiled files must not be present in the submission", and running downloaded native code may be questioned. Voice *models* are data and are fine. | Explain it in the PR (user-initiated, SHA-256 pinned, only for read aloud). Fallbacks: build ONNX Runtime as a manifest module (big: C++, hours of build time), or ship without the local-voice engines and keep the voice-server engines. | owner decision |
| 7 | **Build from a clean checkout.** `.gitignore` had `repo/`, which silently excluded `crates/bookx-backup/src/repo/` (the backup repository). A clone of `main` does **not compile** without it (found by the Flatpak build of a `git archive`). Fixed to `/repo/` in this change. | **Commit `crates/bookx-backup/src/repo/`.** Then check a clean clone builds: `git clone … /tmp/x && cd /tmp/x && cargo check`. | owner/agent |
| 8 | Screenshots in the metainfo must be "from a tag or a commit and not a branch". They point to `main` now. | Before tagging: `build-aux/pin-screenshots.sh vX.Y.Z`, commit, then tag. | agent |
| 9 | `aarch64` was never run; Flathub builds both architectures by default. | `packaging/flathub/flathub.json` limits the first submission to `x86_64`. Remove the file once an aarch64 build works (or delete `only-arches`). | owner |

## 1. What is in the repository

| File | Purpose |
|---|---|
| `build-aux/com.adhadse.Bookx.json` | Development manifest (local directory source). Used by `build-aux/flatpak.sh`, `release.yml`, `package-flatpak.yml`. **Not** what Flathub gets. |
| `build-aux/cargo-sources.json`, `build-aux/flatpak-cargo-sources.py` | Offline crates from `Cargo.lock` (our own generator, pure stdlib). `release-check.sh` fails when stale. Regenerated in this change: `keyring` and its target crates were added to the lock file. |
| `packaging/flathub/com.adhadse.Bookx.yml` | **Reference draft** of the Flathub manifest (git source, `x-checker-data`). See blocker 1: re-type it yourself. |
| `packaging/flathub/flathub.json` | `only-arches: [x86_64]`. |
| `packaging/flathub/prepare.sh vX.Y.Z` | Clones the tag, writes manifest + `cargo-sources.json` + `flathub.json` into a directory (default `/tmp/opencode/flathub-bookx`). Tested against a throw-away local tag only. |
| `build-aux/flathub-update.py`, `.github/workflows/flathub-update.yml`, `build-aux/tests/` | Update pull requests for the Flathub repository after a release (`docs/flathub-automation.md`). |
| `po/reviewed-locales.txt`, `build.rs`, cargo feature `reviewed-locales-only` | The Flathub build embeds English and only these reviewed languages (blocker 5). `REVIEWED_LOCALES=1 build-aux/flatpak.sh` builds the development manifest that way. |
| `.github/workflows/package-flatpak.yml` | Builds the bundle in CI (artifact, no publishing) and runs the Flathub linter (`continue-on-error` until it has run once). |
| `data/com.adhadse.Bookx.{desktop,metainfo.xml,mime.xml}` | Metadata. Metainfo validates with `appstreamcli validate --pedantic` (only the unavoidable `cid-contains-uppercase-letter` pedantic note remains: the ID is fixed). |

Changes made for Flathub in this pass: summary shortened to 34 characters ("E-book library, reader and editor"; the
linter wants fewer than 36), `vcs-browser` URL added, screenshot URLs fixed (`master` -> `main`; the default branch is
`main`, so every screenshot URL was a 404), `--own-name=org.mpris.MediaPlayer2.Bookx.*` added to the sandbox (without it
the media-key player `src/mpris.rs` cannot own its bus name inside a Flatpak), `.gitignore` fixed (blocker 7).

## 2. Application ID and verification

`com.adhadse.Bookx`: three components, domain `adhadse.com` (reversed ID without the last component). It is valid for
a domain you control, and verification is by token: Flathub gives you a token at <https://flathub.org/developer-portal>
and you publish it at `https://adhadse.com/.well-known/org.flathub.VerifiedApps.txt` (the apex domain of the ID, not
`bookx.adhadse.com`). The page <https://bookx.adhadse.com> should link the app to the developer ("visible proof"). The ID
must be reachable over HTTPS. The ID cannot be renamed later without resubmitting, so decide now (the alternative
`io.github.adhadse.Bookx` would be verified through GitHub but changes the ID in the desktop file, metainfo, icons,
settings paths, D-Bus names and the `bookx://` handler: not recommended).

## 3. Permissions (what to write in the PR, in your own words)

| Permission | Why it is needed | Fallback if refused |
|---|---|---|
| `--share=network` | metadata sources, OPDS, WebDAV backups, KOReader/Bookx sync, library share server, model downloads | none (core) |
| `--socket=wayland`, `fallback-x11`, `--device=dri`, `--share=ipc` | window, GPU | none |
| `--socket=pulseaudio` | read aloud and audiobook playback: Bookx pipes PCM to `pacat`/`pw-cat` (`crates/bookx-tts/src/sink.rs`, `bookx-audio/src/player.rs`) | none today (no portal for streaming audio) |
| `--filesystem=xdg-documents` | default library `~/Documents/Bookx Library`, plain files that the user also uses from other programs and backs up | choose the library with the file chooser portal on first start (not built: the library path must then be persisted through a portal document, or documented as `flatpak override`) |
| `--filesystem=/run/media` | e-readers mounted as USB disks: Bookx lists, writes and reads back books and the Kobo database on them (`crates/bookx-serve/src/devices.rs`). There is no portal for "any removable drive". | the user picks the mounted reader once in the file chooser (works, but no hot-plug detection); or `flatpak override` |
| `--filesystem=xdg-run/gvfs`, `xdg-run/gvfsd`, `--talk-name=org.gtk.vfs.*` | MTP phones and e-readers, network shares and cloud drives mounted by GNOME Files (`crates/bookx-serve/src/mtp.rs`, backup places) | drop; those devices are then not reachable |
| `--own-name=org.mpris.MediaPlayer2.Bookx.*` | media keys and the shell media widget while reading aloud | drop; no media keys |
| Passwords | no permission: `oo7` uses the `org.freedesktop.portal.Secret` portal inside a Flatpak (encrypted file in the app's data directory, unlocked through the portal) | -- |
| Printing, notifications, "Open With", links | portals (`gtk::FileDialog`, `show_uri`) | -- |

Not requested (and the linter/`release-check.sh` refuse them): `--filesystem=host|home`, `--device=all`, session or
system bus access. Speech: the engines are subprocesses (`espeak-ng` is a manifest module; Kokoro/Piper run inside
the app through the ONNX Runtime library), no `speech-dispatcher` (removed). `bookx-cli` is installed next to the app.

## 4. Step-by-step

1. **Prepare the tree** (agent work, done before you start): commit `crates/bookx-backup/src/repo/` and the changes of
   this pass; `build-aux/release-check.sh` passes; wiki has the install page (`wiki/docs/install.md`).
2. **Decide the open points** in section 0 (AI disclosure, permissions, ONNX Runtime, translations, aarch64).
3. **Pin screenshots and release**: `build-aux/pin-screenshots.sh v0.3.0`, update CHANGELOG/metainfo release entry, commit,
   `git tag -a v0.3.0 -m "Bookx 0.3.0" && git push origin v0.3.0`. The tag must be a real GitHub release (release notes).
4. **Set up the domain token** (section 2) once you have a Flathub account (log in at flathub.org with GitHub; 2FA
   on GitHub is required for the maintainer invitation).
5. **Write the manifest by hand.** Style rules (2 spaces, logical key order, no value alignment, comments only where
   needed) are on the requirements page. Start from the values in `packaging/flathub/com.adhadse.Bookx.yml`.
   Generate `cargo-sources.json` with the official tool, which is not AI output and which reviewers know:
   ```bash
   git clone https://github.com/flatpak/flatpak-builder-tools && cd flatpak-builder-tools/cargo
   pip install aiohttp tomlkit && python3 flatpak-cargo-generator.py /path/to/Bookx/Cargo.lock -o cargo-sources.json
   ```
   (Our `build-aux/cargo-sources.json` is equivalent in effect and is what CI uses; the official file is the safer
   thing to put in the Flathub repo. The build commands stay: `CARGO_HOME=/run/build/bookx/cargo`,
   `cargo --offline build --release --locked -p bookx -p bookx-cli --features bookx/reviewed-locales-only`, `make install-flatpak PREFIX=/app TARGET_DIR=target/release CARGO_FEATURES=bookx/reviewed-locales-only`; blocker 5.)
   `packaging/flathub/prepare.sh v0.3.0` shows what the pinned values look like (tag and the full 40-character commit).
6. **Build and lint locally** (as Flathub does):
   ```bash
   flatpak install -y flathub org.flatpak.Builder
   flatpak run --command=flathub-build org.flatpak.Builder --install com.adhadse.Bookx.yml
   flatpak run com.adhadse.Bookx
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest com.adhadse.Bookx.yml
   flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
   ```
   Fix findings; an exception request (for example for `/run/media`) goes into a `flathub/flatpak-builder-lint` pull
   request or is asked for in the PR.
7. **Open the PR** (yourself, in the web UI or with `gh`; not with an AI tool):
   - Fork `flathub/flathub` **with all branches** (untick "Copy the master branch only").
   - `git clone --branch=new-pr git@github.com:<you>/flathub.git && cd flathub && git checkout -b add-bookx new-pr`
   - Add only the manifest, `cargo-sources.json` and `flathub.json` (top level; no source code, no binaries, no copy
     of the metainfo/desktop file/icon: those come from the Bookx source).
   - Open the PR **against the `new-pr` base branch**, titled `Add com.adhadse.Bookx`. Keep the PR template and fill it
     in; explain permissions, ONNX Runtime download and the AI disclosure.
   - When review comments are answered, comment `bot, build` to start a test build. Screenshots do not show in
     test builds. Do not close the PR to change something; do not merge `master` into the branch.
8. **After approval** a repository `flathub/com.adhadse.Bookx` is created and you get a write invitation (accept within
   a week). The first official build is published in one to two hours. Verify the app in the developer portal
   (section 2), then link the Flathub page from the README and the wiki.

## 5. Local build status (this machine, 2026-11-22, GNOME 51)

Built from a `git archive HEAD` snapshot at version 0.3.0 with `flatpak-builder --user --install --force-clean` on
`org.gnome.Platform//51` + `org.gnome.Sdk//51` + `org.freedesktop.Sdk.Extension.rust-stable//26.08` (all in `--user`; the
development manifest `build-aux/com.adhadse.Bookx.json` and `packaging/flathub/com.adhadse.Bookx.yml` now say runtime `51`).

| Check | Result |
|---|---|
| Full offline build on GNOME 51 (espeak-ng 1.52.0 module + 584 vendored crates + release build) | **passes** (about 10 minutes including the crate downloads; no manifest or code change was needed for 51) |
| `flatpak run --command=bookx-cli com.adhadse.Bookx --version` | `bookx-cli 0.3.0` |
| `bookx-cli convert` (EPUB to FB2) and `check` inside the sandbox | work (the folder was granted with `--filesystem`) |
| Installed tree (`/app`) | `bin/{bookx,bookx-cli,espeak-ng,espeak,speak}`, desktop file, metainfo, MIME XML, icons, locale `de es fr hi`; no missing shared library (`ldd`) |
| `flatpak-builder-lint manifest` (dev manifest and `packaging/flathub` manifest, with `cargo-sources.json` next to it) | passes, no findings |
| `flatpak-builder-lint appstream` | passes |
| `flatpak-builder-lint repo` on the local build repository | only `appstream-external-screenshot-url` / `appstream-screenshots-not-mirrored-in-ostree`, which is what a local build without `--mirror-screenshots-url=https://dl.flathub.org/media` always shows; Flathub's builder mirrors them |
| Official `flatpak-cargo-generator.py` output vs ours | same 584 crates with identical URLs and sha256 (1169 entries each); the official file is committed as `packaging/flathub/cargo-sources.json`; ours stays in `build-aux/` for CI |
| GUI inside the sandbox, headless | **not verified**: the sandbox's GTK could not initialise the Broadway backend against the host `gtk4-broadwayd` (the runtime has no broadwayd), and a Wayland window on the owner's desktop was not opened during the real-session test run. Run `flatpak run com.adhadse.Bookx` once by hand |
| `release.yml` / `package-flatpak.yml` / `ci.yml` | updated to the `gnome-51` image and `flatpak/flatpak-github-actions/flatpak-builder@v6.8` (the old `flathub-infra/flatpak-github-actions` repository is archived; the images now come from `flathub-infra/actions-images`, which lists `gnome-51`). **Never run on GitHub.** |
| Natural voices (ONNX Runtime download), devices, secrets portal inside the sandbox | not tested (remaining-work A) |

What remains for the owner (everything else is done): the AI disclosure and decision (blocker 1), the Flathub account and
the hand-written manifest and PR, the domain token on `adhadse.com`, the permission decisions (section 3), the first real
GitHub run of the workflows, one manual `flatpak run` of the installed app, native translation review, `only-arches` for
aarch64. The tag `v0.3.0` must exist before `packaging/flathub/prepare.sh v0.3.0` works. Full runbook: `docs/release-0.3.0.md`.

## 6. How updates flow after acceptance

- The Flathub repository holds the manifest. Each new version is a pull request there (a maintainer merges it; builds
  and publishing are automatic; beta/nightly branches are not for new apps and nightlies are not allowed).
- `flathub-infra/flatpak-external-data-checker` (a bot running on the Flathub repositories) reads the
  `x-checker-data` blocks: the `git` type bumps `tag` and `commit` of the Bookx source when a new `vX.Y.Z` tag appears,
  `anitya` bumps `espeak-ng`. It opens a PR; **it cannot regenerate `cargo-sources.json`**, so the PR needs
  `cargo-sources.json` regenerated for the new `Cargo.lock` (run `flatpak-cargo-generator.py` on the new lock and push to
  the PR branch, or use the `packaging/flathub/prepare.sh vX.Y.Z` output as a comparison).
- **Automated updates (decision: use them): `.github/workflows/flathub-update.yml`**, a deterministic script
  (`build-aux/flathub-update.py`, no AI) that opens the update pull request when a release is published. It bumps
  `tag`/`commit`, regenerates `cargo-sources.json` with the official generator and never merges. Flathub's own bot
  is then switched off with `disable-external-data-checker` in `flathub.json` (the `espeak-ng` `anitya` bump is
  then done by hand, see the automation page). Setup, flow, rollback and failure modes:
  **`docs/flathub-automation.md`**. Mention the workflow in the submission PR (or on Matrix) so reviewers know a
  bot opens update pull requests. The paragraph above describes what Flathub's bot does when the workflow is not enabled.
- Runtime upgrades (GNOME 52 and so on) are manual edits of the manifest; Flathub marks old runtimes end-of-life and
  an app on an EOL runtime is flagged, so plan one runtime bump per GNOME release (every six months).
- Metadata (description, screenshots, release notes) comes from the metainfo in the tagged source, so a metainfo fix
  needs a new tag. Keep `release-check.sh` green before tagging.
- The developer portal (flathub.org) shows download statistics and the verification state.

## 7. Not done / needs the owner

Flathub account, the domain token on `adhadse.com`, the AI disclosure and decision (blocker 1), writing and submitting the
PR, the tag and GitHub release, native review of translations, hardware and sandbox tests (remaining-work A), and the
run of `package-flatpak.yml` on GitHub (the container image tag `gnome-50` and the linter call are from memory; with
GNOME 51 the image tag changes to `gnome-51` once the manifest moves).

## 8. Fallback channel if Flathub says no

If Flathub rejects the submission (generative-AI policy, development history, permissions), Bookx stays installable
without it. Documentation only, nothing below is implemented. The in-app text and the wiki install page must then say
where the builds come from.

1. **GitHub release bundles (exists).** `release.yml` attaches `bookx.flatpak` (x86_64) with a checksum to every release.
   Install: `flatpak install --user bookx.flatpak` (the GNOME runtime comes from Flathub as a dependency). No updates;
   the user installs the next bundle by hand.
2. **Self-hosted Flatpak repository on GitHub Pages (to build).** Gives `flatpak update`:
   - Create a signing key **held by the owner only** (`gpg --quick-gen-key "Bookx Flatpak <hello@adhadse.com>" ed25519 sign 3y`),
     keep the private key offline or in a password manager and put it in a CI secret only if the owner accepts that
     risk; publish the public key (`gpg --export`, base64 into the `.flatpakrepo`). Plan renewal before it expires and
     a revocation path.
   - In `release.yml` (or a new job): `flatpak build-export --gpg-sign=KEYID --gpg-homedir=... repo build-dir stable`
     from the already built `build-dir`, then `flatpak build-update-repo --gpg-sign=KEYID --generate-static-deltas
     --prune --prune-depth=3 repo`. Keep the repository directory between releases (a branch such as `flatpak-repo`
     or a release asset), otherwise old commits vanish and users cannot update incrementally.
   - Publish `repo/` with GitHub Pages under `https://bookx.adhadse.com/repo/` (mind the Pages size limit of 1 GB and
     the bandwidth soft limit; a `--prune` and few branches keep it small).
   - Write `bookx.flatpakrepo` (`[Flatpak Repo]`, `Title=Bookx`, `Url=https://bookx.adhadse.com/repo/`,
     `GPGKey=<base64 public key>`, `Homepage=`, `Icon=`). Users run
     `flatpak remote-add --user --if-not-exists bookx https://bookx.adhadse.com/bookx.flatpakrepo` and then
     `flatpak install --user bookx com.adhadse.Bookx`.
   - aarch64 needs its own build (a runner or QEMU) before it is listed.
3. **Cost of that route:** no Flathub verification badge or store listing, the owner runs the key and the hosting, and
   the machine-translated locales may be shipped there (the gate only matters for Flathub): turn the feature off for this channel.

