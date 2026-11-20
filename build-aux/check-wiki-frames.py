#!/usr/bin/env python3
"""Checks the framing (shadow, outline, transparent background) of the wiki pictures (Pillow only).

  build-aux/check-wiki-frames.py            # unit tests of screenshot-frame.py, then every wiki/docs/assets/*-light|dark.png

A framed picture must have
  * a fully transparent background: the corner pixels and the outermost rows and columns have alpha 0 (the old
    palette quantiser turned the whole margin into a 6/255 black haze: a dark rectangle on the page);
  * a window that is opaque (alpha >= 253 in the middle);
  * a subtle shadow: the strongest pixel outside the window is below 45% alpha, and no pixel of the margin is a solid ring;
  * an outline in the wiki's own text colour (extra.css `--md-primary-bg-color`), light on the slate page, not black.
"""
import importlib.util
import os
import re
import sys
import unittest

from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
ASSETS = os.path.join(ROOT, "wiki", "docs", "assets")
CSS = os.path.join(ASSETS, "extra.css")
_spec = importlib.util.spec_from_file_location("screenshot_frame", os.path.join(ROOT, "build-aux", "screenshot-frame.py"))
frame = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(frame)

MAX_SHADOW_ALPHA = 115  # 45 %


def px(img, x, y):
    return img.convert("RGBA").getpixel((x, y))


def problems_of(img, margin):
    """What is wrong with a framed picture (a list of messages, empty when it is fine)."""
    rgba = img.convert("RGBA")
    w, h = rgba.size
    alpha = rgba.getchannel("A")
    out = []
    corners = [px(rgba, 0, 0), px(rgba, w - 1, 0), px(rgba, 0, h - 1), px(rgba, w - 1, h - 1)]
    if any(c[3] > 1 for c in corners):
        out.append(f"background is not transparent in the corners: {corners}")
    edge = max(
        max(alpha.getpixel((x, y)) for x in range(w) for y in (0, h - 1)),
        max(alpha.getpixel((x, y)) for y in range(h) for x in (0, w - 1)),
    )
    if edge > 1:
        out.append(f"outermost pixels have alpha {edge}, the shadow is cut off or there is a haze")
    mid = px(rgba, w // 2, h // 2)
    if mid[3] < 253:
        out.append(f"window is translucent in the middle: {mid}")
    outside = [alpha.getpixel((x, y)) for y in range(h) for x in range(w)
               if not (margin <= x < w - margin and margin <= y < h - margin)]
    if outside and max(outside) > MAX_SHADOW_ALPHA:
        out.append(f"shadow is too strong: alpha {max(outside)} outside the window")
    return out


def css_colour(scheme):
    css = open(CSS, encoding="utf8").read()
    name = "default" if scheme == "light" else "slate"
    block = re.search(r'\[data-md-color-scheme="%s"\]\s*\{(.*?)\}' % name, css, re.S).group(1)
    v = re.search(r"--md-primary-bg-color:\s*#([0-9a-fA-F]{6})", block).group(1)
    return tuple(int(v[i:i + 2], 16) for i in (0, 2, 4))


def old_quantised(img):
    """What the old code did to a detail: Pillow's octree quantiser on RGBA."""
    return img.quantize(256, method=Image.FASTOCTREE, dither=Image.NONE).convert("RGBA")


def sample_window(scheme, colour):
    win = Image.new("RGBA", (300, 200), colour + (255,))
    for x in range(40, 120):  # a few more colours, like an interface
        for y in range(40, 80):
            win.putpixel((x, y), (30 + x % 50, 80, 160 + y % 40, 255))
    return win


class Frames(unittest.TestCase):
    def check(self, scheme, bg):
        out = frame.frame_detail(sample_window(scheme, bg), scheme)
        self.assertEqual(problems_of(out, frame.DETAIL_MARGIN), [])
        return out

    def test_new_detail_light(self):
        self.check("light", (250, 250, 251))

    def test_new_detail_dark(self):
        self.check("dark", (36, 36, 36))

    def test_saved_detail_keeps_alpha(self):
        import tempfile
        out = frame.frame_detail(sample_window("light", (255, 255, 255)), "light")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "x.png")
            frame.save_detail(out, path)
            self.assertEqual(problems_of(Image.open(path), frame.DETAIL_MARGIN), [])

    def test_saved_window_keeps_alpha(self):
        import tempfile
        out = frame.frame(sample_window("dark", (36, 36, 36)), "dark")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "x.png")
            frame.save_window(out, path)
            img = Image.open(path)
            a = img.convert("RGBA").getchannel("A")
            margin = next((y for y in range(a.height) if a.getpixel((a.width // 2, y)) >= 250), 0)
            self.assertEqual(problems_of(img, max(margin - 1, 0)), [])

    def test_old_quantiser_output_is_rejected(self):
        out = old_quantised(frame.frame_detail(sample_window("light", (250, 250, 251)), "light"))
        found = problems_of(out, frame.DETAIL_MARGIN)
        self.assertTrue(found, "the old palette output must fail the check")

    def test_ring_is_theme_coloured_and_subtle(self):
        for scheme in ("light", "dark"):
            self.assertEqual(frame.DETAIL_RING[scheme][0], css_colour(scheme))
            self.assertLessEqual(frame.DETAIL_RING[scheme][1], 40)  # a thin hint, not a dark ring

    def test_ring_on_page_is_close_to_page(self):
        page = {"light": (250, 250, 251), "dark": (36, 36, 36)}
        for scheme in ("light", "dark"):
            out = frame.frame_detail(sample_window(scheme, page[scheme]), scheme)
            m = frame.DETAIL_MARGIN
            edge = px(out, m + 150, m)  # the ring on the top edge, over the window colour
            comp = Image.alpha_composite(Image.new("RGBA", out.size, page[scheme] + (255,)), out).getpixel((m + 150, m))
            self.assertLess(max(abs(a - b) for a, b in zip(comp[:3], page[scheme])), 30, (scheme, edge, comp))

    def test_reframe_repairs_old_output(self):
        for scheme, bg in (("light", (250, 250, 251)), ("dark", (36, 36, 36))):
            old = old_quantised(frame.frame_detail(sample_window(scheme, bg), scheme))
            fixed = frame.reframe_detail(old, scheme)
            self.assertEqual(problems_of(fixed, frame.DETAIL_MARGIN), [])


def check_assets():
    bad = 0
    for name in sorted(os.listdir(ASSETS)):
        m = re.match(r".*-(light|dark)\.png$", name)
        if not m:
            continue
        img = Image.open(os.path.join(ASSETS, name))
        # whole windows use the larger window margin, details the small one: the margin is where the window starts
        a = img.convert("RGBA").getchannel("A")
        margin = next((y for y in range(a.height) if a.getpixel((a.width // 2, y)) >= 250), 0)
        found = problems_of(img, max(margin - 1, 0))
        for f in found:
            print(f"{name}: {f}")
        bad += bool(found)
    return bad


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Frames)
    ok = unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful()
    bad = check_assets()
    print(f"{bad} pictures with a framing problem" if bad else "all wiki pictures are framed correctly")
    sys.exit(0 if ok and not bad else 1)
