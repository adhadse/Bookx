import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
spec = importlib.util.spec_from_file_location("wiki_video", os.path.join(ROOT, "build-aux", "wiki_video.py"))
wv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wv)


class Durations(unittest.TestCase):
    def test_a_frame_lasts_until_the_next_capture(self):
        d = wv.frame_durations([0.0, 0.5, 1.5])
        self.assertEqual([i for i, _ in d], [0, 1, 2])
        self.assertAlmostEqual(d[0][1], 0.5)
        self.assertAlmostEqual(d[1][1], 1.0)
        self.assertAlmostEqual(d[2][1], 0.5)  # the last one gets the tail

    def test_frames_are_never_shorter_than_one_output_frame(self):
        d = wv.frame_durations([0.0, 0.001, 0.002])
        self.assertTrue(all(s >= wv.MIN_FRAME_SECONDS for _, s in d))

    def test_the_clip_is_cut_at_the_maximum_length(self):
        d = wv.frame_durations([0.0, 5.0, 11.0, 12.5, 14.0])
        self.assertEqual([i for i, _ in d], [0, 1, 2])
        self.assertLessEqual(sum(s for _, s in d), wv.MAX_SECONDS + 1e-6)

    def test_nothing_in_nothing_out(self):
        self.assertEqual(wv.frame_durations([]), [])
        self.assertEqual(wv.concat_script(["a"], []), "\n")


class Command(unittest.TestCase):
    def test_concat_list_repeats_the_last_frame(self):
        s = wv.concat_script(["/x/a", "/x/b"], [(0, 0.5), (1, 1.25)])
        self.assertEqual(s.splitlines(), ["file '/x/a'", "duration 0.500", "file '/x/b'", "duration 1.250", "file '/x/b'"])

    def test_the_filter_never_exceeds_the_maximum_width_and_stays_even(self):
        self.assertTrue(wv.scale_filter(1280).startswith(f"scale={wv.MAX_WIDTH}:"))
        self.assertTrue(wv.scale_filter(801).startswith("scale=800:"))
        self.assertIn(f"fps={wv.FPS}", wv.scale_filter(960))

    def test_vp9_without_audio_or_metadata(self):
        cmd = wv.ffmpeg_command("in.txt", "out.webm", 960)
        self.assertIn("libvpx-vp9", cmd)
        self.assertIn("-an", cmd)
        self.assertEqual(cmd[cmd.index("-map_metadata") + 1], "-1")
        self.assertEqual(cmd[-1], "out.webm")
        self.assertEqual(float(cmd[cmd.index("-t") + 1]), wv.MAX_SECONDS)


class Budget(unittest.TestCase):
    def test_a_clip_within_the_budget_passes(self):
        self.assertEqual(wv.budget_problems("a.webm", 300 * 1024, 10.0, 960), [])

    def test_each_limit_is_reported(self):
        self.assertEqual(len(wv.budget_problems("a.webm", wv.MAX_CLIP_BYTES + 1, 10.0, 960)), 1)
        self.assertEqual(len(wv.budget_problems("a.webm", 1, wv.MAX_SECONDS + 1, 960)), 1)
        self.assertEqual(len(wv.budget_problems("a.webm", 1, 1.0, wv.MAX_WIDTH + 2)), 1)
        self.assertEqual(len(wv.budget_problems("a.webm", wv.MAX_CLIP_BYTES + 1, 20.0, 1920)), 3)

    def test_the_total_is_limited_too(self):
        self.assertEqual(wv.total_problems([wv.MAX_CLIP_BYTES] * 6), [])
        self.assertEqual(len(wv.total_problems([wv.MAX_CLIP_BYTES] * 8)), 1)


class Manifest(unittest.TestCase):
    def test_every_listed_clip_has_a_scene_and_a_page(self):
        scenes = open(os.path.join(ROOT, "build-aux", "session", "scenarios_wiki_video.py"), encoding="utf8").read()
        sh = open(os.path.join(ROOT, "build-aux", "wiki-screenshots.sh"), encoding="utf8").read()
        listed = 0
        for line in open(os.path.join(ROOT, "build-aux", "wiki-videos.txt"), encoding="utf8"):
            if not line.strip() or line.startswith("#"):
                continue
            page, _, names = line.partition(":")
            self.assertTrue(os.path.exists(os.path.join(ROOT, "wiki", "docs", page.strip())), page)
            for n in names.split():
                listed += 1
                self.assertIn(f'"{n}"', scenes)
        self.assertGreaterEqual(listed, 3)
        for scene in ("_wiki_video_aloud", "_wiki_video_speed", "_wiki_video_ruler"):
            self.assertIn(f"def {scene}(", scenes)
            self.assertIn(scene, sh)


if __name__ == "__main__":
    unittest.main()
