# Install

Bookx is not published as a package yet. You build it from source.

## Requirements

- GNOME 49 or newer: **GTK 4.20** and **libadwaita 1.8** or newer, and **GtkSourceView 5.6** or newer (the code view of the
  [book editor](editing.md)).
- A recent stable Rust toolchain.
- Optional: `espeak-ng` and `pacat`/`pw-cat` (PulseAudio or PipeWire) for natural voices in [read aloud](read-aloud.md);
  system Hunspell dictionaries for [spell checking](editing.md#spelling).

Install the build dependencies:

=== "Fedora"

    ```bash
    sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel gtksourceview5-devel pango-devel cairo-devel gdk-pixbuf2-devel gettext
    ```

=== "Debian / Ubuntu"

    ```bash
    sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev
    ```

=== "Arch"

    ```bash
    sudo pacman -S base-devel gtk4 libadwaita gtksourceview5
    ```

## Build and run

```bash
git clone https://github.com/adhadse/Bookx
cd Bookx
cargo run --release
```

## Install for your user

`run.sh` builds in release mode, installs the binary, desktop file, icons and metainfo into `~/.local`, and starts
the app:

```bash
./run.sh              # build, install to ~/.local, launch
./run.sh --no-run     # build and install only
./run.sh --debug      # faster build, run from target/, no install
```

Or install to a prefix of your choice:

```bash
make install PREFIX=~/.local
```

## Command-line tool

`bookx-cli` is built with the rest of the workspace and lands in `target/release/`:

```bash
cargo build --release -p bookx-cli
target/release/bookx-cli --help
```

See [Command line](command-line.md).

## Flatpak

The repository builds a Flatpak for the **GNOME 50** runtime (`build-aux/com.adhadse.Bookx.json`). Bookx is **not on
Flathub** yet, so you build it yourself, or download `bookx.flatpak` from a [release](https://github.com/adhadse/Bookx/releases)
once one is published.

=== "Build it"

    ```bash
    flatpak install flathub org.gnome.Platform//50 org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
    BUNDLE=1 build-aux/flatpak.sh      # or: make flatpak
    flatpak install --user bookx.flatpak
    flatpak run com.adhadse.Bookx
    ```

    The build is offline: the crates come from `build-aux/cargo-sources.json`, which
    `build-aux/flatpak-cargo-sources.py` writes from `Cargo.lock`. The script ends with a few smoke tests inside the
    sandbox. It needs `flatpak-builder`; without it only the manifest and the desktop files are checked.

=== "Install a downloaded bundle"

    ```bash
    sha256sum -c bookx.flatpak.sha256
    flatpak install --user bookx.flatpak
    ```

The Flatpak includes `espeak-ng` (voices for [read aloud](read-aloud.md)) and `bookx-cli`
(`flatpak run --command=bookx-cli com.adhadse.Bookx --help`).

### What the sandbox may do

| Permission | Why |
|---|---|
| Network | Fetching metadata, news, Wikipedia and translation, OPDS catalogues, WebDAV backups, e-mail, KOReader progress sync, the library share server, model downloads for read aloud |
| Your **Documents** folder | The default library, `~/Documents/Bookx Library` |
| `/run/media` | E-readers and drives that the desktop mounts (Kobo, Kindle, USB sticks) so books can be sent, read back and removed |
| `xdg-run/gvfs` and the GVfs service | Phones and e-readers that use MTP, network shares and cloud drives mounted by GNOME Files |
| PulseAudio / PipeWire | Read aloud |
| Wayland (or X11), GPU | The window |

Other folders (a library elsewhere, a backup folder, export targets) are reached through the file chooser, which hands
the app that one folder. A library outside Documents has to be chosen again after a restart, or you can grant it:
`flatpak override --user --filesystem=/path/to/library com.adhadse.Bookx`. Passwords go to the desktop's secret
service through the portal; they never sit in a file. Accessibility needs no extra permission: Flatpak connects the
accessibility bus by itself.

### Opening books and links

The package registers Bookx for EPUB, KEPUB, MOBI, AZW, AZW3, FB2, FBZ, CBZ, CBR, CBT, PDF, RTF, ODT, DOCX, LRF and CHM
(as an *Open With* choice; it does not take over PDF) and for `bookx://` [links](troubleshooting.md#bookx-links).
Text, HTML and Markdown files are not registered; add them from inside Bookx.

## Where things are stored

| What | Where |
|---|---|
| Library | `~/Documents/Bookx Library` (change it in main menu → *Preferences* → *Library*) |
| Settings | `~/.config/Bookx/config.json` |
| Dictionaries | `~/.local/share/bookx/dictionaries/` |
| Plugins and patches | `~/.local/share/bookx/plugins/`, `~/.local/share/bookx/patches/` |

!!! note "Limitations"
    - Release files (tarball, `.deb`, `.rpm`, Flatpak bundle, `SHA256SUMS`) are attached to each GitHub release once a version is
      tagged; there is no AppImage and no Flathub listing yet, and no Windows or macOS builds. Check a download with
      `sha256sum -c SHA256SUMS`. The packages have not been tested on other distributions than the build machine's.
    - The Flatpak has been built and started on one machine only; devices and the natural voices are not tested inside
      the sandbox. The Flatpak translations are the built-in ones (see [Translating Bookx](translating.md)).
    - Older GNOME releases (before 49) are not supported when you build from source; the Flatpak needs the GNOME 50
      runtime.

## Running servers

Bookx can serve your library, sync and AI assistants on the network. For anything beyond one computer, run it behind a
reverse proxy: [Put Bookx behind a reverse proxy](reverse-proxy.md).
