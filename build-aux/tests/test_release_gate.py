import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
spec = importlib.util.spec_from_file_location("release_gate", os.path.join(ROOT, "build-aux", "release-gate.py"))
rg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rg)

SHA = "a" * 40
HEAD = "b" * 40


def run(name, conclusion, at="2026-11-22T10:00:00Z"):
    return {"name": name, "conclusion": conclusion, "started_at": at}


def pull(number=7, merged=True, merge_sha=SHA, head=HEAD):
    return {"number": number, "merged_at": "2026-11-22T11:00:00Z" if merged else None, "merge_commit_sha": merge_sha, "head_sha": head}


def trees(**by):
    return lambda ref: by.get(ref)


class Gate(unittest.TestCase):
    def test_skip_gates_always_passes(self):
        ok, msg = rg.decide(SHA, True, [], [], [], {}, trees())
        self.assertTrue(ok)
        self.assertIn("skip_gates", msg)

    def test_a_green_required_check_on_the_commit_is_enough(self):
        ok, msg = rg.decide(SHA, False, [run("required", "success")], [], [], {}, trees())
        self.assertTrue(ok)
        self.assertIn("`required`", msg)

    def test_a_red_required_check_on_the_commit_refuses_even_with_a_green_pull_request(self):
        ok, msg = rg.decide(SHA, False, [run("required", "failure")], [], [pull()], {HEAD: [run("required", "success")]}, trees(**{SHA: "t", HEAD: "t"}))
        self.assertFalse(ok)
        self.assertIn("failure", msg)

    def test_a_skipped_required_check_on_the_commit_is_no_check_so_the_merged_pull_request_decides(self):
        # a push to main reports `required` as skipped (it only warms the caches): found when v0.3.0-beta.3 was refused
        green = {HEAD: [run("required", "success")]}
        ok, msg = rg.decide(SHA, False, [run("required", "skipped")], [], [pull()], green, trees(**{SHA: "t", HEAD: "t"}))
        self.assertTrue(ok, msg)
        self.assertIn("#7", msg)
        # and without a pull request behind it a skipped check proves nothing
        self.assertFalse(rg.decide(SHA, False, [run("required", "skipped")], [], [], {}, trees())[0])
        # cancelled and timed out are red, not skipped
        for bad in ("cancelled", "timed_out", "failure"):
            self.assertFalse(rg.decide(SHA, False, [run("required", bad)], [], [pull()], green, trees(**{SHA: "t", HEAD: "t"}))[0], bad)

    def test_the_newest_required_check_counts(self):
        runs = [run("required", "failure", "2026-11-22T10:00:00Z"), run("required", "success", "2026-11-22T10:30:00Z")]
        self.assertTrue(rg.decide(SHA, False, runs, [], [], {}, trees())[0])

    def test_a_successful_nightly_run_on_the_commit_is_enough(self):
        ok, msg = rg.decide(SHA, False, [], ["success"], [], {}, trees())
        self.assertTrue(ok)
        self.assertIn("Nightly", msg)

    def test_a_failed_nightly_alone_is_not_enough(self):
        self.assertFalse(rg.decide(SHA, False, [], ["failure"], [], {}, trees())[0])

    def test_a_merged_pull_request_with_an_equal_tree_passes(self):
        ok, msg = rg.decide(SHA, False, [], [], [pull()], {HEAD: [run("required", "success")]}, trees(**{SHA: "tree1", HEAD: "tree1"}))
        self.assertTrue(ok)
        self.assertIn("#7", msg)

    def test_different_trees_refuse_and_say_why(self):
        ok, msg = rg.decide(SHA, False, [], [], [pull()], {HEAD: [run("required", "success")]}, trees(**{SHA: "tree1", HEAD: "tree2"}))
        self.assertFalse(ok)
        self.assertIn("differs", msg)
        self.assertIn("not up to date", msg)

    def test_a_pull_request_whose_head_was_red_or_unchecked_refuses(self):
        ok, msg = rg.decide(SHA, False, [], [], [pull()], {HEAD: [run("required", "failure")]}, trees(**{SHA: "t", HEAD: "t"}))
        self.assertFalse(ok)
        self.assertIn("`failure`", msg)
        ok, msg = rg.decide(SHA, False, [], [], [pull()], {HEAD: []}, trees(**{SHA: "t", HEAD: "t"}))
        self.assertFalse(ok)
        self.assertIn("missing", msg)

    def test_no_pull_request_refuses(self):
        ok, msg = rg.decide(SHA, False, [], [], [], {}, trees())
        self.assertFalse(ok)
        self.assertIn("direct push", msg)

    def test_an_open_pull_request_or_another_merge_commit_does_not_count(self):
        self.assertFalse(rg.decide(SHA, False, [], [], [pull(merged=False)], {HEAD: [run("required", "success")]}, trees(**{SHA: "t", HEAD: "t"}))[0])
        self.assertFalse(rg.decide(SHA, False, [], [], [pull(merge_sha="c" * 40)], {HEAD: [run("required", "success")]}, trees(**{SHA: "t", HEAD: "t"}))[0])

    def test_an_unreadable_tree_refuses(self):
        ok, msg = rg.decide(SHA, False, [], [], [pull()], {HEAD: [run("required", "success")]}, trees(**{SHA: "t"}))
        self.assertFalse(ok)
        self.assertIn("could not read the tree", msg)


if __name__ == "__main__":
    unittest.main()
