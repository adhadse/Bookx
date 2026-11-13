#!/usr/bin/env python3
"""Checks the social cards of a built static site (build-aux/wiki.sh).

  check-social-cards.py SITE_DIR

Zensical's `social` plugin generates one 1200 x 630 PNG per page and writes the Open Graph and Twitter Card meta tags
that make a pasted link render as a preview in chat apps and social sites. The plugin only logs a warning when it cannot
render a card (a missing font, an unreachable Google Fonts), so this check fails the build when a page has no tags, when
its card is missing or has the wrong size, or when the Twitter tags disagree with the Open Graph ones.

Standard library only.
"""

import re
import struct
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlparse

CARD_WIDTH = 1200
CARD_HEIGHT = 630


class Meta(HTMLParser):
    def __init__(self):
        super().__init__()
        self.meta = {}

    def handle_starttag(self, tag, attrs):
        if tag != "meta":
            return
        a = dict(attrs)
        key = a.get("property") or a.get("name")
        if key and "content" in a:
            self.meta.setdefault(key, a["content"])


def parse(path):
    m = Meta()
    m.feed(path.read_text(encoding="utf-8"))
    return m.meta


def png_size(path):
    """(width, height) of a PNG, read from its IHDR chunk, or None when it is not a PNG."""
    with open(path, "rb") as f:
        head = f.read(24)
    if len(head) < 24 or head[:8] != b"\x89PNG\r\n\x1a\n" or head[12:16] != b"IHDR":
        return None
    return struct.unpack(">II", head[16:24])


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    site = Path(sys.argv[1]).resolve()
    # 404.html is the theme's error page, never shared, and the plugin does not card it
    pages = [p for p in sorted(site.rglob("*.html")) if p.relative_to(site).as_posix() != "404.html"]
    if not pages:
        print(f"no HTML pages in {site}")
        return 1
    errors = []
    images = {}
    for page in pages:
        rel = page.relative_to(site)
        meta = parse(page)
        for key in ("og:type", "og:title", "og:description", "og:url", "og:image", "og:image:type",
                    "og:image:width", "og:image:height", "twitter:card", "twitter:title", "twitter:image"):
            if not meta.get(key):
                errors.append(f"{rel}: missing {key}")
        if not meta.get("og:title", "").strip():
            errors.append(f"{rel}: og:title is empty")
        if meta.get("og:type") and meta["og:type"] != "website":
            errors.append(f"{rel}: og:type is {meta['og:type']!r}, expected 'website'")
        if meta.get("og:image:type") and meta["og:image:type"] != "image/png":
            errors.append(f"{rel}: og:image:type is {meta['og:image:type']!r}, expected 'image/png'")
        for key, want in (("og:image:width", CARD_WIDTH), ("og:image:height", CARD_HEIGHT)):
            if meta.get(key) and meta[key] != str(want):
                errors.append(f"{rel}: {key} is {meta[key]!r}, expected {want}")
        if meta.get("twitter:card") and meta["twitter:card"] != "summary_large_image":
            errors.append(f"{rel}: twitter:card is {meta['twitter:card']!r}, expected 'summary_large_image'")
        if meta.get("og:image") and meta.get("twitter:image") and meta["og:image"] != meta["twitter:image"]:
            errors.append(f"{rel}: twitter:image differs from og:image")
        url, image = meta.get("og:url", ""), meta.get("og:image", "")
        if url and not url.startswith(("http://", "https://")):
            errors.append(f"{rel}: og:url is not absolute: {url!r}")
        if image:
            u, i = urlparse(url), urlparse(image)
            if not i.scheme or not i.netloc:
                errors.append(f"{rel}: og:image is not absolute: {image!r}")
            elif u.netloc and i.netloc != u.netloc:
                errors.append(f"{rel}: og:image host {i.netloc!r} differs from og:url host {u.netloc!r}")
            if i.path and i.netloc:
                target = (site / i.path.lstrip("/")).resolve()
                try:
                    target.relative_to(site)
                except ValueError:
                    errors.append(f"{rel}: og:image points outside the site: {image!r}")
                else:
                    images[target] = rel
        # the card must be the page's own URL, so a shared link previews the page it names
        if url and meta.get("og:image"):
            if rel.name == "index.html":
                expected = "/" + ("" if rel.parent.as_posix() == "." else rel.parent.as_posix() + "/")
            else:
                expected = "/" + rel.as_posix()
            if urlparse(url).path != expected:
                errors.append(f"{rel}: og:url path {urlparse(url).path!r} does not match the page ({expected!r})")
    for card, rel in sorted(images.items()):
        if not card.is_file():
            errors.append(f"{rel}: social card {card.relative_to(site)} does not exist")
            continue
        size = png_size(card)
        if size is None:
            errors.append(f"{rel}: social card {card.relative_to(site)} is not a PNG")
        elif size != (CARD_WIDTH, CARD_HEIGHT):
            errors.append(f"{rel}: social card {card.relative_to(site)} is {size[0]}x{size[1]}, "
                          f"expected {CARD_WIDTH}x{CARD_HEIGHT}")
    for e in errors:
        print("ERROR", e)
    print(f"checked {len(images)} social cards in {len(pages)} pages: {len(errors)} problems")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
