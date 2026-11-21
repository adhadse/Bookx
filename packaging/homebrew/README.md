# Homebrew packaging for Bookx

Status: **untested scaffolding** (no macOS machine was available). Read `docs/macos-port.md` first: the
app has not been built on macOS yet, and read aloud / audiobooks need a native audio sink that does not exist yet.

## The tap

Homebrew taps are separate repositories named `homebrew-<name>`. Create `github.com/adhadse/homebrew-bookx`:

```
homebrew-bookx/
  Formula/bookx.rb        <- packaging/homebrew/bookx.rb   (build from source)
  Casks/bookx.rb          <- packaging/homebrew/Casks/bookx.rb   (signed .dmg)
  README.md
```

Users then run:

```bash
brew tap adhadse/bookx
brew install bookx                 # the formula: compiles with cargo (10-30 minutes), no Apple account needed
brew install --cask adhadse/bookx/bookx   # the cask: the .dmg, needs a notarized build
```

(`brew install --cask bookx` conflicts in name with the formula once both exist; pick one name per tap or
call the cask `bookx-app`. Decide before publishing.)

## Update procedure per release

1. Tag `vX.Y.Z` and wait for the release assets.
2. Formula: `url` to `https://github.com/adhadse/Bookx/archive/refs/tags/vX.Y.Z.tar.gz`, `sha256` from
   `curl -L <url> | shasum -a 256`.
3. Cask: `version` and the two `sha256` values of `Bookx-X.Y.Z-arm64.dmg` and `Bookx-X.Y.Z-x86_64.dmg`.
4. `brew audit --strict --new --online adhadse/bookx/bookx` and `brew test bookx`; `brew install --build-from-source bookx`
   on both Apple silicon and Intel. `brew bump-formula-pr` and `brew bump-cask-pr` can automate steps 2 and 3
   (or the `homebrew/actions` workflow in the tap).

## Why both

* The **formula** needs nothing from Apple, but compiles on the user's Mac and installs a command-line launcher
  (`bookx`) without an app icon in Launchpad. It is the realistic first target.
* The **cask** is the polished channel (a real `Bookx.app`, file associations, `bookx://`). It needs a Developer ID
  certificate and notarization (US$99 per year) and a macOS CI run that nobody has watched yet.
