# Bookx on Windows (package managers)

Owner decision (2026-11): Windows comes **later**, through a package manager (**winget** and/or **Scoop**). A standalone
`.exe` installer download is **not** the primary channel. This page is the honest state of that work.

> **Nothing here has been built or run on Windows.** No Windows machine was available and the workspace has never
> been compiled for a Windows target. The Rust changes compile and pass on Linux only. Treat every Windows file as
> scaffolding until someone has run it.

## 1. Verdict

GTK 4 and libadwaita do run on Windows (MSYS2 UCRT64 packages, or gvsbuild), but the experience is rougher than on Linux
(fonts, IME, file dialogs and the dark-mode switch need testing). Bookx still has real Linux assumptions: audio output
(external `pacat`/`pw-cat`), media keys over D-Bus, GVfs devices, Unix-only code in a few crates, and a UI test suite that
does not run there. Effort to a first usable build: **L** (compile errors across crates, the audio sink, packaging).
Effort to winget/Scoop once there is a zip: **S**.

## 2. Gap list

| Area | Where | Linux-only thing | What Windows needs | Size | State |
|---|---|---|---|---|---|
| Passwords | `crates/bookx-secrets/src/lib.rs` | `oo7` | `NativeStore` on the Windows Credential Manager through the `keyring` crate (3.6, feature `windows-native`, which wraps `windows-sys` in safe code; our crate has no `unsafe`). Items are generic credentials of target `<key>.com.adhadse.Bookx`; the blob limit is 2560 bytes, enough for passwords, not for tokens with certificates | S | **Done, untested** |
| Device detection | `crates/bookx-serve/src/devices.rs` | `/run/media`, `/proc/mounts` | `windows_drive_roots`: present drive letters D to Z scanned as mounts (pure function with tests). Removable vs fixed disks (`GetDriveTypeW`) needs FFI, so fixed secondary drives also show up when they have a `.kobo` or `documents` marker | S | **Done, untested** |
| Free space | `crates/bookx-serve/src/devfs.rs:36` | `nix::sys::statvfs` | now `cfg(unix)`, returns `None` on Windows (no free-space line). `GetDiskFreeSpaceExW` needs `unsafe` FFI: use the `fs4`/`sysinfo` crate or a small non-forbid crate | S | gated |
| `nix` dependency | `crates/bookx-serve/Cargo.toml` | Unix only | moved to `[target.'cfg(unix)'.dependencies]`; `mtp.rs` `user_id()` gated | S | **Done** |
| TLS key file mode | `crates/bookx-serve/src/cert.rs:58` | `OpenOptionsExt::mode(0o600)` | gated: on Windows the key is written with default ACLs of the profile folder (private to the user in practice). Review for the security checklist before shipping a share server on Windows | S | gated |
| Other `std::os::unix` | `crates/bookx-backup/src/repo/source.rs:367`, `store.rs`, `crates/bookx-tts/src/download.rs:529,723` (`FileExt::read_at`/`write_at` -> `std::os::windows::fs::FileExt::seek_read`/`seek_write`), `bookx-tts/src/sink.rs:208` (`PermissionsExt`) | Unix | each needs a `cfg(windows)` branch; most are tests but `download.rs` and `sink.rs` are production code | M | open |
| Library lock | `crates/bookx-store/src/guard.rs:73-87,255-330` | `/proc` | host name via `COMPUTERNAME` env; `pid_alive` assumed true (stale lock after a crash is never taken over); no network-share detection | S | open |
| Read aloud sink | `crates/bookx-tts/src/sink.rs` | `pacat`, `pw-cat`, `aplay` | **none exist**: needs the `cpal` (WASAPI) sink, shared with audiobooks (`bookx-audio/src/player.rs`) | M | **blocker for read aloud and audiobooks** |
| ONNX Runtime | `crates/bookx-tts/src/catalog_data.rs`, `models.rs` | Linux builds only | add `win-x64` `onnxruntime.dll` entries with pinned SHA-256 | M | open |
| Media keys | `src/mpris.rs` | MPRIS over D-Bus (`zbus`) | no bus: feature is off. Real support: System Media Transport Controls through the `windows` crate (`unsafe`/COM: separate crate) | M | off |
| External tools | `ebook-convert`, `ebook-meta` (Calibre), `espeak-ng` | `PATH` | add `C:\Program Files\Calibre2` lookup; espeak-ng is bundled | S | open |
| Paths | `src/config.rs:503` (`directories`) | none | `%APPDATA%\adhadse\Bookx\config`, `%LOCALAPPDATA%`; audit the places that build `~/.config`/`~/.local/share` by hand | S-M | audit |
| Console window | `src/main.rs` | none | `#![cfg_attr(windows, windows_subsystem = "windows")]` added (no console behind the GUI); `bookx-cli` stays a console program | S | **Done** |
| `bookx://` and file types | `data/*.desktop`, `*.mime.xml` | freedesktop | registry keys (`HKCU\Software\Classes`): a winget portable package cannot register them; an MSIX or an in-app "register" action later | M | open |
| Long paths, reserved names, `\` | everywhere paths are built from book metadata | `/` | `sanitize_file_name` already removes reserved characters; add `CON`, `NUL`, `COM1`... and trailing dots; test the path join logic with `\` | M | open |
| File dialogs, DnD | GTK | portals | GTK's native dialogs on Windows | none | check |
| Test suites | `build-aux/ui-tests*.sh` (Broadway), session tests | Linux | not runnable; only `cargo test --workspace --lib` | M | open |

## 3. Packaging options, ranked

Constraint: no standalone `.exe` installer as the primary channel.

1. **winget, portable zip** (`packaging/winget/`, schema 1.10.0): `InstallerType: zip` with
   `NestedInstallerType: portable`, command alias `bookx`. Needs a zip with everything inside (GTK DLLs, schemas, icons,
   pixbuf loaders). The same zip is what Scoop uses, so one build serves both. Unsigned executables trigger
   SmartScreen and sometimes antivirus in the winget validation pipeline; code signing (a certificate, or Azure Trusted
   Signing) fixes that and is an owner decision. A portable package cannot create Start menu entries or register file
   types.
2. **Scoop** (`packaging/scoop/bookx.json`, own bucket `adhadse/scoop-bookx`): same zip, creates a Start-menu shortcut
   itself, `checkver`/`autoupdate` from GitHub releases; no signing needed. Lowest friction to publish.
3. **MSYS2 package** (`packaging/msys2/PKGBUILD`): for people who live in MSYS2; also the cleanest way to get the build
   dependencies right (the recipe is the build instructions). Upstreaming to `msys2/MINGW-packages` needs the project to
   be notable and to build in their CI.
4. **MSIX** (later): the only route to proper start menu entries, file types and URL handlers through winget without
   an `.exe` installer; needs a signing certificate. Not started.
5. **Chocolatey** (optional): a nuspec that points at the same zip; its moderation is slow and unsigned apps are
   scrutinised. Not started.

Build routes for the dependencies: **MSYS2 UCRT64** (`mingw-w64-ucrt-x86_64-gtk4`, `-libadwaita`, `-gtksourceview5`,
`-rust`; what `package-native.yml` uses) is the shorter path; **gvsbuild** builds GTK from source with MSVC and
is the route if the project wants the MSVC Rust target. The workflow uses MSYS2 and the `x86_64-pc-windows-gnu`
Rust target of that environment.

## 4. What was done on Linux

* `bookx-secrets`: `NativeStore` under `cfg(any(target_os = "macos", windows))`; the Linux build is unchanged (`oo7` is
  now a target dependency for the other systems).
* `bookx-serve`: `windows_drive_roots` and `macos_volume_roots` (pure, unit tested), `platform_scan`, direct mounts in
  `MassStorageDriver`, `nix` only on Unix, TLS key writer and free-space gated.
* `src/main.rs`: GUI subsystem on Windows.
* Files: `packaging/msys2/PKGBUILD`, `packaging/winget/…`, `packaging/scoop/…`, `build-aux/windows-bundle.sh`,
  `.github/workflows/package-native.yml` (manual dispatch only).
* Offline checks: YAML and JSON parse; shell scripts pass `bash -n`; the winget manifests were **not** run through
  `winget validate` (Windows only) and the PKGBUILD was **not** run through `makepkg`.

## 5. Owner: accounts and decisions

* A Windows machine or a manual run of `package-native.yml` to learn what breaks in the build.
* winget: a GitHub account to send the pull request to `microsoft/winget-pkgs` (no Microsoft account needed);
  code signing certificate (optional but strongly recommended).
* Scoop: the bucket repository `adhadse/scoop-bookx`.
* MSYS2: a maintainer account and agreement to maintain a recipe.
* Whether Windows ships without read aloud/audiobooks until the `cpal` sink exists.
* Windows-specific security review of the library share server (certificate key permissions, firewall prompt).
