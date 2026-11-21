# Packaging, translations and diagnostics: internal notes

Public pages: `wiki/docs/install.md`, `translating.md`, `troubleshooting.md`. This file is for caveats and status.

## Flatpak (X7, X16)

* `build-aux/com.adhadse.Bookx.json`: GNOME **50** Platform/SDK (49, 50 and 51 Platforms were installed; 50 has Platform and SDK
  both, 51 had no SDK here), `org.freedesktop.Sdk.Extension.rust-stable//25.08` (Rust 1.98, `ort` needs ≥ 1.92).
* Offline: `build-aux/flatpak-cargo-sources.py` writes `build-aux/cargo-sources.json` from `Cargo.lock` (551 crates, sha256 from
  the lock, `.cargo-checksum.json` per crate, `cargo/config` replacing crates.io). `flatpak.sh` regenerates and checks it; CI runs
  `--check`. **Regenerate after every dependency change.** Git dependencies are refused by the script.
* Modules: `espeak-ng` 1.52.0 (autotools, no pcaudiolib/sonic/mbrola; GPL-3.0-or-later, run as a separate program) and `bookx`
  (`cargo --offline build --release --locked -p bookx -p bookx-cli`, then `make install-flatpak PREFIX=/app`).
* `build-aux/flatpak.sh` built it for real on 2026-11-13 (flatpak-builder 1.4.12, ~12 minutes cold) and ran: `bookx-cli --help`,
  `espeak-ng --ipa`, the installed desktop/metainfo/MIME/icon files, and the app on Broadway inside the sandbox with a scratch
  library (`diag:` printed the report: Flatpak, GTK 4.22.5, libadwaita 1.9.4). **Not tested in the sandbox:** devices (`/run/media`,
  MTP through GVfs), natural voices (ONNX Runtime is downloaded by the user into the data dir; `pacat` availability in the runtime
  is assumed), the secrets portal, backups to network places, `bookx://` forwarding, file-manager "Open With".
* Permissions and why (`finish-args`): `--share=network` (metadata, news, OPDS, WebDAV, mail, KOSync, share server, model
  downloads), wayland/fallback-x11/dri (window), `--socket=pulseaudio` (read aloud pipes PCM to `pacat`/`pw-cat`),
  `--filesystem=xdg-documents` (default library
  `~/Documents/Bookx Library`), `--filesystem=/run/media` (mounted e-readers and drives), `--filesystem=xdg-run/gvfs`,
  `xdg-run/gvfsd` and `--talk-name=org.gtk.vfs.*` (MTP phones, network shares, cloud drives mounted by GNOME Files). Not granted:
  home, host, `org.freedesktop.secrets` (oo7 uses the Secret portal in a sandbox), a11y bus (Flatpak proxies it itself). A test
  (`desktop_integration`) and `flatpak.sh` fail on `--filesystem=host|home` and `--device=all`.
* Model downloads (read aloud): the voice models and `libonnxruntime.so` go to `$XDG_DATA_HOME/bookx/tts` (`~/.var/app/com.adhadse.Bookx/data/bookx/tts` in the sandbox), which is writable without any extra permission; `BOOKX_TTS_DIR` overrides it. Nothing from the models is bundled in the Flatpak.
* Translations: the `.po` files are embedded in the binary, so the Flatpak needs no locale extension. `make install` also
  installs compiled `.mo` files (`msgfmt`) for system packages; inside a Flatpak the `.Locale` extension moves them out of
  `/app/share/locale`, which is fine because the embedded text is used.
* Not done: AppImage, update notice, Flathub submission (needs a tagged release URL + hash instead of the `dir` source,
  `x-checker-data` for the crates, screenshots on a stable URL), a reproducible-build check of the binaries, aarch64.

## CI and release workflows

`.github/workflows/ci.yml` (Fedora container: fmt, clippy `-D warnings`, `cargo test --workspace --locked`, desktop/AppStream/MIME
validation, i18n tests, cargo-sources check, mockups, Broadway UI suite when `gtk4-broadwayd` exists; session tests never run in
CI; Flatpak job with the static checks and a bundle) and `release.yml` (tag `v*`: release build, tarball + sha256, Flatpak bundle,
`SHA256SUMS`, GitHub release). Both were parsed with PyYAML; **neither has run on GitHub**. The `ghcr.io/flathub-infra/
flatpak-github-actions:gnome-50` image name is an assumption from the `gnome-49` pattern.

## Translations (X6, X17)

* `src/i18n.rs`: `tr`, `tr_c` (context), `trf` (`{}` placeholders), `ntr` (plural, `{}` = the count with the language's
  grouping), `N_` (marks a constant). Catalogue = gettext raw form (`ctx\x04id\0plural` → forms joined by `\0`), so `.mo` files
  work unchanged; `Plural-Forms` header → `OneOther` (n != 1), `ZeroOneOther` (n > 1), `Single`.
* `po/bookx.pot` is sorted and written by `build-aux/i18n.sh`; tests fail when it is stale, when es/de miss a string, when a
  placeholder or plural form is wrong, when fr/hi have fewer than 150 strings.
* Marking was done with scripts that rewrite widget calls (`set_title("…")`, `.title("…")`, `Button::with_label`, toasts, menu
  `append`, `add_response(s)`, helper functions such as `row(`/`button(`/`item(`), `format!` templates (→ `trf`), and the
  `if n == 1 { "" } else { "s" }` idiom (→ `ntr`). **What is still English** (about 700 literals by a rough heuristic, many of
  them not user text): composed sentences in `device_sync.rs`, `preflight.rs` check texts, `export.rs` format labels, enum
  `label()` tables returning `&'static str` (tap zones, layout names, encryption, plugin permission kinds), the language-name
  lists (`web.rs`, `tweaks.rs`), the Pango markup built from fragments, `Err(anyhow!(..))` messages that surface in toasts, the
  CLI, reader text-metric labels, editor report bodies. New code should call `tr`/`trf`/`ntr` itself.
* es and de are machine-assisted for every marked string and need native review (header says so); fr and hi cover the 191 most
  visible strings (menus, buttons, preferences sections, reader and library vocabulary, plurals).
* Dates: `i18n::fmt_when` (used by `library_health::when`) and `fmt_date`; numbers `fmt_int`, `ntr`. Most other dates are still
  strftime English (`%d %b %Y` in the details panel, news, kosync).
* RTL: `language_settings::apply_direction` sets the GTK default direction for right-to-left languages (`ar he fa ur yi`); there is
  no shipped RTL catalogue, so this has unit tests only (`is_rtl`) and no UI scenario.
* Language switching: windows opened afterwards use it, the main window and menus after a restart (the row says so). Live
  re-translation of an open window was not attempted.
* No Weblate project yet; files follow its defaults.

## Diagnostics and links (X19, X20)

* `src/diagnostics.rs`: `Facts`, `render`, `redact` (home → `~`, user name, e-mail, URL credentials, `password|token|key: …`, IPv4,
  MAC, 28+ character tokens; the application id is kept). Shown in About → Troubleshooting (`debug_info`), copied by
  `app.copy-diagnostics` (main menu, palette). Wave 11: the debug log (`src/debug_log.rs`: in-memory ring of the last 400 tracing lines via `Tee`, `scrub` removes library folder, titles (longest 5000), exact secret values, then `diagnostics::redact`) is the About → Troubleshooting `debug_info` (the AdwAboutDialog gives Save and Copy). Safe mode (`src/safe_mode.rs`): `--safe-mode` or `BOOKX_SAFE_MODE=1`; plugins/patches dirs replaced by empty temp dirs, `privacy::apply(true)` (offline), MCP/share/KOSync not started, `Config::load` returns defaults and `Config::save` is a no-op. Other files (editor.json, vocabulary, …) are still written in safe mode; the library is opened as usual.
* `src/links.rs`: `bookx://open?id=N`, `bookx://book/<identifier>`; `app.open-uri` action; a second `bookx` process forwards links
  to the running one (`main.rs`) and exits. Link forwarding was exercised only through the `open-uri:` dev command, never from a
  real desktop launcher.
* `data/com.adhadse.Bookx.desktop` registers 16 media types + `x-scheme-handler/bookx`; `data/com.adhadse.Bookx.mime.xml` defines
  KEPUB and AZW; `src/desktop_integration.rs` tests that desktop file, metainfo, MIME file, icon and manifest agree, and fails when
  a new readable extension is neither listed nor deliberately left out.
* Wave 11: `src/launch.rs` parses links, files, `--doc FILE` and `--edit`; the second process forwards links (`app.open-uri`), files (`app.open-files`, absolute paths) and documents (`app.open-doc`) and exits. `--doc` imports synchronously if the format is EPUB or importable and opens the reader, else falls back to the import. Tested with the real second process through the `launch:spawn` dev hook in Broadway scenarios `launch_forward_doc` and `launch_forward_open_with` (real GApplication on the scenario's D-Bus session). Not tested: a real desktop launcher / file manager (needs the real-session suite).

## Release pipeline (wave 6, PP-C-19)

What a release is, in order (nothing publishes from a developer machine; nobody pushes tags from scripts):

1. Bump the version in `Cargo.toml` and every `crates/*/Cargo.toml`, add a `## [x.y.z] - date` section to `CHANGELOG.md` (hand written,
   Keep a Changelog) and a `<release>` to `data/com.adhadse.Bookx.metainfo.xml`.
2. `make release-check` (`build-aux/release-check.sh [vX.Y.Z]`): versions agree across all crates, metainfo and changelog, the tag matches,
   `cargo-sources.json` matches `Cargo.lock`, golden plugin files unchanged, `desktop-file-validate`, `appstreamcli validate --no-net`,
   `xmllint` on the MIME file, deb/rpm asset sources exist, workflow YAML parses. A missing tool is printed as *skipped*, never as passed.
   The same version/asset logic is a unit test (`src/release_consistency.rs`, runs in `cargo test -p bookx`).
3. The maintainer tags and pushes `vX.Y.Z`. `.github/workflows/release.yml` then runs `release-check.sh` with the tag, builds with
   `--locked`, runs `build-aux/tarball.sh` (tarball + `.sha256`), builds `.deb` (`cargo-deb`) and `.rpm` (`cargo-generate-rpm`) from the
   `[package.metadata.deb]` / `[package.metadata.generate-rpm]` tables in the root `Cargo.toml`, builds the Flatpak bundle in the
   `flatpak-github-actions` GNOME 50 image, and attaches everything plus `SHA256SUMS` to a GitHub release. Release notes are generated by
   GitHub from the merged PRs; `CHANGELOG.md` is the curated text.
4. `deny.toml` + the `deny` CI job: `cargo deny check advisories bans licenses sources`.

Verified locally on 2026-11-20 (Fedora, flatpak-builder 1.4.12): `release-check.sh` passes (actionlint missing: skipped);
`build-aux/flatpak.sh` built the whole Flatpak from a `git archive HEAD` snapshot (GNOME 50 + rust-stable 25.08) and its smoke tests passed
(`bookx-cli`, `espeak-ng`, installed desktop/metainfo/MIME/icon, app starts on Broadway in a scratch library); `build-aux/tarball.sh`
twice with the same binaries gives a byte-identical `.tar.gz` (same SHA-256); `cargo deny` clean (see `docs/security.md`).
**Not verified:** the GitHub workflows (parsed with PyYAML only, never ran; the `gnome-50` flatpak-github-actions image name and the
`EmbarkStudios/cargo-deny-action@v2` / `softprops/action-gh-release@v2` inputs are from memory), `cargo deb` and `cargo generate-rpm`
(neither tool is installed here; the tables are only checked by name and by file existence), installing the `.deb`/`.rpm` on a real
Debian/Fedora, `dpkg-shlibdeps` style dependency discovery (the deb lists libraries by hand: `libc6`, GTK 4.20, libadwaita 1.8,
GtkSourceView 5), the `.rpm`/tarball built on Fedora's glibc may not run on older distributions.

* **Runtime:** the manifest uses GNOME **50**. GNOME 51 is current; its Platform is installed here but its SDK is not, so moving the
  manifest (and `gnome-50` image names in the workflows) to 51 was not tested. Do it with `flatpak install flathub org.gnome.Sdk//51` and
  rerun `build-aux/flatpak.sh`.
* **GtkSourceView 5** comes from the runtime (no module). **espeak-ng 1.52.0** is built from source (module). **ONNX Runtime** is not
  bundled: it is downloaded on first use of a natural voice into `$XDG_DATA_HOME/bookx/tts` together with the voice model (hash-checked,
  resumable), which needs no extra permission.
* **Offline crates:** `build-aux/flatpak-cargo-sources.py` (ours, no external generator) writes `cargo-sources.json` from `Cargo.lock`;
  `cargo vendor` is not needed. Regenerate it after every dependency change (the release check and CI fail otherwise).
* **Icons:** the application icon is the scalable SVG plus a symbolic SVG (GNOME convention; Flathub renders the PNG sizes itself). No
  pre-rendered PNG sizes are shipped: `rsvg-convert`/Inkscape are not available here to produce and verify them. Add them if a
  distribution asks for PNGs.
* **Reproducibility:** `Cargo.lock` is committed and builds use `--locked`; the Flatpak has pinned crate hashes and an `espeak-ng` archive
  hash; `tarball.sh` packs deterministically (sorted names, uid 0, `SOURCE_DATE_EPOCH` or the HEAD commit time, `gzip -n`). Rust binaries are
  *not* checked for bit-for-bit reproducibility between two builds (paths are embedded unless `RUSTFLAGS=--remap-path-prefix` is used);
  that is open. Releases are not signed; users verify `SHA256SUMS`.
* **AppImage:** not provided. A GTK 4.20 + libadwaita 1.8 AppImage needs a bundled GTK stack (linuxdeploy plus a GTK plugin), which was
  not attempted; the Flatpak is the portable package.
