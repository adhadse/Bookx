#!/usr/bin/env python3
"""Turns a full-screen capture of a maximised Bookx window into a wiki picture: the GNOME top bar is cut off, the
corners are rounded like GNOME draws them, a thin outline is added and a soft drop shadow is painted around the window
on a transparent background (Pillow only).

  screenshot-frame.py --scheme light|dark IN.png OUT.png [--panel 32]
  screenshot-frame.py --preview OUT.png FRAMED.png...   # composites the pictures on a light and a dark page

Defaults match what the window looks like on a GNOME desktop: radius 14 px, blur 24 px, y offset 8 px, alpha 0.35.
"""
import argparse
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

RADIUS = 14
BLUR = 24
OFFSET_Y = 8
ALPHA = {"light": 0.35, "dark": 0.5}  # the same shadow is less visible on a dark page
MARGIN = BLUR * 2 + OFFSET_Y  # room for the blur on every side


def rounded_mask(size, radius, scale=4):
    """An anti-aliased rounded rectangle mask (drawn at `scale`x and shrunk)."""
    w, h = size
    big = Image.new("L", (w * scale, h * scale), 0)
    ImageDraw.Draw(big).rounded_rectangle((0, 0, w * scale - 1, h * scale - 1), radius * scale, fill=255)
    return big.resize(size, Image.LANCZOS)


def frame(img, scheme, panel=32):
    img = img.convert("RGBA")
    img = img.crop((0, panel, img.width, img.height))
    w, h = img.size
    mask = rounded_mask((w, h), RADIUS)
    # the outline: a 1 px ring just inside the edge
    inner = rounded_mask((w, h), RADIUS).crop((0, 0, w, h))
    ring = ImageChops.subtract(mask, inner.resize((w, h)).filter(ImageFilter.MinFilter(3)))
    colour = (0, 0, 0) if scheme == "light" else (255, 255, 255)
    ring_alpha = 46 if scheme == "light" else 40
    out = Image.new("RGBA", (w + 2 * MARGIN, h + 2 * MARGIN), (0, 0, 0, 0))
    # shadow: the window's silhouette, blurred, shifted down
    sh = Image.new("L", out.size, 0)
    sh.paste(mask, (MARGIN, MARGIN + OFFSET_Y))
    sh = sh.filter(ImageFilter.GaussianBlur(BLUR / 2))
    sh = sh.point(lambda v: int(v * ALPHA[scheme]))
    shadow = Image.new("RGBA", out.size, (0, 0, 0, 0))
    shadow.putalpha(sh)
    out = Image.alpha_composite(out, shadow)
    win = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    win.paste(img, (0, 0), mask)
    out.alpha_composite(win, (MARGIN, MARGIN))
    ringimg = Image.new("RGBA", (w, h), colour + (0,))
    ringimg.putalpha(ring.point(lambda v: int(v * ring_alpha / 255)))
    out.alpha_composite(ringimg, (MARGIN, MARGIN))
    return out


def preview(out_path, files):
    """The framed pictures on a light (#fafafb) and a dark (#242424) page, side by side, to judge the shadow."""
    imgs = [Image.open(f).convert("RGBA") for f in files]
    scale = 0.4
    pad = 20
    tiles = [i.resize((int(i.width * scale), int(i.height * scale)), Image.LANCZOS) for i in imgs]
    wmax = max(t.width for t in tiles)
    hsum = sum(t.height for t in tiles) + pad * (len(tiles) + 1)
    sheet = Image.new("RGBA", (wmax * 2 + pad * 3, hsum), (0, 0, 0, 0))
    for col, bg in enumerate(((250, 250, 251, 255), (36, 36, 36, 255))):
        sheet.paste(Image.new("RGBA", (wmax + pad * 2 // 2 + pad // 2, hsum), bg), (col * (wmax + pad), 0))
        y = pad
        for t in tiles:
            sheet.alpha_composite(t, (col * (wmax + pad) + pad // 2, y))
            y += t.height + pad
    sheet.convert("RGB").save(out_path)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scheme", choices=("light", "dark"), default="light")
    ap.add_argument("--panel", type=int, default=32, help="height of the GNOME top bar to cut off")
    ap.add_argument("--preview", action="store_true")
    ap.add_argument("files", nargs="+")
    a = ap.parse_args()
    if a.preview:
        preview(a.files[0], a.files[1:])
        return 0
    if len(a.files) != 2:
        ap.error("IN.png OUT.png")
    out = frame(Image.open(a.files[0]), a.scheme, a.panel)
    out.save(a.files[1], optimize=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
