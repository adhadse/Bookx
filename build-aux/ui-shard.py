#!/usr/bin/env python3
"""Which UI scenarios belong to shard i of n, balanced by how long each one takes.

    ui-shard.py i/n                       the scenarios of shard i (1-based), one per line
    ui-shard.py --update times.txt ...    merge "name seconds" lines (TIMES_FILE of ui-tests.sh) into ui-shard-weights.json

The scenarios are the `scenario NAME` lines of build-aux/ui-tests*.sh. Each goes, longest first, to the shard with the
least total so far (greedy; a scenario without a measured time counts as the median). The weights file is a plain
{name: seconds} map; refresh it from the `ui-times` artifacts of a CI run (docs/ci-gates.md). Without the file
ui-tests.sh falls back to a hash of the name.
"""
import glob
import json
import os
import re
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
WEIGHTS = os.path.join(HERE, "ui-shard-weights.json")


def names():
    seen = []
    for f in sorted(glob.glob(os.path.join(HERE, "ui-tests*.sh"))):
        for line in open(f, encoding="utf-8", errors="replace"):
            m = re.match(r"scenario ([A-Za-z0-9_]+)\b", line)
            if m and m.group(1) not in seen:
                seen.append(m.group(1))
    return seen


def assign(all_names, weights, n):
    known = [w for w in weights.values() if w > 0]
    default = statistics.median(known) if known else 1.0
    items = sorted(all_names, key=lambda x: (-weights.get(x, default), x))
    load = [0.0] * n
    shards = [[] for _ in range(n)]
    for name in items:
        i = min(range(n), key=lambda k: (load[k], k))
        shards[i].append(name)
        load[i] += max(weights.get(name, default), 0.1)
    return shards, load


def main(argv):
    if len(argv) >= 2 and argv[0] == "--update":
        old = json.load(open(WEIGHTS)) if os.path.exists(WEIGHTS) else {}
        for f in argv[1:]:
            for line in open(f, encoding="utf-8"):
                p = line.split()
                if len(p) == 2 and p[1].isdigit():
                    old[p[0]] = max(1, int(p[1]))
        live = set(names())
        out = {k: v for k, v in sorted(old.items()) if k in live}
        json.dump(out, open(WEIGHTS, "w"), indent=0, sort_keys=True)
        open(WEIGHTS, "a").write("\n")
        print(f"{len(out)} scenario weights written to {WEIGHTS}")
        return 0
    m = re.fullmatch(r"(\d+)/(\d+)", argv[0]) if argv else None
    if not m or not 1 <= int(m.group(1)) <= int(m.group(2)):
        print(__doc__, file=sys.stderr)
        return 2
    i, n = int(m.group(1)), int(m.group(2))
    weights = json.load(open(WEIGHTS)) if os.path.exists(WEIGHTS) else {}
    shards, load = assign(names(), weights, n)
    print("\n".join(shards[i - 1]))
    if os.environ.get("UI_SHARD_VERBOSE"):
        print("shard loads (s): " + " ".join(f"{x:.0f}" for x in load), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
