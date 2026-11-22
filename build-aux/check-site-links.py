#!/usr/bin/env python3
"""Checks a built static site: internal links, #anchors and images must resolve.

Usage: check-site-links.py SITE_DIR

Also: every x-light.png (or .webp) has its x-dark one, a page that shows one shows the other, and both carry the #only-light /
#only-dark fragment that makes the palette toggle swap them.

External (http/https/mailto) links are not fetched. Standard library only.
"""

import re
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urldefrag, urlparse


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids = set()
        self.refs = []  # (attribute, value)

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if a.get("id"):
            self.ids.add(a["id"])
        if tag == "a" and a.get("name"):
            self.ids.add(a["name"])
        if tag == "a" and a.get("href"):
            self.refs.append(("href", a["href"]))
        elif tag == "img" and a.get("src"):
            self.refs.append(("src", a["src"]))
        elif tag == "link" and a.get("href") and a.get("rel") in ("stylesheet", "icon", "shortcut icon"):
            self.refs.append(("href", a["href"]))
        elif tag == "video" and a.get("poster"):
            self.refs.append(("poster", a["poster"]))
        elif tag == "source" and a.get("src"):
            self.refs.append(("src", a["src"]))
        elif tag == "script" and a.get("src"):
            self.refs.append(("src", a["src"]))


def parse(path):
    p = Page()
    p.feed(path.read_text(encoding="utf-8"))
    return p


def target_file(site, page, url_path):
    """Maps a URL path (relative to `page`) to a file in the site, or None."""
    base = page.parent
    if url_path.startswith("/"):
        cand = site / url_path.lstrip("/")
    elif url_path == "":
        return page
    else:
        cand = (base / unquote(url_path)).resolve()
    try:
        cand.relative_to(site)
    except ValueError:
        return None
    if cand.is_dir():
        cand = cand / "index.html"
    return cand if cand.is_file() else None


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    site = Path(sys.argv[1]).resolve()
    pages = sorted(site.rglob("*.html"))
    if not pages:
        print(f"no HTML pages in {site}")
        return 1
    parsed = {}
    errors = []
    for page in pages:
        parsed[page] = parse(page)
    checked = 0
    for page, info in parsed.items():
        rel = page.relative_to(site)
        for attr, value in info.refs:
            u = urlparse(value)
            if u.scheme or value.startswith("//") or value.startswith("data:"):
                continue
            path, frag = urldefrag(value)
            path = urlparse(path).path
            checked += 1
            target = target_file(site, page, path)
            if target is None:
                errors.append(f"{rel}: {attr}={value!r} does not exist")
                continue
            if frag and target.suffix == ".html":
                tp = parsed.get(target) or parse(target)
                if unquote(frag) not in tp.ids:
                    errors.append(f"{rel}: anchor #{frag} not found in {target.relative_to(site)}")
    # screenshots come in pairs: x-light.png (shown on the light palette) and x-dark.png (on the dark one)
    for page, info in parsed.items():
        rel = page.relative_to(site)
        srcs = {urldefrag(v)[0] for a, v in info.refs if a == "src"}
        for src in sorted(srcs):
            for ext in (".png", ".webp", ".webm", ".mp4"):
                for mine, other in ((f"-light{ext}", f"-dark{ext}"), (f"-dark{ext}", f"-light{ext}")):
                    if src.endswith(mine) and src[: -len(mine)] + other not in srcs:
                        errors.append(f"{rel}: {src} is used without its {other} variant")
    for ext in (".png", ".webp"):
        for f in sorted(site.rglob(f"*-light{ext}")):
            if not f.with_name(f.name[: -len(f"-light{ext}")] + f"-dark{ext}").is_file():
                errors.append(f"{f.relative_to(site)} has no -dark{ext} variant")
        for f in sorted(site.rglob(f"*-dark{ext}")):
            if not f.with_name(f.name[: -len(f"-dark{ext}")] + f"-light{ext}").is_file():
                errors.append(f"{f.relative_to(site)} has no -light{ext} variant")
    for page, info in parsed.items():
        for a, v in info.refs:
            base = v.split("#")[0]
            if a == "src" and re.search(r"-light\.(png|webp)$", base) and not v.endswith("#only-light"):
                errors.append(f"{page.relative_to(site)}: {v} must end with #only-light")
            if a == "src" and re.search(r"-dark\.(png|webp)$", base) and not v.endswith("#only-dark"):
                errors.append(f"{page.relative_to(site)}: {v} must end with #only-dark")
    used = {(page.parent / urldefrag(v)[0]).resolve() for page, info in parsed.items() for a, v in info.refs
            if a in ("src", "poster")}
    for f in [g for e in ("webm", "mp4") for k in ("light", "dark") for g in sorted(site.rglob(f"*-{k}.{e}"))]:
        if f.resolve() not in used:
            errors.append(f"{f.relative_to(site)} is not used by any page (delete it from wiki/docs/assets)")
    for f in sorted(site.rglob("*-poster-light.webp")) + sorted(site.rglob("*-poster-dark.webp")):
        if f.resolve() not in used:
            errors.append(f"{f.relative_to(site)} is not used by any page (delete it from wiki/docs/assets)")
    for ext in (".png", ".webp"):
        for f in sorted(site.rglob(f"*-light{ext}")) + sorted(site.rglob(f"*-dark{ext}")):
            if "-poster-" in f.name:
                continue
            if f.resolve() not in used:
                errors.append(f"{f.relative_to(site)} is not used by any page (delete it from wiki/docs/assets)")
    for e in errors:
        print("ERROR", e)
    print(f"checked {checked} internal references in {len(pages)} pages: {len(errors)} problems")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
