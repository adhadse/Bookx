#!/usr/bin/env python3
"""Fails when a wiki screenshot shows a keyboard-focus ring or a stray mouse cursor.

  check-screenshots.py [--allow FILE] [--debug] PNG_OR_DIR...

A libadwaita focus ring is a thin (1 to 4 px) accent-blue outline in the shape of a rounded rectangle around a widget (a
focused search entry, the reading canvas, a button reached with the keyboard). The check looks for that shape: two long thin
bluish horizontal runs of the same extent with two thin bluish vertical runs joining them. Progress bars, sliders,
underline highlights and filled buttons are not outlines and do not match. The mouse cursor is not in the pictures by
construction (the scenarios take them with cursor=false); this script also fails when a screenshot scenario of
build-aux/session/scenarios_wiki.py asks for the cursor.

Tunable: MIN_RUN (shortest side of a ring, px), MAX_THICK (thickest line), HUE and SAT ranges. A picture that is meant to
show a ring can be listed (one file name per line) in the --allow file. Needs Pillow. Exits 1 on a suspected ring.
"""
import argparse
import colorsys
import os
import re
import sys

from PIL import Image

MIN_RUN = 48       # px: the shortest side of a ring to look for
MAX_THICK = 4      # px: thicker than this is a fill, not an outline
HUE = (190, 240)   # degrees: libadwaita accent blue, light and dark
SAT = (0.28, 1.0)
VAL = (0.35, 1.0)
EDGE_SLACK = 5     # px: how well the two sides of a ring line up
MIN_BOX_H = 14


def bluish(px):
    r, g, b, a = px
    if a < 200:
        return False
    h, s, v = colorsys.rgb_to_hsv(r / 255, g / 255, b / 255)
    return HUE[0] <= h * 360 <= HUE[1] and SAT[0] <= s <= SAT[1] and VAL[0] <= v <= VAL[1]


def runs(mask_rows, w, h):
    """Horizontal runs of bluish pixels of at least MIN_RUN: [(y, x0, x1)] that are at most MAX_THICK thick."""
    found = []
    for y in range(h):
        row = mask_rows[y]
        x = 0
        while x < w:
            if row[x]:
                x0 = x
                while x < w and row[x]:
                    x += 1
                if x - x0 >= MIN_RUN:
                    found.append((y, x0, x))
            else:
                x += 1
    return found


def thin(runs_list, mask_rows, h):
    """Keep the runs whose vertical extent at the run's middle is at most MAX_THICK pixels."""
    out = []
    for y, x0, x1 in runs_list:
        xm = (x0 + x1) // 2
        top = y
        while top > 0 and mask_rows[top - 1][xm]:
            top -= 1
        bot = y
        while bot < h - 1 and mask_rows[bot + 1][xm]:
            bot += 1
        if bot - top + 1 <= MAX_THICK and y == top:
            out.append((y, x0, x1))
    return out


def find_rings(path):
    im = Image.open(path).convert("RGBA")
    w, h = im.size
    px = im.load()
    mask = [[bluish(px[x, y]) for x in range(w)] for y in range(h)]
    hr = thin(runs(mask, w, h), mask, h)
    rings = []
    for i, (y0, a0, a1) in enumerate(hr):
        for y1, b0, b1 in hr[i + 1:]:
            if y1 - y0 < MIN_BOX_H or abs(a0 - b0) > EDGE_SLACK or abs(a1 - b1) > EDGE_SLACK:
                continue
            # the left and right sides: bluish most of the way between the two lines (rounded corners excluded)
            ok = 0
            total = 0
            for yy in range(y0 + 8, y1 - 8):
                total += 1
                if any(mask[yy][x] for x in range(max(0, a0 - 1), min(w, a0 + MAX_THICK + 1))):
                    ok += 1
            if total and ok / total >= 0.8:
                rings.append((a0, y0, a1 - a0, y1 - y0))
    # one ring is found several times (every pair of its lines): merge boxes that overlap a lot
    merged = []
    for r in sorted(rings):
        if not any(abs(r[0] - m[0]) < 12 and abs(r[1] - m[1]) < 12 for m in merged):
            merged.append(r)
    return merged


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--allow")
    ap.add_argument("--debug", action="store_true")
    ap.add_argument("paths", nargs="+")
    a = ap.parse_args()
    allowed = set()
    if a.allow and os.path.exists(a.allow):
        allowed = {l.strip() for l in open(a.allow) if l.strip() and not l.startswith("#")}
    files = []
    for p in a.paths:
        if os.path.isdir(p):
            for root, _, names in os.walk(p):
                files += [os.path.join(root, n) for n in sorted(names) if n.endswith(".png")]
        else:
            files.append(p)
    bad = 0
    for f in files:
        if os.path.basename(f) in allowed:
            continue
        rings = find_rings(f)
        if rings or a.debug:
            print(f"{'RING' if rings else 'ok'}  {f}: {rings}")
        if rings:
            bad += 1
    # the pictures are taken without the pointer: no scenario of the wiki may ask for it
    here = os.path.dirname(os.path.abspath(__file__))
    scen = os.path.join(here, "session", "scenarios_wiki.py")
    if os.path.exists(scen):
        text = open(scen).read()
        for m in re.finditer(r"screenshot_to\([^)]*cursor\s*=\s*True|\bc\.shot\(|\.screenshot\(", text):
            print(f"CURSOR  scenarios_wiki.py takes a screenshot with the pointer: {m.group(0)}")
            bad += 1
    print(f"checked {len(files)} pictures: {bad} problems")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
