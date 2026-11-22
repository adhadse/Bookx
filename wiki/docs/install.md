# Install

!!! warning "Beta"

    Bookx 0.3.0-beta.1 is beta software. It can have bugs, so keep a backup of anything you care about (Bookx can back up your library for you). Reports help us fix bugs: choose **Main Menu → Report a Problem**, read the log report, then **Open GitHub Issue**.

**Download Bookx from [GitHub Releases](https://github.com/adhadse/Bookx/releases/latest).** That is the official and,
for now, the only way to get Bookx: every release carries ready-made files for Linux that anybody can download, and each
file is signed so you can check that it is the one the project built ([Verify your download](#verify-your-download)).

Bookx is not on Flathub, Homebrew, winget or Scoop yet. Those may come later, once the project has a longer history of
releases; nothing is promised and there is no date. There are no macOS or Windows builds yet either.

## Download from GitHub Releases

Open the [latest release](https://github.com/adhadse/Bookx/releases/latest) and look under **Assets**. Each version
(here 0.3.0-beta.1) has these files:

| File | What it is | Choose it when |
|---|---|---|
| `bookx-0.3.0-beta.1-x86_64.flatpak` | A single-file Flatpak bundle (x86_64). Runs in a [sandbox](#what-the-sandbox-may-do) with its own GNOME 51 runtime | Any Linux distribution with [Flatpak](https://flatpak.org/setup/) installed. The safest choice and the one that works on older distributions |
| `bookx-0.3.0-beta.1.x86_64.rpm` | RPM package | Fedora 43 or newer, and other RPM distributions that ship GTK 4.20 and libadwaita 1.8 |
| `bookx_0.3.0-beta.1_amd64.deb` | Debian package | Debian, Ubuntu and derivatives that ship GTK 4.20 and libadwaita 1.8 (a recent release; older ones cannot install it) |
| `bookx-0.3.0-beta.1-linux-x86_64.tar.gz` | Plain archive with the same files under `usr/` | Arch and everything else; or when you want no package manager involved |
| `SHA256SUMS` | Checksums of all the files above | [Verifying](#verify-your-download) |
| `*.sigstore.json` | The signature of the file with the same name (also for `SHA256SUMS`) | [Verifying](#verify-your-download) |

The file names follow each package type (`bookx-<version>-<architecture>.<extension>`, `bookx_<version>_amd64.deb`); the examples below use 0.3.0-beta.1, use the version you downloaded. All builds are
for 64-bit Intel/AMD (x86_64); there is no ARM build yet.

### Which file do I pick?

| Your system | Pick |
|---|---|
| Fedora 43 or newer | the `.rpm` (or the Flatpak bundle) |
| Debian 13+, Ubuntu with GTK 4.20 and libadwaita 1.8 | the `.deb` (or the Flatpak bundle) |
| Older Debian or Ubuntu, Linux Mint, anything without GTK 4.20 | the Flatpak bundle |
| Arch, openSUSE, NixOS and others | the Flatpak bundle, or the tarball if your distribution already has GTK 4.20 and libadwaita 1.8 |
| You want the app sandboxed | the Flatpak bundle |
| You want Bookx to see all your files and devices without permissions | a native package (`.rpm`, `.deb`, tarball) |

Check your versions with `pkg-config --modversion gtk4 libadwaita-1` (or your package manager) when unsure. If a native
package does not install because a library is too old, use the Flatpak bundle.

### Install the file

Download the file **and** `SHA256SUMS` (and the `.sigstore.json` files if you verify signatures), then
[verify](#verify-your-download) before you install.

=== "Flatpak bundle"

    Bookx is **not on Flathub yet**, so you install the bundle file you downloaded from GitHub. Flatpak itself has to
    be installed first ([setup for your distribution](https://flatpak.org/setup/)); the bundle then needs the GNOME 51
    runtime, which comes from Flathub, so add Flathub once if you have not:

    ```bash
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    flatpak install --user ./bookx-0.3.0-beta.1-x86_64.flatpak
    flatpak run com.adhadse.Bookx
    ```

    Flatpak lists what it will fetch (the GNOME 51 runtime, a few hundred MB, once) and asks you to confirm. That
    download is the runtime, not Bookx. After the install Bookx is in your application menu. The bundle includes
    `espeak-ng` (voices for [read aloud](read-aloud.md)) and the command-line tool:
    `flatpak run --command=bookx-cli com.adhadse.Bookx --help`.

    **Updating:** a beta has no automatic updates. When a new beta is out, download its bundle and run the same
    `flatpak install --user ./bookx-<new version>-x86_64.flatpak` again: it replaces the old one and keeps your
    settings. Watch the [releases page](https://github.com/adhadse/Bookx/releases) (or use "Watch → Custom → Releases"
    on GitHub) to hear about new betas.

    **If it does not work:** `flatpak install` says "runtime not found" when Flathub is not added (run the first
    command above). `error: Flatpak system operation ... not allowed` means you left out `--user`. Start from a
    terminal with `flatpak run com.adhadse.Bookx` to see messages, and use Main Menu → Report a Problem.

=== "Fedora (.rpm)"

    ```bash
    sudo dnf install ./bookx-0.3.0-beta.1.x86_64.rpm
    ```

=== "Debian / Ubuntu (.deb)"

    ```bash
    sudo apt install ./bookx_0.3.0-beta.1_amd64.deb
    ```

=== "Tarball"

    The archive holds a tree starting at `usr/`. Unpack it into a prefix of your choice, for example your home folder:

    ```bash
    tar -xzf bookx-0.3.0-beta.1-linux-x86_64.tar.gz
    cp -r bookx-0.3.0-beta.1-linux-x86_64/usr/* ~/.local/
    ```

    Or system-wide (`sudo cp -r bookx-0.3.0-beta.1-linux-x86_64/usr/* /usr/`). You need GTK 4.20, libadwaita 1.8 and GtkSourceView
    5 installed; `espeak-ng` and `pacat`/`pw-cat` (PulseAudio or PipeWire) are optional for natural voices in
    [read aloud](read-aloud.md). The tree is the same one `make install` makes, so it contains the desktop file, icons, translations and
    `bookx-cli`.

## Update

Download the next release from [GitHub Releases](https://github.com/adhadse/Bookx/releases/latest) and install it the
same way; your library and settings stay where they are.

- Flatpak: `flatpak install --user ./bookx-0.3.0-beta.1-x86_64.flatpak` again replaces the old version.
- `.rpm` / `.deb`: `sudo dnf install ./new-file.rpm`, `sudo apt install ./new-file.deb` upgrades in place.
- Tarball: copy the new tree over the old one.

Bookx has **no automatic update check and never contacts GitHub by itself**: the app makes no network request you did not
ask for. Watch the releases (on GitHub: *Watch* → *Custom* → *Releases*, or the releases Atom feed
`https://github.com/adhadse/Bookx/releases.atom`) to hear about new versions. What changed is in the
[changelog](https://github.com/adhadse/Bookx/blob/main/CHANGELOG.md). Your version is shown in the main menu → *About Bookx* → *Troubleshooting*.

## Uninstall

- Flatpak: `flatpak uninstall --user com.adhadse.Bookx` (add `--delete-data` to remove its settings; your library folder is never deleted).
- `.rpm`: `sudo dnf remove bookx`; `.deb`: `sudo apt remove bookx`.
- Tarball: delete the files you copied (`bookx`, `bookx-cli`, the desktop file, icons, metainfo and `share/locale/*/LC_MESSAGES/bookx.mo`).

Your library folder (`~/Documents/Bookx Library` by default) holds your books and is **not** removed by any of these.

## Verify your download

Every file of a release (tarball, `.deb`, `.rpm`, Flatpak bundle and `SHA256SUMS`) is signed by the release workflow on
GitHub, with no key that anyone holds: the signature says "this file was built by the `release.yml` workflow of
`adhadse/Bookx` for a version tag", and is recorded in the public [Sigstore](https://www.sigstore.dev/) log. Each file has
a `.sigstore.json` file next to it (download it into the same folder). Release files are also covered by a GitHub build-provenance attestation.

1. Check the file against the checksums (catches a damaged download):

    ```bash
    sha256sum -c SHA256SUMS --ignore-missing
    ```

2. Check who built it, with [cosign](https://docs.sigstore.dev/cosign/system_config/installation/):

    ```bash
    cosign verify-blob \
      --bundle SHA256SUMS.sigstore.json \
      --certificate-identity-regexp '^https://github\.com/adhadse/Bookx/\.github/workflows/release\.yml@refs/tags/v' \
      --certificate-oidc-issuer https://token.actions.githubusercontent.com \
      SHA256SUMS
    ```

    Do the same with any single file (replace `SHA256SUMS`). It prints `Verified OK`; anything else means do not install it.

3. Or with the GitHub CLI, which checks the build-provenance attestation:

    ```bash
    gh attestation verify bookx-0.3.0-beta.1-linux-x86_64.tar.gz --repo adhadse/Bookx
    ```

If a check fails, do not install the file and report it as described in the
[security policy](https://github.com/adhadse/Bookx/blob/main/SECURITY.md). The source you build yourself is the same code
as the tag: compare `git rev-parse v0.3.0-beta.1` with the commit shown on the release page.


## What the sandbox may do

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

## Opening books and links

The package registers Bookx for EPUB, KEPUB, MOBI, AZW, AZW3, FB2, FBZ, CBZ, CBR, CBT, PDF, RTF, ODT, DOCX, LRF and CHM
(as an *Open With* choice; it does not take over PDF) and for `bookx://` [links](troubleshooting.md#bookx-links).
Text, HTML and Markdown files are not registered; add them from inside Bookx.

## Where things are stored

| What | Native package or tarball | Flatpak bundle |
|---|---|---|
| Library | `~/Documents/Bookx Library` (change it in main menu → *Preferences* → *Library*) | the same folder (the sandbox may use Documents) |
| Settings | `~/.config/bookx/config.json` | `~/.var/app/com.adhadse.Bookx/config/bookx/config.json` |
| Dictionaries | `~/.local/share/bookx/dictionaries/` | `~/.var/app/com.adhadse.Bookx/data/bookx/dictionaries/` |
| Plugins and patches | `~/.local/share/bookx/plugins/`, `~/.local/share/bookx/patches/` | `~/.var/app/com.adhadse.Bookx/data/bookx/plugins/`, `.../patches/` |

!!! note "Limitations"
    - Linux x86_64 only. There is no AppImage, no ARM build, and no Windows or macOS build. Flathub, Homebrew, winget and
      Scoop are not available (see [Why is Bookx not on Flathub?](#questions-and-answers)).
    - The packages have been tested on few distributions; if one does not install on yours, use the Flatpak bundle and
      [report it](troubleshooting.md#reporting-a-bug).
    - The Flatpak has been built and started on one machine only; devices and the natural voices are not tested inside
      the sandbox. The Flatpak translations are the built-in ones (see [Translating Bookx](translating.md)).
    - Older GNOME releases (before 49) are not supported when you build from source; the Flatpak needs the GNOME 51
      runtime (Flatpak downloads it for you).

## Build from source

Prefer this when you want the newest code, a distribution package that does not exist, or to contribute. Developers: the
workflow is in the [README](https://github.com/adhadse/Bookx#quick-start) and `_developer/docs.md`.

### Requirements

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

### Build and run

```bash
git clone https://github.com/adhadse/Bookx
cd Bookx
cargo run --release
```

### Install for your user

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

### Command-line tool

`bookx-cli` is built with the rest of the workspace and lands in `target/release/`:

```bash
cargo build --release -p bookx-cli
target/release/bookx-cli --help
```

See [Command line](command-line.md).

### Build the Flatpak yourself

The repository builds the same Flatpak for the **GNOME 51** runtime (`build-aux/com.adhadse.Bookx.json`):

```bash
flatpak install flathub org.gnome.Platform//51 org.gnome.Sdk//51 org.freedesktop.Sdk.Extension.rust-stable//26.08
BUNDLE=1 build-aux/flatpak.sh      # or: make flatpak
flatpak install --user bookx.flatpak
```

The build is offline: the crates come from `build-aux/cargo-sources.json`. It needs `flatpak-builder`.

## Running servers

Bookx can serve your library, sync and AI assistants on the network. For anything beyond one computer, run it behind a
reverse proxy: [Put Bookx behind a reverse proxy](reverse-proxy.md).

## Questions and answers

??? question "How do I install Bookx?"
    Download the file that fits your system from [GitHub Releases](https://github.com/adhadse/Bookx/releases/latest)
    ([which one?](#which-file-do-i-pick)), [verify it](#verify-your-download) and install it as shown in
    [Install the file](#install-the-file).

??? question "How do I update Bookx?"
    Download the next release and install it over the old one. There is no automatic update, and Bookx never contacts
    GitHub on its own. See [Update](#update).

??? question "Which file do I pick?"
    The Flatpak bundle works on any distribution with Flatpak. Use the `.rpm` on Fedora 43+ and the `.deb` on a recent
    Debian or Ubuntu if you want a native install. See [the table](#which-file-do-i-pick).

??? question "Is it safe to download and run?"
    Bookx is free software (GPL-3.0-or-later) whose source you can read. The release files are built by a public GitHub
    workflow from a version tag and signed with no key that a person holds; you can check that yourself. The Flatpak
    runs in a sandbox with the permissions listed [below](#what-the-sandbox-may-do). The project is young: back up your
    library before you let Bookx write to an e-reader.

??? question "How do I verify a download?"
    Run `sha256sum -c SHA256SUMS --ignore-missing` and `cosign verify-blob` (or `gh attestation verify`). The commands
    are in [Verify your download](#verify-your-download).

??? question "Why is Bookx not on Flathub (or Homebrew, winget, Scoop)?"
    The project is new and wants a track record first: several releases, real use and an issue tracker with answers. Package
    stores can follow later; no date is promised. Until then GitHub Releases is the one official place, which also means
    nobody else's repackaged build is endorsed by the project.

??? question "Is there a Windows or macOS version?"
    Not yet. Only Linux builds are made. Read aloud and audiobooks also depend on PulseAudio or PipeWire for sound.
