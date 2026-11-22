# winget manifests for Bookx

Status: **untested scaffolding**. Nobody has built or run Bookx on Windows (`docs/windows-port.md`); the `0.3.0` values,
the release date and the checksum are placeholders.

Package identifier `adhadse.Bookx`, installer type: a **portable zip** (`InstallerType: zip`,
`NestedInstallerType: portable`): winget extracts `Bookx-<version>-windows-x64.zip` and links
`Bookx\bin\bookx.exe` as the command `bookx`. No `.exe` installer download is involved.
The zip is built by `.github/workflows/package-native.yml` (manual dispatch).

## Per release

1. Upload the zip to the GitHub release, compute `sha256sum` and put it into `*.installer.yaml`.
2. Copy the directory to a new version folder, update `PackageVersion` in all three files, `InstallerUrl`, `ReleaseDate`.
3. On Windows: `winget validate --manifest packaging\winget\manifests\a\adhadse\Bookx\<version>` and a local test,
   `winget settings --enable LocalManifestFiles` then `winget install --manifest <folder>`.
4. Submit with `wingetcreate submit` or a pull request to `microsoft/winget-pkgs` under
   `manifests/a/adhadse/Bookx/<version>/`. Their bot validates the URL, checksum and runs a sandbox install;
   Microsoft's validation also scans the zip with Defender, and an unsigned GTK app can get flagged: expect a first
   rejection to be about SmartScreen/AV, not about the manifest.
5. Later versions: `wingetcreate update adhadse.Bookx --version <v> --urls <zip-url> --submit`, or the
   `vedantmgoyal9/winget-releaser` action in the release workflow.

Offline syntax check done here: the YAML parses (`python3 -c "import yaml"`) and the files follow manifest schema 1.10.0.
`winget validate` was not run.
