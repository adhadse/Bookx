#!/usr/bin/env python3
"""Writes the offline cargo sources for the Flatpak manifest from Cargo.lock.

Pure standard library (Python 3.11+): no flatpak-cargo-generator and no network needed, because
Cargo.lock already carries the sha256 of every crates.io package. The output is a list of
flatpak-builder sources that unpack each crate into cargo/vendor/<name>-<version>, write the
`.cargo-checksum.json` cargo expects, and point cargo at the vendored directory.

    build-aux/flatpak-cargo-sources.py [Cargo.lock] [-o build-aux/cargo-sources.json]
    build-aux/flatpak-cargo-sources.py --check          # exit 1 when the file is out of date

Crates from git or other registries are refused (the build is meant to be reproducible offline);
path dependencies (the workspace crates) need nothing.
"""
import argparse
import json
import sys
import tomllib
from pathlib import Path

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
CRATE_URL = "https://static.crates.io/crates/{name}/{name}-{version}.crate"
VENDOR = "cargo/vendor"


def sources_for(lock_text: str) -> list:
    lock = tomllib.loads(lock_text)
    out = []
    seen = set()
    for pkg in sorted(lock.get("package", []), key=lambda p: (p["name"], p["version"])):
        source = pkg.get("source")
        if source is None:
            continue  # workspace member
        if source != CRATES_IO:
            raise SystemExit(f"{pkg['name']} {pkg['version']}: unsupported source {source}")
        checksum = pkg.get("checksum")
        if not checksum:
            raise SystemExit(f"{pkg['name']} {pkg['version']}: no checksum in Cargo.lock")
        key = (pkg["name"], pkg["version"])
        if key in seen:
            continue
        seen.add(key)
        name, version = key
        dest = f"{VENDOR}/{name}-{version}"
        out.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": CRATE_URL.format(name=name, version=version),
                "sha256": checksum,
                "strip-components": 1,
                "dest": dest,
            }
        )
        out.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    out.append(
        {
            "type": "inline",
            "contents": (
                "[source.crates-io]\n"
                'replace-with = "vendored-sources"\n\n'
                "[source.vendored-sources]\n"
                f'directory = "{VENDOR}"\n'
            ),
            "dest": "cargo",
            "dest-filename": "config",
        }
    )
    return out


def render(lock_path: Path) -> str:
    return json.dumps(sources_for(lock_path.read_text()), indent=2) + "\n"


def main() -> int:
    here = Path(__file__).resolve().parent
    ap = argparse.ArgumentParser()
    ap.add_argument("lock", nargs="?", default=str(here.parent / "Cargo.lock"))
    ap.add_argument("-o", "--output", default=str(here / "cargo-sources.json"))
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    text = render(Path(args.lock))
    out = Path(args.output)
    if args.check:
        if not out.exists() or out.read_text() != text:
            print(f"{out} is out of date: run build-aux/flatpak-cargo-sources.py", file=sys.stderr)
            return 1
        return 0
    out.write_text(text)
    print(f"{out}: {len(json.loads(text)) // 2} crates")
    return 0


if __name__ == "__main__":
    sys.exit(main())
