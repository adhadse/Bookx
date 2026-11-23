#!/usr/bin/env python3
"""Reads a Bookx problem report (pasted into an issue) and says what is probably wrong.

    build-aux/triage-report.py report.md          # or: pbpaste | build-aux/triage-report.py
    build-aux/triage-report.py --json report.md   # for scripts (the labeller idea in docs)

It uses the same rule table as the report's own "Summary and suspected area" block,
build-aux/triage-rules.json (read by src/triage.rs too): one table, two readers. Standard library only.

Prints the area, the severity (crash / data risk / annoyance), the suspected cause, the key facts,
and the five things to ask the user when the report cannot tell, so a "needs more info" reply is
mechanical.
"""
import json
import os
import re
import sys

RULES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "triage-rules.json")
SEVERITY = {"crash": "crash", "data": "data risk", "annoyance": "annoyance"}


def load_rules(path=RULES):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def rule_input(report):
    """The part of a report the rules read: no earlier summary, no user text."""
    text = report
    i = text.find("### What happened")
    if i >= 0:
        text = text[:i]
    i = text.find("### Summary and suspected area")
    if i >= 0:
        rest = text[i + 3:]
        j = rest.find("\n### ")
        end = len(text) if j < 0 else i + 3 + j + 1
        text = text[:i] + text[end:]
    return text


def fill(template, caps):
    out = template
    for k, v in caps.items():
        out = out.replace("{" + k + "}", v)
    return re.sub(r" ?\{[a-z_]+\}", "", out)


def analyse(report, table=None):
    """Every rule that matches, in the order of the table."""
    table = table or load_rules()
    text = rule_input(report)
    found = []
    for r in table["rules"]:
        caps = {}

        def take(pattern):
            m = re.search(pattern, text)
            if not m:
                return False
            for name, value in m.groupdict().items():
                if value is not None:
                    caps.setdefault(name, value.strip())
            return True

        if not all(take(p) for p in r["all"]):
            continue
        if r["any"] and not any([take(p) for p in r["any"]]):
            continue
        if any(re.search(p, text) for p in r["none"]):
            continue
        evidence = []
        for p in r["evidence"]:
            m = re.search(p, text)
            if m and m.group(0).strip():
                evidence.append(m.group(0).strip())
        asks = list(r["asks"])
        for a in table["generic_asks"]:
            if len(asks) >= 5:
                break
            if a not in asks:
                asks.append(a)
        area = r["area"]
        if area == "@area":
            area = table["area_labels"].get(caps.get("area", ""), "Network")
        found.append({
            "id": r["id"],
            "area": area,
            "severity": SEVERITY.get(r["severity"], "annoyance"),
            "state": bool(r.get("state")),
            "cause": fill(r["cause"], caps),
            "hint": r["hint"],
            "evidence": evidence[:3],
            "asks": asks,
        })
    return found


def headline(found):
    for f in found:
        if not f["state"]:
            return f
    return found[0] if found else None


def environment_facts(report):
    """The lines that say which build and machine it is."""
    keys = ("- Version:", "- Installed as:", "- OS:", "- Desktop:", "- Language:", "- Start-up:")
    return [l.strip() for l in report.splitlines() if l.strip().startswith(keys)][:6]


def render(report, table=None):
    found = analyse(report, table)
    h = headline(found)
    out = []
    if h is None:
        out.append("Area: not clear from the report (nothing failed that Bookx could see)")
        out.append("Severity: unknown")
        out.append("")
        out.append("Ask the user:")
        for i, a in enumerate((table or load_rules())["generic_asks"], 1):
            out.append(f"  {i}. {a}")
        return "\n".join(out)
    out.append(f"Area: {h['area']}")
    out.append(f"Severity: {h['severity']}")
    out.append(f"Suspected cause: {h['cause']}")
    out.append(f"Hint: {h['hint']}")
    facts = h["evidence"] + environment_facts(report)
    if facts:
        out.append("Key facts:")
        out.extend(f"  {f}" for f in facts)
    others = []
    for f in found:
        if f["id"] != h["id"] and f["area"] not in others and f["area"] != h["area"]:
            others.append(f["area"])
    if others:
        out.append("Also noticed: " + "; ".join(others[:4]))
    out.append("If the report cannot tell, ask the user:")
    for i, a in enumerate(h["asks"], 1):
        out.append(f"  {i}. {a}")
    return "\n".join(out)


def main(argv):
    as_json = "--json" in argv
    args = [a for a in argv if not a.startswith("--")]
    text = open(args[0], encoding="utf-8").read() if args else sys.stdin.read()
    if as_json:
        found = analyse(text)
        h = headline(found)
        print(json.dumps({"headline": h, "findings": found}, ensure_ascii=False, indent=1))
    else:
        print(render(text))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
