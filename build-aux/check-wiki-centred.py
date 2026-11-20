#!/usr/bin/env python3
"""Checks that the framed window of every wiki picture is centred (Pillow only).

  build-aux/check-wiki-centred.py [--tolerance 2] [DIR]

The window is the part of the picture that is (almost) opaque; the shadow around it is transparent-ish and shifts down,
never sideways, so the left and right transparent margins must be equal. A picture whose window sits off-centre makes
the page look crooked (the picture is centred by CSS, so the window moves with it). The vertical margins differ on
purpose (the shadow is offset downwards) and are not checked, except that the window may not touch the edge.
"""
import os
import re
import sys
import unittest

from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
ASSETS = os.path.join(ROOT, "wiki", "docs", "assets")
OPAQUE = 250


def window_margins(img):
    """(left, right, top, bottom) transparent margins around the opaque window, or None when there is none."""
    alpha = img.convert("RGBA").getchannel("A").point(lambda v: 255 if v >= OPAQUE else 0)
    box = alpha.getbbox()
    if box is None:
        return None
    return box[0], img.width - box[2], box[1], img.height - box[3]


def problems_of(img, tolerance=2):
    m = window_margins(img)
    if m is None:
        return ["no opaque window found"]
    left, right, top, bottom = m
    out = []
    if abs(left - right) > tolerance:
        out.append(f"window is off-centre: left margin {left} px, right margin {right} px")
    if min(top, bottom) < 1:
        out.append(f"window touches the top or bottom edge (margins {top}/{bottom})")
    return out


class Centred(unittest.TestCase):
    def test_centred_passes(self):
        img = Image.new("RGBA", (120, 80), (0, 0, 0, 0))
        img.paste((255, 255, 255, 255), (10, 10, 110, 70))
        self.assertEqual(problems_of(img), [])

    def test_off_centre_fails(self):
        img = Image.new("RGBA", (120, 80), (0, 0, 0, 0))
        img.paste((255, 255, 255, 255), (4, 10, 104, 70))
        self.assertTrue(problems_of(img))

    def test_shadow_does_not_count(self):
        img = Image.new("RGBA", (120, 80), (0, 0, 0, 0))
        img.paste((0, 0, 0, 60), (0, 0, 120, 80))  # a translucent shadow reaching the edges
        img.paste((255, 255, 255, 255), (20, 10, 100, 60))
        self.assertEqual(problems_of(img), [])

    def test_empty_fails(self):
        self.assertTrue(problems_of(Image.new("RGBA", (10, 10), (0, 0, 0, 0))))


def check_assets(directory, tolerance):
    bad = total = 0
    for name in sorted(os.listdir(directory)):
        if not re.match(r".*-(light|dark)\.png$", name):
            continue
        total += 1
        found = problems_of(Image.open(os.path.join(directory, name)), tolerance)
        for f in found:
            print(f"{name}: {f}")
        bad += bool(found)
    return total, bad


if __name__ == "__main__":
    args = sys.argv[1:]
    tol = 2
    if "--tolerance" in args:
        i = args.index("--tolerance")
        tol = int(args[i + 1])
        del args[i:i + 2]
    ok = unittest.TextTestRunner(verbosity=0).run(unittest.defaultTestLoader.loadTestsFromTestCase(Centred)).wasSuccessful()
    total, bad = check_assets(args[0] if args else ASSETS, tol)
    print(f"wiki picture centring: {total} pictures, {bad} off-centre (tolerance {tol} px)")
    sys.exit(0 if ok and not bad else 1)
