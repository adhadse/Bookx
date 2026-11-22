#!/usr/bin/env python3
"""Markdown report of a CI run: jobs, tests, coverage, gate verdict. Standard library only.

    ci-report.py --jobs jobs.json --artifacts DIR [--baseline coverage-baseline.json] [--run-url URL] [--pr-comment]

DIR holds the artifacts of the run, one folder each (what `gh run download` or download-artifact writes):
  report-rust/{gate.json,patch.json,junit.xml}   coverage gate, patch coverage, nextest results
  report-ui-N/ui.log                              output of build-aux/ui-tests.sh of shard N
jobs.json is the answer of `GET /repos/OWNER/REPO/actions/runs/ID/jobs`.

The artifacts come from a pull request, so everything read from them is data: it is size-limited, parsed, never
executed, and escaped before it goes into Markdown (no HTML, no links, no mentions). `--pr-comment` adds the marker
that the sticky comment of .github/workflows/pr-report.yml is found by.
"""
import argparse
import datetime
import glob
import json
import os
import re
import sys
import xml.etree.ElementTree as ET

MARKER = "<!-- bookx-ci-report -->"
MAX_FILE = 20 * 1024 * 1024
MAX_REPORT = 60000
LIST_CAP = 15


def esc(text, limit=160):
    """Plain text for a Markdown table cell: control characters, markup and mentions made harmless."""
    t = re.sub(r"[\x00-\x1f\x7f]+", " ", str(text)).strip()
    if len(t) > limit:
        t = t[: limit - 1] + "…"
    t = t.replace("\\", "\\\\").replace("|", "\\|").replace("`", "'")
    t = t.replace("<", "&lt;").replace(">", "&gt;").replace("@", "@\u200b").replace("[", "\\[")
    return t


def read(path):
    try:
        if os.path.getsize(path) > MAX_FILE:
            return None
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return None


def read_json(path):
    t = read(path)
    if t is None:
        return None
    try:
        return json.loads(t)
    except ValueError:
        return None


def seconds(job):
    try:
        f = "%Y-%m-%dT%H:%M:%SZ"
        a = datetime.datetime.strptime(job["started_at"], f)
        b = datetime.datetime.strptime(job["completed_at"], f)
        return max(0, int((b - a).total_seconds()))
    except (KeyError, TypeError, ValueError):
        return None


def dur(s):
    if s is None:
        return ""
    return f"{s // 60}m {s % 60:02d}s" if s >= 60 else f"{s}s"


ICON = {"success": "✅", "failure": "❌", "cancelled": "🚫", "skipped": "⏭️", "timed_out": "⏱️", None: "⏳"}


def icon(job):
    if job.get("status") != "completed":
        return "⏳"
    return ICON.get(job.get("conclusion"), "❔")


def jobs_section(data, run_url):
    jobs = (data or {}).get("jobs", [])
    if not jobs:
        return "No job information.\n", []
    rows = ["| job | result | time |", "|---|---|---:|"]
    failed = []
    for j in jobs:
        link = j.get("html_url") or run_url or ""
        name = esc(j.get("name", "?"), 80)
        # only the job link of this repository's run is used; it is built by GitHub, not read from an artifact
        cell = f"[{name}]({link})" if link.startswith("https://github.com/") else name
        rows.append(f"| {cell} | {icon(j)} {esc(j.get('conclusion') or j.get('status') or '', 20)} | {dur(seconds(j))} |")
        if j.get("conclusion") == "failure":
            for s in j.get("steps", []):
                if s.get("conclusion") == "failure":
                    failed.append((j.get("name", "?"), s.get("name", "?")))
    return "\n".join(rows) + "\n", failed


def junit(path):
    """(tests, failed, skipped, [names of the failed tests]) of a nextest JUnit file."""
    text = read(path)
    if text is None:
        return None
    try:
        root = ET.fromstring(text)
    except ET.ParseError:
        return None
    tests = failed = skipped = 0
    names = []
    for case in root.iter("testcase"):
        tests += 1
        kinds = {c.tag for c in case}
        if kinds & {"failure", "error"}:
            failed += 1
            names.append(f"{case.get('classname', '')} {case.get('name', '')}".strip())
        elif "skipped" in kinds:
            skipped += 1
    return tests, failed, skipped, names


def ui_log(text):
    """(passed, failed, [(scenario, first failing check)]) from the output of ui-tests.sh."""
    passed = failed = 0
    bad = {}
    for line in text.splitlines():
        m = re.match(r"passed: (\d+), failed: (\d+)", line)
        if m:
            passed += int(m.group(1))
            failed += int(m.group(2))
            continue
        m = re.match(r"\s+FAIL\s+(\S+)\s+(.*)", line)
        if m:
            bad.setdefault(m.group(1), m.group(2))
    return passed, failed, sorted(bad.items())


def tests_section(art, jobs_data):
    out = []
    j = junit(os.path.join(art, "report-rust", "junit.xml"))
    if j:
        t, f, s, names = j
        out.append(f"- **Unit and integration tests (nextest):** {t - f - s} passed, {f} failed, {s} skipped, {t} in all")
        for n in names[:LIST_CAP]:
            out.append(f"  - ❌ `{esc(n, 120)}`")
        if len(names) > LIST_CAP:
            out.append(f"  - … and {len(names) - LIST_CAP} more")
    else:
        out.append("- **Unit and integration tests (nextest):** no result file (the job did not get that far, or was skipped)")
    p = f = 0
    bad = []
    shards = 0
    for log in sorted(glob.glob(os.path.join(art, "report-ui-*", "ui.log"))):
        text = read(log)
        if text is None:
            continue
        shards += 1
        a, b, c = ui_log(text)
        p, f, bad = p + a, f + b, bad + c
    if shards:
        out.append(f"- **Broadway scenarios:** {p} checks passed, {f} failed ({shards} shards)")
        for n, d in bad[:LIST_CAP]:
            out.append(f"  - ❌ `{esc(n, 60)}`: {esc(d)}")
        if len(bad) > LIST_CAP:
            out.append(f"  - … and {len(bad) - LIST_CAP} more scenarios")
    else:
        out.append("- **Broadway scenarios:** no result (skipped or not reached)")
    steps = {}
    for job in (jobs_data or {}).get("jobs", []):
        for s in job.get("steps", []):
            steps[s.get("name", "")] = s.get("conclusion")
    for label, key in (("KOReader plugin (Lua)", "KOReader plugin tests"), ("Translations", "po/bookx.pot"), ("Translation review table", "Review state table")):
        for name, c in steps.items():
            if name.startswith(key):
                out.append(f"- **{label}:** {ICON.get(c, '⏳')} {esc(c or 'running', 20)}")
                break
    return "\n".join(out) + "\n"


def arrow(d):
    if d is None:
        return ""
    if d > 0.005:
        return f"▲ {d:+.2f}"
    if d < -0.005:
        return f"▼ {d:+.2f}"
    return "= 0.00"


def coverage_section(art, baseline_path):
    gate = read_json(os.path.join(art, "report-rust", "gate.json"))
    if not gate:
        return "No coverage result (the Rust job did not get that far, or was skipped).\n", None
    base = (read_json(baseline_path) or {}).get("groups", {}) if baseline_path else {}
    rows = ["| group | lines | % | baseline | change | |", "|---|---:|---:|---:|---:|---|"]
    for g, v in sorted(gate.get("groups", {}).items()):
        b = v.get("baseline", base.get(g))
        d = None if b is None else v["pct"] - b
        flag = "❌" if v.get("status") in ("DROPPED", "NEW BELOW TARGET") else ""
        rows.append(f"| {esc(g, 40)} | {v['lines']} | {v['pct']:.2f} | {'' if b is None else f'{b:.2f}'} | {arrow(d)} | {flag} |")
    tot = gate.get("total", {})
    target = gate.get("target", 90)
    ok = tot.get("pct", 0) >= target
    out = "\n".join(rows)
    out += f"\n\n**Logic tier total: {tot.get('pct', 0):.2f}%** (target {target:.0f}%) {'✅' if ok else '❌'}\n"
    return out, gate


def patch_section(art):
    p = read_json(os.path.join(art, "report-rust", "patch.json"))
    if not p:
        return "Patch coverage: not measured (not a pull request, or the job did not get that far).\n", None
    pct = p.get("total_percent_covered")
    out = [f"**Patch coverage (lines this pull request adds or changes in logic code): {pct}%** (needs 80%)" if pct is not None else "Patch coverage: no changed logic lines."]
    n = 0
    for f, s in sorted((p.get("src_stats") or {}).items()):
        miss = s.get("violation_lines") or []
        if miss:
            n += 1
            if n <= LIST_CAP:
                shown = ", ".join(str(x) for x in miss[:20]) + (" …" if len(miss) > 20 else "")
                out.append(f"- `{esc(f, 100)}` ({s.get('percent_covered')}%): uncovered lines {esc(shown, 120)}")
    return "\n".join(out) + "\n", pct


def build(args):
    jobs_data = read_json(args.jobs) if args.jobs else None
    jobs_md, failed_steps = jobs_section(jobs_data, args.run_url)
    cov_md, gate = coverage_section(args.artifacts, args.baseline)
    patch_md, patch_pct = patch_section(args.artifacts)
    problems = [f"{j}: step “{s}” failed" for j, s in failed_steps]
    if gate:
        problems += [f"coverage: {x}" for x in gate.get("failed", [])]
    if isinstance(patch_pct, (int, float)) and patch_pct < 80:
        problems.append(f"patch coverage {patch_pct}% is below 80%")
    running = any(j.get("status") != "completed" for j in (jobs_data or {}).get("jobs", []) if j.get("name") != "required")
    if problems:
        verdict = "❌ **CI is red**"
    elif running:
        verdict = "⏳ **CI is still running**"
    else:
        verdict = "✅ **CI is green**"
    parts = []
    if args.pr_comment:
        parts.append(MARKER)
    parts.append(f"## {verdict}")
    if args.sha:
        parts.append(f"Commit `{esc(args.sha[:12], 12)}`")
    parts += ["### Jobs", jobs_md]
    if problems:
        parts.append("### Gate verdict\n\n" + "\n".join(f"- {esc(x, 300)}" for x in problems) + "\n")
    parts += ["### Tests", tests_section(args.artifacts, jobs_data), "### Coverage", cov_md, patch_md]
    if args.run_url:
        parts.append(f"[Artifacts (HTML coverage report, lcov, test results)]({args.run_url}#artifacts) · [This run]({args.run_url})")
    md = "\n".join(parts) + "\n"
    if len(md) > MAX_REPORT:
        md = md[:MAX_REPORT] + "\n\n_(report cut)_\n"
    return md


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--jobs")
    ap.add_argument("--artifacts", default=".")
    ap.add_argument("--baseline")
    ap.add_argument("--run-url", default="")
    ap.add_argument("--sha", default="")
    ap.add_argument("--pr-comment", action="store_true")
    args = ap.parse_args(argv)
    if args.run_url and not args.run_url.startswith("https://github.com/"):
        args.run_url = ""
    sys.stdout.write(build(args))
    return 0


if __name__ == "__main__":
    sys.exit(main())
