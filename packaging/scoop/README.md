# Scoop bucket for Bookx

Status: **untested scaffolding** (`docs/windows-port.md`).

Scoop buckets are Git repositories. Create `github.com/adhadse/scoop-bookx` with `bucket/bookx.json` (this file), or
send the manifest to the `extras` bucket later. Users:

```powershell
scoop bucket add bookx https://github.com/adhadse/scoop-bookx
scoop install bookx
```

`bin` exposes only `bookx-cli.exe` on the PATH; the GUI `bookx.exe` gets a Start-menu shortcut. `checkver` reads the
latest GitHub release and `autoupdate` rewrites the URL and takes the hash from the `.sha256` file the release workflow
writes next to the zip (`Bookx-<version>-windows-x64.zip.sha256` containing the bare hash or `hash  filename`).
Test with `scoop install .\bucket\bookx.json`, check with `scoop checkver bookx` and `.\bin\checkver.ps1` from
the Scoop installation (the bucket template includes it).
