#!/usr/bin/env python3
"""Turns the lcov file of `cargo llvm-cov` into the coverage table and the PR gate.

The measuring is done by cargo-llvm-cov (LLVM source-based coverage); this script only
  * drops what is not application code from the numbers: inline `#[cfg(test)]` modules (their lines are
    always "covered" and would inflate every file), test helpers, and, for the logic tier, the GTK glue,
  * groups the lines per crate and per tier (see build-aux/coverage-baseline.json),
  * compares them with the committed baseline (a ratchet) and exits 1 when a group fell by more than the
    tolerance, or when a group that is not in the baseline is below the target,
  * writes a filtered lcov file for the patch check (`diff-cover`) and a Markdown summary.

Usage: coverage-gate.py LCOV [--baseline FILE] [--out DIR] [--update-baseline] [--only-logic] [--ui]
"""
import argparse
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOLERANCE_DEFAULT = 0.2
TARGET_DEFAULT = 90.0

# Files that are test support compiled into the library, never application behaviour.
SUPPORT = re.compile(r"(^|/)(testutil|testgen|test_support|tests_extra|for_tests)(\.rs|/)")

# bookx (the GTK app) files that are glue to the widget toolkit or to a dev/hardware check: they belong to the UI
# tier, which is measured by the Broadway scenarios (build-aux/coverage-ui.sh), not by cargo test.
UI_FILES = re.compile(
    r"^src/(ui/|main\.rs$|app\.rs$|devtools\.rs$|kobo_real_check\.rs$|voice_download\.rs$|[a-z0-9_]*_ui\.rs$)"
)


def group_of(rel):
    """Name of the group a source file counts in, or None when it is not measured."""
    if SUPPORT.search(rel):
        return None
    m = re.match(r"crates/([^/]+)/src/", rel)
    if m:
        return m.group(1)
    if rel.startswith("src/"):
        return "bookx-ui" if UI_FILES.match(rel) else "bookx-logic"
    return None


def test_ranges(path):
    """1-based inclusive line ranges of the `#[cfg(test)]` items at column 0 of a Rust file."""
    try:
        lines = open(path, encoding="utf-8").read().split("\n")
    except OSError:
        return []
    out = []
    i = 0
    while i < len(lines):
        if lines[i] == "#[cfg(test)]":
            j = i + 1
            while j < len(lines) and lines[j].startswith("#["):
                j += 1
            if j < len(lines) and lines[j].rstrip().endswith(";"):
                end = j
            else:
                end = j
                while end < len(lines) and lines[end] not in ("}", "};"):
                    end += 1
            out.append((i + 1, min(end + 1, len(lines))))
            i = end + 1
        else:
            i += 1
    return out


def read_lcov(path):
    """{absolute source path: {line: hits}}."""
    files = {}
    cur = None
    for raw in open(path, encoding="utf-8"):
        raw = raw.strip()
        if raw.startswith("SF:"):
            cur = files.setdefault(raw[3:], {})
        elif raw.startswith("DA:") and cur is not None:
            n, _, h = raw[3:].partition(",")
            h = int(h.split(",")[0])
            n = int(n)
            cur[n] = max(cur.get(n, 0), h)
        elif raw == "end_of_record":
            cur = None
    return files


def measure(lcov):
    """(groups {name: [covered, total]}, per-file {rel: (group, covered, total)}, filtered lcov text)."""
    groups, per_file, out = {}, {}, []
    for path, lines in sorted(read_lcov(lcov).items()):
        rel = os.path.relpath(path, ROOT) if path.startswith(ROOT) else path
        g = group_of(rel)
        if g is None:
            continue
        skip = test_ranges(path)
        keep = {n: h for n, h in lines.items() if not any(a <= n <= b for a, b in skip)}
        if not keep:
            continue
        cov = sum(1 for h in keep.values() if h > 0)
        per_file[rel] = (g, cov, len(keep))
        t = groups.setdefault(g, [0, 0])
        t[0] += cov
        t[1] += len(keep)
        if g != "bookx-ui":
            out.append(f"SF:{path}")
            out += [f"DA:{n},{h}" for n, h in sorted(keep.items())]
            out += [f"LF:{len(keep)}", f"LH:{cov}", "end_of_record"]
    return groups, per_file, "\n".join(out) + "\n"


def pct(c, t):
    return 100.0 * c / t if t else 100.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("lcov")
    ap.add_argument("--baseline", default=os.path.join(ROOT, "build-aux", "coverage-baseline.json"))
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "llvm-cov"))
    ap.add_argument("--update-baseline", action="store_true", help="raise the baseline to the measured numbers")
    ap.add_argument("--tier", choices=["logic", "ui"], default="logic", help="which groups to gate")
    ap.add_argument("--partial", action="store_true", help="only some crates were measured (--quick): others are not compared")
    ap.add_argument("--worst", type=int, default=12, help="how many of the worst-covered files to list")
    a = ap.parse_args()

    groups, per_file, filtered = measure(a.lcov)
    os.makedirs(a.out, exist_ok=True)
    with open(os.path.join(a.out, "lcov.filtered.info"), "w", encoding="utf-8") as f:
        f.write(filtered)

    base = {}
    if os.path.exists(a.baseline):
        base = json.load(open(a.baseline, encoding="utf-8"))
    tol = base.get("tolerance_pp", TOLERANCE_DEFAULT)
    target = base.get("target", TARGET_DEFAULT)
    bg = base.get("groups", {})

    wanted = {g: v for g, v in groups.items() if (g == "bookx-ui") == (a.tier == "ui")}
    if a.partial and a.tier == "logic":
        wanted.pop("bookx-logic", None)  # the app crate was not run (--quick)
    rows, failed = [], []
    for g in sorted(wanted):
        c, t = wanted[g]
        p = pct(c, t)
        b = bg.get(g)
        if b is None:
            status = "new" if p >= target or a.tier == "ui" else "NEW BELOW TARGET"
            if p < target and a.tier == "logic":
                failed.append(f"{g}: new group at {p:.1f}% (target {target:.0f}%); add it with --update-baseline once it is intended")
        else:
            d = p - b
            status = "ok" if d >= -tol else "DROPPED"
            if d < -tol:
                failed.append(f"{g}: {p:.2f}% is {-d:.2f} points below the baseline {b:.2f}% (tolerance {tol})")
        rows.append((g, c, t, p, b, status))
    if a.tier == "logic" and not a.partial:
        for g in bg:
            if g not in groups and g != "bookx-ui":
                failed.append(f"{g}: in the baseline but not measured (crate removed or tests did not run)")

    tc = sum(r[1] for r in rows)
    tt = sum(r[2] for r in rows)
    if a.tier == "logic" and not a.partial and pct(tc, tt) < base.get("fail_under_total", target):
        failed.append(f"total: {pct(tc, tt):.2f}% is below the required {base.get('fail_under_total', target):.0f}% for the whole logic tier")
    lines = [f"| group | lines | covered | % | baseline | change | target {target:.0f}% | |", "|---|---:|---:|---:|---:|---:|:-:|---|"]
    for g, c, t, p, b, st in rows:
        ch = f"{p - b:+.2f}" if b is not None else ""
        lines.append(f"| {g} | {t} | {c} | {p:.2f} | {'' if b is None else f'{b:.2f}'} | {ch} | {'yes' if p >= target else 'no'} | {st} |")
    lines.append(f"| **total ({a.tier} tier)** | {tt} | {tc} | **{pct(tc, tt):.2f}** | | | {'yes' if pct(tc, tt) >= target else 'no'} | |")

    table = "\n".join(lines)

    worst = sorted(
        ((t - c, rel, c, t) for rel, (g, c, t) in per_file.items() if g in wanted and t - c > 0), reverse=True
    )[: a.worst]
    wtxt = "\n".join(f"| `{rel}` | {t - c} | {pct(c, t):.1f} |" for _, rel, c, t in worst)
    md = f"### Coverage ({a.tier} tier, lines; inline test modules and GTK glue excluded)\n\n{table}\n\n"
    md += f"Worst covered files (uncovered lines)\n\n| file | uncovered | % |\n|---|---:|---:|\n{wtxt}\n"
    if failed:
        md += ("\n**Below the baseline (advisory in a quick run)**\n\n" if a.partial else "\n**Gate failed**\n\n") + "\n".join(f"- {x}" for x in failed) + "\n"
    open(os.path.join(a.out, "summary.md"), "w", encoding="utf-8").write(md)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(md)
    print(md)

    if a.update_baseline:
        new = dict(base) if base else {}
        new.setdefault("comment", "Lines coverage per group (cargo llvm-cov, inline test modules and GTK glue excluded). "
                       "A PR may not lower a group by more than tolerance_pp. Raise with build-aux/coverage.sh --update-baseline. "
                       "Goal: every group at target; see docs/ci-gates.md.")
        new["tolerance_pp"] = tol
        new["target"] = target
        ng = dict(new.get("groups", {}))
        for g, c, t, p, b, st in rows:
            ng[g] = max(round(p, 2), ng.get(g, 0)) if a.partial or b is not None else round(p, 2)
        new["groups"] = dict(sorted(ng.items()))
        with open(a.baseline, "w", encoding="utf-8") as f:
            json.dump(new, f, indent=2)
            f.write("\n")
        print(f"baseline updated: {a.baseline}")
        return 0
    if failed and a.partial:
        # the app crate's tests also run code of the other crates, so a --quick run reads lower than the baseline
        print("coverage (quick run, advisory: the full run counts what the app's own tests reach too):", *failed, sep="\n  ")
        return 0
    if failed:
        print("COVERAGE GATE FAILED:", *failed, sep="\n  ", file=sys.stderr)
        return 1
    print("coverage gate: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
