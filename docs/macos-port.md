# Bookx on macOS (Homebrew)

Owner decision (2026-11): macOS is distributed through **Homebrew** (first class): a formula that builds from source and
a cask with a `.dmg`. This page is the honest state of that work.

> **Nothing in this port has been run on macOS.** No Mac was available. The Rust changes compile and pass on Linux only
> (`cargo test --workspace`); the macOS-only code paths (`cfg(target_os = "macos")`), the Homebrew files, the bundle
> script and the workflow are **untested**. Do not announce macOS support before someone has built and used it on a Mac.

## 1. Verdict

GTK 4 and libadwaita run on macOS (Homebrew `gtk4`, `libadwaita`, `gtksourceview5` exist; the GTK macOS backend is
supported but less polished than Wayland). The workspace is mostly portable Rust, so a build is realistic. What is
**not** portable today: audio output (the biggest gap), media keys, a few Linux-only helpers, the ONNX Runtime
download catalogue, and everything around packaging (a `.app`, signing, notarization). Estimated effort to a usable
0.x: **L overall** (mostly audio and testing on real hardware), the Homebrew formula alone is **S** once it builds.

## 2. Gap list per crate

Size: S under a day, M a few days, L a week or more. "Done" means a Linux-safe change is already in the tree.

| Area | Where | Linux-only thing | What macOS needs | Size | State |
|---|---|---|---|---|---|
| Passwords | `crates/bookx-secrets/src/lib.rs` | `oo7` (Secret Service, Flatpak portal) | `NativeStore` on the Keychain through the `keyring` crate (3.6, feature `apple-native`, which uses `security-framework`). The crate has no `unsafe`. `oo7`/`async-io` are now target dependencies for non-macOS/non-Windows only. `open_default()` picks the store by `cfg`. | S | **Done, untested**. Check that the first access shows the Keychain prompt once and not per read; keys are stored as service `com.adhadse.Bookx`, account = key |
| Device detection | `crates/bookx-serve/src/devices.rs` | `/run/media`, `/media`, `/mnt`, `/proc/mounts` | `platform_scan()`: on macOS the entries of `/Volumes` (`macos_volume_roots`, a pure function with tests) are scanned as mounts; `MassStorageDriver` has a new `mounts` list. `/proc/mounts` is simply absent (skipped). | S | **Done, untested on a Mac** (the pure parts are unit tested) |
| MTP / GVfs | `crates/bookx-serve/src/mtp.rs:30` (`/run/user/<uid>/gvfs`) | GVfs | No MTP stack on macOS. The path does not exist, so MTP devices are never listed; check nothing logs an error. Android phones need Android File Transfer, out of scope. | S | check |
| Free space | `crates/bookx-serve/src/devfs.rs:36` | `nix::sys::statvfs` | `nix` supports macOS (`statvfs`, `getuid`), no change | none | portable |
| Library lock | `crates/bookx-store/src/guard.rs:73-87` | `/proc/sys/kernel/hostname`, `/proc/<pid>` | host name falls back to `$HOSTNAME` ("this computer" when unset: macOS does not export it); `pid_alive` returns `true` on non-Linux, so a stale lock of a crashed Bookx is never taken over. Use `nix::unistd::gethostname` and `nix::sys::signal::kill(pid, None)`. | S | open; behaviour is safe but annoying |
| Mount type of a path | `crates/bookx-store/src/guard.rs:255-330` (`/proc/self/mountinfo`) | detects network file systems for the lock | `getfsstat`/`statfs` through `nix` on macOS | S | open; without it the "library is on a network share" warning is silent |
| Backup places | `crates/bookx-backup/src/places.rs` (`/proc/mounts`, `/run/media`, `$XDG_RUNTIME_DIR/gvfs`) | removable drive and network mount detection | list `/Volumes`, `statfs` type for `smbfs`/`nfs`/`webdav` | M | open; destinations still work through a chosen folder |
| Read aloud sink | `crates/bookx-tts/src/sink.rs:59,75,151` | pipes PCM to `pacat`, `pw-cat`, `aplay` | **No such player on macOS** (`afplay` plays files, not a stream). Needs a native sink: `cpal` (CoreAudio) behind the existing sink trait; the roadmap already lists it (`docs/tts-notes.md`). Without it the app says "no audio player found". | M | **blocker for read aloud** |
| Audiobook player | `crates/bookx-audio/src/player.rs` (same players) | same | same `cpal` sink; decoding (Symphonia) and stretch are portable | M (shared with the above) | **blocker for audiobooks** |
| ONNX Runtime download | `crates/bookx-tts/src/catalog_data.rs:6-11`, `models.rs:222-320` | only `linux-x64` and `linux-arm64` entries, `libonnxruntime.so` | add `osx-arm64` / `osx-x86_64` builds with pinned size and SHA-256 (`libonnxruntime.dylib`), `RUNTIME_LIB` per OS; or ship the dylib in the `.app` | M | open |
| Speech phonemes | `espeak-ng` (command) | needs the program and `ESPEAK_DATA_PATH` | formula depends on `espeak-ng`; `macos-bundle.sh` bundles binary and data | S | scaffolded |
| Media keys | `src/mpris.rs` (`zbus::blocking`, session bus) | MPRIS over D-Bus | `Mpris::start` returns `None` without a bus, so the feature is off, not broken (verify: the `DBUS_SESSION_BUS_ADDRESS` lookup must not hang). Real support: `MPRemoteCommandCenter`/`MPNowPlayingInfoCenter` through the `objc2` crates (a new dependency and some `unsafe` that must live in a separate crate, not in a `forbid(unsafe_code)` one). | M | off; verify no hang |
| File associations, `bookx://` | `data/*.desktop`, `data/*.mime.xml`, `src/desktop_integration.rs` | freedesktop MIME | `Info.plist` `CFBundleDocumentTypes`/`CFBundleURLTypes` (written by `macos-bundle.sh`). Opening a file from Finder arrives as a GApplication `open`; check that `src/main.rs` handles it the same way. | S | scaffolded |
| Paths | `src/config.rs:503` (`directories::ProjectDirs("com","adhadse","Bookx")`) | none | gives `~/Library/Application Support/com.adhadse.Bookx`. User-facing docs say `~/.config/Bookx` and `~/.local/share/bookx`: some code uses the XDG variables directly (`src/tts.rs`, `src/editor_prefs.rs`, `src/diagnostics.rs`, `src/plugins.rs`, `src/editor_spell_dicts.rs`): audit that each goes through `directories`. | S-M | audit |
| Spell check dictionaries | `src/editor_spell_dicts.rs` | `/usr/share/hunspell` | add `$(brew --prefix)/share/hunspell` and `~/Library/Spelling` | S | open |
| Calibre tools | `ebook-convert`, `ebook-meta`, `calibredb` through `PATH` | `PATH` | Calibre on macOS lives in `/Applications/calibre.app/Contents/MacOS/`; add that to the lookup | S | open |
| `std::os::unix` code | `crates/bookx-serve/src/cert.rs:58,101`, `bookx-backup/src/repo/*.rs`, `bookx-tts/src/download.rs:529,723`, `bookx-tts/src/sink.rs:208` | Unix permissions, `FileExt` | all work on macOS | none | portable |
| Dialogs, portals | `gtk::FileDialog`, `show_uri` | xdg-desktop-portal | GTK uses its own (native `NSOpenPanel`) dialogs without portals | none | check |
| Dark mode | libadwaita `AdwStyleManager` | settings portal | libadwaita follows the macOS appearance only with a recent GTK; check on 4.20+ and otherwise offer the existing theme setting | S | check |
| Packaging | `build-aux/macos-bundle.sh`, `.github/workflows/package-native.yml`, `packaging/homebrew/` | n/a | `.app` bundle: dylibbundler, gdk-pixbuf loaders cache, schemas, icons, launcher script, `Info.plist`, icns, codesign, notarize, `create-dmg` | M-L | scaffolded, **never run** |
| Test suites | `build-aux/ui-tests*.sh` (Broadway), `session-tests.sh` (GNOME Shell) | Linux | none of the UI scenario infrastructure runs on macOS; only `cargo test` (the pure logic) can. Add a macOS job to `ci.yml` that runs `cargo test --workspace` (the test owner edits `ci.yml`). | M | open |

What had to be gated with `cfg`: the Keychain store (`cfg(any(target_os = "macos", windows))` in `bookx-secrets`), the
Secret Service store (`cfg(not(any(macos, windows)))`), the device scan (`cfg!(target_os = "macos")` in
`platform_scan`). Still to gate when someone builds on a Mac: `src/mpris.rs` (a no-op stub keeps `zbus` out of the
macOS build), the TTS sinks (`pacat` and friends), the ONNX catalogue.

## 3. The two Homebrew channels

| | Formula (`packaging/homebrew/bookx.rb`) | Cask (`packaging/homebrew/Casks/bookx.rb`) |
|---|---|---|
| What the user gets | command-line launcher `bookx`, built on their Mac with `cargo install` | `Bookx.app` from a `.dmg`, Launchpad icon, file associations |
| Needs from the owner | nothing from Apple | Apple Developer ID certificate (US$99/year), notarization, a working `package-native.yml` |
| Gatekeeper | irrelevant (built locally) | **Homebrew is dropping casks that fail Gatekeeper**, so notarization is required (check the current policy when publishing) |
| Build time for the user | long (cargo, 10-30 min) | none |
| Risk | the dependency stack (gtk4, libadwaita 1.8+, gtksourceview5, espeak-ng) must be new enough in Homebrew: formula versions are not checked here | the bundle script is the riskiest file: schemas, pixbuf loaders, dylib paths |

Recommended order: get `cargo build --release` working on a Mac, then the formula (tap `adhadse/homebrew-bookx`, see
`packaging/homebrew/README.md`), then the audio sink, then the `.app`/cask.

## 4. Owner: accounts and decisions

* A Mac (or a macOS runner session) to do the first build; the workflow runs only on manual dispatch.
* The tap repository `github.com/adhadse/homebrew-bookx`.
* For the cask: Apple Developer Program membership, a **Developer ID Application** certificate exported as `.p12`,
  an app-specific password for `notarytool`; GitHub secrets `APPLE_CERT_P12_BASE64`, `APPLE_CERT_PASSWORD`,
  `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` (names used by the workflow).
* A decision on read aloud / audiobooks on macOS: ship without them until the `cpal` sink exists, or make the sink first.
* Name clash: a formula and a cask both called `bookx` in one tap (see the tap README).

## 5. What was verified here

* `cargo test -p bookx-secrets -p bookx-serve` on Linux (the pure functions `macos_volume_roots`,
  `windows_drive_roots`, the direct-mount scan of `MassStorageDriver`, and `system_store_name`).
* `cargo check -p bookx-secrets --target aarch64-apple-darwin` passes (type-checks `NativeStore` against `keyring`/`security-framework`).
  Nothing was linked or run, and the other crates were not checked for that target.
* Not verified: any Homebrew file (`brew audit`), the bundle script, the workflow (not even through `actionlint`).

## First run of `package-native.yml` (macos-arm64), 2026-11-22

* `macos-14` is refused by GitHub ("configuration not supported"): the workflow uses `macos-15` (Apple silicon) and
  `macos-15-intel`.
* On `macos-15`, with the Homebrew packages of the workflow, the app and `bookx-cli` **compile** (13 minutes cold).
* `build-aux/macos-bundle.sh` first hung for 48 minutes until the job was cancelled: `dylibbundler` could not find
  `@rpath/librsvg-2.2.dylib` (the gdk-pixbuf SVG loader) and asked for a path on stdin, for ever. It now gets the Homebrew search
  paths (`-s`, including librsvg) and stdin is closed, so a missing library fails the step.
* With that, run 45023832637 produced the `.dmg` (15 minutes). **Not tested**: that the app starts from the `.dmg`
  (the runner has no display), signing and notarization (no Apple secrets), the Intel build.
