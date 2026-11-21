#!/usr/bin/env python3
"""Checks the pictures in README.md. They must be the generated wiki pictures (build-aux/wiki-screenshots.sh writes
wiki/docs/assets/<name>-light.png and -dark.png), never hand-made copies: every <img src> and <source srcset> must be
a wiki/docs/assets file that exists, is a real PNG, is listed in build-aux/wiki-pictures.txt (so the screenshot run
takes it), and comes as a light and a dark pair. Run by build-aux/release-check.sh."""
import re, sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
readme = (root / "README.md").read_text()
listed = {n for l in (root / "build-aux/wiki-pictures.txt").read_text().splitlines()
          if l and not l.startswith("#") for n in l.split(":", 1)[1].split()}
refs = re.findall(r'(?:src|srcset)="([^"]+)"', readme) + re.findall(r'!\[[^\]]*\]\(([^)\s]+)', readme)
refs = [r for r in refs if not r.startswith(("http://", "https://"))]   # external badges are fine
errors = []
if not refs:
    errors.append("README.md has no pictures")
names = set()
for ref in refs:
    m = re.fullmatch(r"wiki/docs/assets/(.+)-(light|dark)\.png", ref)
    if not m:
        errors.append(f"{ref}: not a generated wiki picture (wiki/docs/assets/<name>-light|dark.png)")
        continue
    names.add(m.group(1))
    f = root / ref
    if not f.is_file():
        errors.append(f"{ref}: missing")
    elif f.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        errors.append(f"{ref}: not a PNG")
for n in sorted(names):
    if n not in listed:
        errors.append(f"{n}: not in build-aux/wiki-pictures.txt, so wiki-screenshots.sh would not refresh it")
    for s in ("light", "dark"):
        if f"wiki/docs/assets/{n}-{s}.png" not in refs:
            errors.append(f"{n}: the {s} variant is not used")
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)
print(f"README.md: {len(refs)} picture references, all generated wiki pictures")
