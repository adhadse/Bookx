#!/usr/bin/env python3
"""Checks on the `cards:` lines the app prints for the dev hook `reader:cards-rects`
(used by build-aux/ui-tests.sh). Usage: cards_geom.py MODE N [args] < output.
N is the 1-based number of the `cards:` line. Exit status 0 = the check holds."""
import re
import sys

mode, n = sys.argv[1], int(sys.argv[2])
lines = [l for l in sys.stdin.read().splitlines() if l.startswith("cards:")]
if len(lines) < n:
    sys.exit(2)
line = lines[n - 1]
area = tuple(map(int, re.search(r"area=(\d+)x(\d+)", line).groups()))


def rects(key):
    body = re.search(key + r"=\[([^\]]*)\]", line).group(1)
    out = {}
    for item in filter(None, body.split(";")):
        i, v = item.split(":")
        out[int(i)] = tuple(map(float, v.split(",")))
    return out


def meets(a, b, gap=0.0):
    return (a[0] < b[0] + b[2] + gap and b[0] < a[0] + a[2] + gap
            and a[1] < b[1] + b[3] + gap and b[1] < a[1] + a[3] + gap)


shown, pops = rects("shown"), rects("pops")
target = rects("target")
cards = list(shown.values())
ok = True
if mode == "count":
    ok = len(cards) == int(sys.argv[3])
elif mode == "disjoint":
    ok = len(cards) == 2 and not meets(cards[0], cards[1], 8.0)
elif mode == "target_disjoint":
    # the places the cards slide to (what is drawn in between depends on how far the slide has got)
    t = list(target.values())
    ok = len(t) == 2 and not meets(t[0], t[1], 8.0)
elif mode == "yielded":
    # card ID is asked to go somewhere other than where it was drawn on line argv[3]
    cid, before = int(sys.argv[3]), int(sys.argv[4])
    prev = [l for l in open(sys.argv[5]).read().splitlines() if l.startswith("cards:")][before - 1]
    old = re.search(r"shown=\[([^\]]*)\]", prev).group(1)
    old = {int(i): tuple(map(float, v.split(","))) for i, v in (it.split(":") for it in old.split(";") if it)}
    ok = any(abs(a - b) > 2 for a, b in zip(old[cid], target[cid]))
elif mode == "centred":
    # side by side, a gap of about 12 px, the group centred, bottoms level
    a, b = sorted(cards)
    gap = b[0] - (a[0] + a[2])
    left, right = a[0], b[0] + b[2]
    ok = (len(cards) == 2 and 10 <= gap <= 14 and abs((left + right) / 2 - area[0] / 2) <= 2
          and abs((a[1] + a[3]) - (b[1] + b[3])) <= 1)
elif mode == "stacked":
    a, b = sorted(cards, key=lambda r: r[1])
    ok = len(cards) == 2 and a[1] + a[3] + 8 <= b[1]
elif mode == "inside":
    ok = all(r[0] >= 0 and r[1] >= 0 and r[0] + r[2] <= area[0] and r[1] + r[3] <= area[1] for r in cards)
elif mode == "no_pop_overlap":
    ok = len(pops) > 0 and not any(meets(c, p) for c in cards for p in pops.values())
elif mode == "pop_shown":
    ok = len(pops) > 0
elif mode == "no_pops":
    ok = len(pops) == 0
elif mode == "at_wish":
    # card ID is exactly where its wish (fractions fx fy) puts it
    cid, fx, fy = int(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
    r = shown[cid]
    x = 8 + fx * max(area[0] - r[2] - 16, 0)
    y = 8 + fy * max(area[1] - r[3] - 16, 0)
    ok = abs(r[0] - x) <= 1.5 and abs(r[1] - y) <= 1.5
elif mode == "moved_from_wish":
    cid, fx, fy = int(sys.argv[3]), float(sys.argv[4]), float(sys.argv[5])
    r = shown[cid]
    x = 8 + fx * max(area[0] - r[2] - 16, 0)
    y = 8 + fy * max(area[1] - r[3] - 16, 0)
    ok = abs(r[0] - x) > 2 or abs(r[1] - y) > 2
elif mode == "settled":
    ok = "sliding=0" in line and shown == rects("target")
elif mode == "sliding":
    ok = "sliding=1" in line
sys.exit(0 if ok else 1)
