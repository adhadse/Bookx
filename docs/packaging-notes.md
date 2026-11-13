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
* Not done: AppImage, deb/rpm, update notice, Flathub submission (needs a tagged release URL + hash instead of the `dir` source,
  `x-checker-data` for the crates, screenshots on a stable URL), reproducible-build check, aarch64.

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
  `app.copy-diagnostics` (main menu, palette). Safe mode and a debug-log export (X19) are **not done**.
* `src/links.rs`: `bookx://open?id=N`, `bookx://book/<identifier>`; `app.open-uri` action; a second `bookx` process forwards links
  to the running one (`main.rs`) and exits. Link forwarding was exercised only through the `open-uri:` dev command, never from a
  real desktop launcher.
* `data/com.adhadse.Bookx.desktop` registers 16 media types + `x-scheme-handler/bookx`; `data/com.adhadse.Bookx.mime.xml` defines
  KEPUB and AZW; `src/desktop_integration.rs` tests that desktop file, metainfo, MIME file, icon and manifest agree, and fails when
  a new readable extension is neither listed nor deliberately left out.
* Opening a file from the file manager while Bookx runs still starts a second process that imports the file (GApplication runs
  with empty args); only `bookx://` links are forwarded.
