#!/usr/bin/env python3
"""Makes the built wiki lighter (needs Pillow; build-aux/wiki.sh installs it into the wiki venv).

  wiki-optimize.py SITE_DIR

* every screenshot (x-light.png, x-dark.png) is written as lossy WebP with a lossless alpha channel (the window shadow
  stays smooth) and the PNG is dropped from the built site (the PNGs in wiki/docs/assets stay the source);
* the HTML is rewritten to point at the .webp (the #only-light / #only-dark fragments are kept);
* every <img> that has none gets loading="lazy" decoding="async" (a hidden light or dark variant is then never fetched)
  and its natural width and height, so the page does not jump while pictures arrive. Pictures with fetchpriority
  (the hero) and SVGs are left eager.
"""
import re
import sys
from pathlib import Path

from PIL import Image

QUALITY = 88


def main():
    site = Path(sys.argv[1]).resolve()
    saved = 0
    converted = {}
    for png in sorted(site.rglob("*.png")):
        if not re.search(r"-(light|dark)\.png$", png.name):
            continue
        webp = png.with_suffix(".webp")
        im = Image.open(png).convert("RGBA")
        im.save(webp, "WEBP", quality=QUALITY, alpha_quality=100, method=6)
        converted[png] = im.size
        saved += png.stat().st_size - webp.stat().st_size
        png.unlink()
    img_re = re.compile(r"<img\b[^>]*>")
    n_lazy = 0
    for page in site.rglob("*.html"):
        html = page.read_text(encoding="utf-8")

        def fix(m):
            nonlocal n_lazy
            tag = m.group(0)
            src = re.search(r'\bsrc="([^"]*)"', tag)
            if not src:
                return tag
            url = src.group(1)
            tag = re.sub(r'(-(?:light|dark))\.png(#|")', r"\1.webp\2", tag)
            path = url.split("#")[0]
            if path.endswith(".svg") or "fetchpriority" in tag or "loading=" in tag:
                return tag
            extra = ' loading="lazy" decoding="async"'
            if "width=" not in tag and not path.startswith(("http:", "https:", "//")):
                target = (page.parent / path.replace(".png", ".webp")).resolve() if not path.startswith("/") else site / path.lstrip("/")
                png = target.with_suffix(".png")
                size = converted.get(png)
                if size is None and target.is_file():
                    try:
                        size = Image.open(target).size
                    except Exception:
                        size = None
                if size:
                    extra += f' width="{size[0]}" height="{size[1]}"'
            n_lazy += 1
            return tag[:-1].rstrip("/").rstrip() + extra + ">"

        new = img_re.sub(fix, html)
        new = re.sub(r'(-(?:light|dark))\.png(#only-)', r"\1.webp\2", new)
        if new != html:
            page.write_text(new, encoding="utf-8")
    print(f"{len(converted)} screenshots as WebP ({saved // 1024} KB smaller), {n_lazy} pictures lazy-loaded")


if __name__ == "__main__":
    main()
