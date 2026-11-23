# Arch User Repository (AUR)

Two packages, both for x86_64:

| Package | What | When to use |
|---|---|---|
| `bookx-bin` | the prebuilt tarball of the GitHub release | fast install, no compiler |
| `bookx` | builds the release tag from source (`cargo`, about 10 minutes) | when you want it built on your system |

`bookx-git` (a `-git` package for the newest `main`) is not offered: `main` is not a release.

Both are generated from the release version: `_pkgver` in each `PKGBUILD` is the Cargo version (`0.3.0-beta.2`), `pkgver`
drops the hyphens (`0.3.0beta.2`, which Arch sorts before `0.3.0`). `build-aux/release-check.sh` fails when `_pkgver`
differs from `Cargo.toml`, so a release cannot go out with a stale recipe.

## Update for a release (after the GitHub release exists and is verified)

```bash
v=0.3.0-beta.3                                    # the new version
sed -i "s/^_pkgver=.*/_pkgver=$v/; s/^pkgrel=.*/pkgrel=1/" packaging/aur/bookx/PKGBUILD packaging/aur/bookx-bin/PKGBUILD
cd packaging/aur/bookx-bin && updpkgsums && makepkg --printsrcinfo > .SRCINFO     # `bookx`: same
```

`updpkgsums` (package `pacman-contrib`) downloads the file and writes its sha256. For `bookx-bin` the number must equal the
line of `bookx-<version>-linux-x86_64.tar.gz` in the release's `SHA256SUMS`: compare them. Test in a clean chroot
(`extra-x86_64-build` from `devtools`) or a container before pushing.

## Publish (one time, by the owner: it needs the owner's AUR account)

1. Create an account at <https://aur.archlinux.org>, add an SSH public key in *My Account*.
2. For each package name (`bookx`, `bookx-bin`; the names are free as of 2026-11-23):

```bash
git clone ssh://aur@aur.archlinux.org/bookx-bin.git /tmp/aur-bookx-bin     # an empty repository is created on the first push
cp packaging/aur/bookx-bin/{PKGBUILD,.SRCINFO} /tmp/aur-bookx-bin/
cd /tmp/aur-bookx-bin && git add PKGBUILD .SRCINFO && git commit -m "bookx-bin 0.3.0beta.2" && git push
```

Later releases: the same copy, commit and push. Automating this needs an SSH private key stored as a repository secret
(`AUR_SSH_PRIVATE_KEY`); not set up, so it is a manual step after each release (a minute).

Mention the AUR on the install page (`wiki/docs/install.md`, row "Arch") only after the first push succeeded.
