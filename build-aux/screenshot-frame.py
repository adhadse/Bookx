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

from PIL import Image, ImageChops, ImageDraw, ImageFilter, features

RADIUS = 14
BLUR = 24
OFFSET_Y = 8
ALPHA = {"light": 0.35, "dark": 0.5}  # the same shadow is less visible on a dark page
MARGIN = BLUR * 2 + OFFSET_Y  # room for the blur on every side
PALETTE_COLOURS = 256  # libimagequant keeps the alpha ramp of the shadow with a fully transparent background
PALETTE_MAX_COLOURS = 6000  # a detail with fewer distinct colours is saved as a 256-colour palette
# detail pictures: a subtle shadow and an outline in the wiki's own text colour (wiki/docs/assets/extra.css:
# --md-primary-bg-color #2e3436 on the light page, #ffffff on the slate page), not pure black/white
DETAIL_RADIUS, DETAIL_BLUR, DETAIL_DY = 8, 12, 4
DETAIL_MARGIN = DETAIL_BLUR + DETAIL_DY + 4
DETAIL_SHADOW = {"light": 0.2, "dark": 0.32}
DETAIL_RING = {"light": ((0x2E, 0x34, 0x36), 34), "dark": ((0xFF, 0xFF, 0xFF), 22)}


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


def frame_detail(img, scheme, max_width=1600):
    """A detail picture (a crop of a widget, not a whole window): the top bar is not cut, the corners are rounded a
    little, a thin outline and a small soft shadow go around it. Wider than `max_width` is shrunk."""
    img = img.convert("RGBA")
    if img.width > max_width:
        img = img.resize((max_width, round(img.height * max_width / img.width)), Image.LANCZOS)
    radius, blur, dy, margin = DETAIL_RADIUS, DETAIL_BLUR, DETAIL_DY, DETAIL_MARGIN
    w, h = img.size
    mask = rounded_mask((w, h), radius)
    out = Image.new("RGBA", (w + 2 * margin, h + 2 * margin), (0, 0, 0, 0))
    sh = Image.new("L", out.size, 0)
    sh.paste(mask, (margin, margin + dy))
    sh = sh.filter(ImageFilter.GaussianBlur(blur / 2)).point(lambda v: int(v * DETAIL_SHADOW[scheme]))
    shadow = Image.new("RGBA", out.size, (0, 0, 0, 0))
    shadow.putalpha(sh)
    out = Image.alpha_composite(out, shadow)
    win = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    win.paste(img, (0, 0), mask)
    out.alpha_composite(win, (margin, margin))
    ring = ImageChops.subtract(mask, mask.filter(ImageFilter.MinFilter(3)))
    colour, ring_alpha = DETAIL_RING[scheme]
    ringimg = Image.new("RGBA", (w, h), colour + (0,))
    ringimg.putalpha(ring.point(lambda v: int(v * ring_alpha / 255)))
    out.alpha_composite(ringimg, (margin, margin))
    return out


def save_window(out, path):
    """Saves a framed whole window. A screenshot with covers or photographs has far more than 6000 colours, but
    libimagequant still keeps the shadow's alpha ramp and the transparent background, and cuts the picture size by
    about two thirds so the wiki stays inside its asset budget."""
    if features.check_feature("libimagequant"):
        out = out.quantize(PALETTE_COLOURS, method=Image.LIBIMAGEQUANT, dither=Image.NONE)
    out.save(path, optimize=True)


def save_detail(out, path):
    """Saves a framed detail. Flat interface colours become a palette PNG, but only through libimagequant, which keeps
    the alpha ramp of the shadow and a fully transparent background (Pillow's own octree quantiser squashes the
    shadow into one 6/255 black haze that shows as a dark rectangle and ring on the page). Without libimagequant, or
    for a picture with gradients (covers, photographs), the picture stays lossless RGBA."""
    if features.check_feature("libimagequant") and (out.getcolors(maxcolors=PALETTE_MAX_COLOURS) or None) is not None:
        out = out.quantize(256, method=Image.LIBIMAGEQUANT, dither=Image.NONE)
    out.save(path, optimize=True)


def reframe_detail(img, scheme):
    """Repairs a detail picture that was framed by the old code: the old shadow and ring are cut off again
    (MARGIN_DETAIL + 1 px per side) and the window is framed anew. The rounded corner squares are filled with the
    window colour first so no old shadow shows through the new corners."""
    img = img.convert("RGBA")
    cut = DETAIL_MARGIN + 1
    win = img.crop((cut, cut, img.width - cut, img.height - cut))
    w, h = win.size
    r = DETAIL_RADIUS
    for cx, cy, px, py in ((0, 0, r, r), (w - r, 0, w - r - 1, r), (0, h - r, r, h - r - 1), (w - r, h - r, w - r - 1, h - r - 1)):
        win.paste(Image.new("RGBA", (r, r), win.getpixel((px, py))[:3] + (255,)), (cx, cy))
    return frame_detail(win.convert("RGB").convert("RGBA"), scheme)


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
    ap.add_argument("--detail", action="store_true", help="a crop of a widget: small shadow, top bar kept")
    ap.add_argument("--reframe", action="store_true", help="repair a detail picture framed by the old code (IN may be OUT)")
    ap.add_argument("files", nargs="+")
    a = ap.parse_args()
    if a.preview:
        preview(a.files[0], a.files[1:])
        return 0
    if len(a.files) != 2:
        ap.error("IN.png OUT.png")
    if a.reframe:
        save_detail(reframe_detail(Image.open(a.files[0]), a.scheme), a.files[1])
        return 0
    if a.detail:
        save_detail(frame_detail(Image.open(a.files[0]), a.scheme), a.files[1])
        return 0
    else:
        save_window(frame(Image.open(a.files[0]), a.scheme, a.panel), a.files[1])
    return 0


if __name__ == "__main__":
    sys.exit(main())
