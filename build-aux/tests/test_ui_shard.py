import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
spec = importlib.util.spec_from_file_location("ui_shard", os.path.join(ROOT, "build-aux", "ui-shard.py"))
us = importlib.util.module_from_spec(spec)
spec.loader.exec_module(us)


class Shards(unittest.TestCase):
    def test_every_scenario_is_in_exactly_one_shard(self):
        allnames = us.names()
        shards, _ = us.assign(allnames, {}, 8)
        flat = [n for s in shards for n in s]
        self.assertEqual(sorted(flat), sorted(allnames))
        self.assertEqual(len(flat), len(set(flat)))

    def test_long_scenarios_are_spread_out(self):
        weights = {"a": 100, "b": 100, "c": 100, "d": 1, "e": 1, "f": 1}
        shards, load = us.assign(list(weights), weights, 3)
        self.assertEqual(sorted(len(s) for s in shards), [2, 2, 2])
        self.assertLessEqual(max(load) - min(load), 1)

    def test_a_scenario_without_a_time_counts_as_the_median(self):
        shards, _ = us.assign(["a", "b", "new"], {"a": 10, "b": 10}, 2)
        self.assertEqual(sum(len(s) for s in shards), 3)

    def test_assignment_is_stable(self):
        w = {n: i % 7 + 1 for i, n in enumerate(us.names())}
        self.assertEqual(us.assign(us.names(), w, 8), us.assign(us.names(), w, 8))


if __name__ == "__main__":
    unittest.main()
