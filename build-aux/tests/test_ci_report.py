import importlib.util
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
FIX = os.path.join(HERE, "fixtures", "ci-report")
spec = importlib.util.spec_from_file_location("ci_report", os.path.join(ROOT, "build-aux", "ci-report.py"))
cr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cr)


def report(*extra):
    return subprocess.run(
        [sys.executable, os.path.join(ROOT, "build-aux", "ci-report.py"), "--jobs", os.path.join(FIX, "jobs.json"),
         "--artifacts", FIX, "--run-url", "https://github.com/adhadse/Bookx/actions/runs/1", *extra],
        check=True, capture_output=True, text=True,
    ).stdout


class Report(unittest.TestCase):
    def test_the_fixture_run_is_red_with_every_reason(self):
        md = report("--pr-comment")
        self.assertTrue(md.startswith(cr.MARKER))
        self.assertIn("CI is red", md)
        self.assertIn("clippy (warnings are errors)", md)  # the failed step
        self.assertIn("bookx-tts: 82.50% is 3.30 points below", md)
        self.assertIn("patch coverage 71% is below 80%", md)

    def test_tests_and_scenarios_are_counted(self):
        md = report()
        self.assertIn("2 passed, 1 failed, 1 skipped, 4 in all", md.replace("**", ""))
        self.assertIn("15 checks passed, 2 failed (2 shards)", md.replace("**", ""))
        self.assertIn("`shelves`: a shelf is created", md)
        self.assertNotIn("second failing check", md)  # only the first failing check of a scenario

    def test_coverage_table_has_arrows_and_the_total(self):
        md = report()
        self.assertIn("▲ +0.50", md)
        self.assertIn("▼ -3.30", md)
        self.assertIn("Logic tier total: 93.00%", md)
        self.assertIn("uncovered lines 10, 11, 12", md)

    def test_untrusted_text_is_made_harmless(self):
        md = report()
        self.assertNotIn("<img", md)
        self.assertNotIn("javascript:", md)
        self.assertNotIn("@everyone", md)
        self.assertEqual(cr.esc("a|b\nc`d<e>"), "a\\|b c'd&lt;e&gt;")

    def test_the_marker_is_only_in_a_comment(self):
        self.assertNotIn(cr.MARKER, report())

    def test_missing_artifacts_do_not_crash(self):
        md = cr.build(type("A", (), {"jobs": None, "artifacts": "/nonexistent", "baseline": None, "run_url": "", "sha": "", "pr_comment": False})())
        self.assertIn("No job information", md)
        self.assertIn("No coverage result", md)

    def test_a_running_job_means_not_green_yet(self):
        md = cr.build(type("A", (), {"jobs": os.path.join(FIX, "running.json"), "artifacts": "/nonexistent", "baseline": None, "run_url": "", "sha": "abc", "pr_comment": False})())
        self.assertIn("still running", md)

    def test_changes_decide_which_jobs_run(self):
        def run(files):
            out = subprocess.run([os.path.join(ROOT, "build-aux", "ci-changes.sh")], env={**os.environ, "FILES": files},
                                 check=True, capture_output=True, text=True).stdout
            return dict(line.split("=") for line in out.split())
        self.assertEqual(run("docs/a.md wiki/docs/x.md")["rust"], "false")
        self.assertEqual(run("docs/a.md wiki/docs/x.md")["wiki"], "true")
        self.assertEqual(run("src/a.rs")["rust"], "true")
        self.assertEqual(run("Cargo.lock")["flatpak"], "true")
        self.assertEqual(run("README.md")["flatpak"], "false")


if __name__ == "__main__":
    unittest.main()
