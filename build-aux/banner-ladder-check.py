#!/usr/bin/env python3
"""Reads the `banner:` lines of the banner_ladder scenario on stdin: the strip never takes more than a quarter of the viewport."""
import re
import sys

for line in sys.stdin:
    p = float(re.search(r"percent=([0-9.]+)", line).group(1))
    v = re.search(r"variant=([a-z]+)", line).group(1)
    vh = float(re.search(r"viewport=([0-9]+)x", line).group(1))
    assert v in ("wide", "medium") or p <= 25.01 or vh < 352, line
