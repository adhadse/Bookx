#!/usr/bin/env python3
"""Tests of triage-report.py. Run: python3 build-aux/test_triage_report.py"""
import importlib.util
import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("triage", os.path.join(HERE, "triage-report.py"))
triage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(triage)

PUSH_404 = """<!-- Bookx problem report, made on this computer. Nothing was sent. -->

### Sync (KOSync client)

- Server address: http://<ip:private>:30260/api/koreader
- Document matching: partial-md5

### Recent operations

- Last failure: 12:03:11Z sync/push FAIL http=404 class=status 120 ms fmt=pdf doc=0123456789abcdef0123456789abcdef err="the server answered 404"
- 12:03:11Z sync/push FAIL http=404 class=status 120 ms fmt=pdf doc=0123456789abcdef0123456789abcdef err="the server answered 404"

### Bookx

- Version: 0.3.0-beta.2 (beta release)
- Installed as: Flatpak

### What happened

sync/pull FAIL class=timeout (the user's words must not count)
"""


class Triage(unittest.TestCase):
    def test_the_table_is_complete(self):
        t = triage.load_rules()
        ids = set()
        for r in t["rules"]:
            self.assertNotIn(r["id"], ids)
            ids.add(r["id"])
            self.assertTrue(r["all"] or r["any"], r["id"])
            self.assertGreaterEqual(len(r["asks"]), 3, r["id"])
            self.assertTrue(r["evidence"], r["id"])
            for p in r["all"] + r["any"] + r["none"] + r["evidence"]:
                re.compile(p)
        self.assertEqual(len(t["generic_asks"]), 5)

    def test_a_404_on_push_names_sync(self):
        found = triage.analyse(PUSH_404)
        h = triage.headline(found)
        self.assertEqual(h["id"], "sync-404-push")
        self.assertEqual(h["area"], "Sync (KOSync client)")
        self.assertEqual(len(h["asks"]), 5)
        self.assertTrue(any("/api/koreader" in e for e in h["evidence"]))

    def test_the_users_words_do_not_steer_the_rules(self):
        found = triage.analyse(PUSH_404)
        self.assertNotIn("network-timeout", [f["id"] for f in found])

    def test_an_event_beats_a_state(self):
        r = "- File system kind: NETWORK file system\n- 12:00:00Z sync/pull FAIL class=timeout 10000 ms\n"
        h = triage.headline(triage.analyse(r))
        self.assertEqual(h["id"], "network-timeout")
        self.assertEqual(h["area"], "Sync (KOSync client)")

    def test_a_generic_rule_names_the_failing_area(self):
        h = triage.headline(triage.analyse("- 12:00:00Z backup/job FAIL class=timeout 30000 ms\n"))
        self.assertEqual(h["area"], "Backups")

    def test_a_pasted_terminal_error(self):
        t = "bookx: /lib64/libc.so.6: version `GLIBC_2.43' not found (required by bookx)\n"
        h = triage.headline(triage.analyse(t))
        self.assertEqual(h["id"], "package-binary")
        self.assertEqual(h["severity"], "crash")

    def test_nothing_failed_still_gives_five_questions(self):
        out = triage.render("### Bookx\n\n- Version: 1\n")
        self.assertIn("not clear", out)
        self.assertEqual(len(re.findall(r"^\s+\d\. ", out, re.M)), 5)

    def test_the_text_output_has_the_parts_a_maintainer_reads(self):
        out = triage.render(PUSH_404)
        for want in ("Area: Sync (KOSync client)", "Severity: annoyance", "Suspected cause:", "Key facts:", "- Version: 0.3.0-beta.2", "ask the user"):
            self.assertIn(want, out)

    def test_the_json_output_is_valid(self):
        p = subprocess.run([sys.executable, os.path.join(HERE, "triage-report.py"), "--json"], input=PUSH_404, capture_output=True, text=True, check=True)
        import json
        self.assertEqual(json.loads(p.stdout)["headline"]["id"], "sync-404-push")

    def test_unfilled_placeholders_vanish(self):
        self.assertEqual(triage.fill("A {x} B ({y})", {"x": "1"}), "A 1 B ()")


if __name__ == "__main__":
    unittest.main()
