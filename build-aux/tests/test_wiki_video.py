import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
spec = importlib.util.spec_from_file_location("wiki_video", os.path.join(ROOT, "build-aux", "wiki_video.py"))
wv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wv)


class Commands(unittest.TestCase):
    def test_mp4_is_h264_yuv420p_faststart_without_audio(self):
        cmd = wv.ffmpeg_command("in.mkv", "out.mp4", "mp4")
        self.assertIn("libx264", cmd)
        self.assertEqual(cmd[cmd.index("-pix_fmt") + 1], "yuv420p")
        self.assertEqual(cmd[cmd.index("-movflags") + 1], "+faststart")
        self.assertIn("-an", cmd)
        self.assertEqual(cmd[-1], "out.mp4")

    def test_webm_is_vp9_without_audio_or_metadata(self):
        cmd = wv.ffmpeg_command("in.mkv", "out.webm", "webm")
        self.assertIn("libvpx-vp9", cmd)
        self.assertIn("-an", cmd)
        self.assertEqual(cmd[cmd.index("-map_metadata") + 1], "-1")
        self.assertEqual(cmd[-1], "out.webm")

    def test_both_are_limited_in_length_width_and_rate(self):
        for fmt in wv.FORMATS:
            cmd = wv.ffmpeg_command("in.mkv", f"o.{fmt}", fmt)
            self.assertEqual(float(cmd[cmd.index("-t") + 1]), wv.MAX_SECONDS)
            vf = cmd[cmd.index("-vf") + 1]
            self.assertIn(f"min({wv.MAX_WIDTH},iw)", vf)
            self.assertIn(f"fps={wv.FPS}", vf)
            self.assertIn("yuv420p", vf)

    def test_an_unknown_format_is_refused(self):
        with self.assertRaises(ValueError):
            wv.ffmpeg_command("a", "b", "gif")

    def test_the_poster_is_one_webp_frame(self):
        cmd = wv.poster_command("in.mp4", "p.webp", 4.0)
        self.assertEqual(cmd[cmd.index("-frames:v") + 1], "1")
        self.assertIn("libwebp", cmd)

    def test_the_frame_rate_is_a_real_video_rate(self):
        self.assertTrue(24 <= wv.FPS <= 30)


class Retry(unittest.TestCase):
    def test_a_file_that_fits_is_kept(self):
        self.assertIsNone(wv.next_crf("mp4", 26, wv.MAX_CLIP_BYTES))

    def test_a_file_that_is_too_big_gets_a_higher_crf(self):
        self.assertGreater(wv.next_crf("mp4", 26, int(wv.MAX_CLIP_BYTES * 1.2)), 26)
        self.assertGreater(wv.next_crf("webm", 34, int(wv.MAX_CLIP_BYTES * 3)), wv.next_crf("webm", 34, int(wv.MAX_CLIP_BYTES * 1.2)))

    def test_the_quality_floor_stops_the_retries(self):
        self.assertIsNone(wv.next_crf("mp4", wv.MAX_CRF["mp4"], wv.MAX_CLIP_BYTES * 2))


class Faststart(unittest.TestCase):
    def test_index_first_is_faststart(self):
        self.assertTrue(wv.moov_before_mdat(b"\0\0\0\x18ftypisom....\0\0\0\x08moov....mdat"))

    def test_index_last_is_not(self):
        self.assertFalse(wv.moov_before_mdat(b"ftypisom....mdat......moov"))

    def test_no_index_is_not(self):
        self.assertFalse(wv.moov_before_mdat(b"ftypisom"))


class Frames(unittest.TestCase):
    LINES = ["#format: frame checksums", "#tb 0: 1/30", "0, 0, 0, 1, 100, aaa", "0, 1, 1, 1, 100, aaa", "0, 2, 2, 1, 100, bbb",
             "0, 3, 3, 1, 100, ccc"]

    def test_distinct_and_total_frames(self):
        self.assertEqual(wv.distinct_frames(self.LINES), (3, 4))

    def test_a_slideshow_repeats_frames_a_recording_of_motion_does_not(self):
        still = ["0, %d, %d, 1, 1, x%d" % (i, i, i // 10) for i in range(40)]
        moving = ["0, %d, %d, 1, 1, x%d" % (i, i, i) for i in range(40)]
        self.assertEqual(wv.distinct_frames(still)[0], 4)
        self.assertEqual(wv.distinct_frames(moving)[0], 40)

    def test_nothing(self):
        self.assertEqual(wv.distinct_frames([]), (0, 0))


class Budget(unittest.TestCase):
    def test_a_clip_within_the_budget_passes(self):
        self.assertEqual(wv.budget_problems("a.mp4", 300 * 1024, 10.0, 960, 30.0), [])

    def test_each_limit_is_reported(self):
        self.assertEqual(len(wv.budget_problems("a.mp4", wv.MAX_CLIP_BYTES + 1, 10.0, 960, 30.0)), 1)
        self.assertEqual(len(wv.budget_problems("a.mp4", 1, wv.MAX_SECONDS + 1, 960, 30.0)), 1)
        self.assertEqual(len(wv.budget_problems("a.mp4", 1, 1.0, wv.MAX_WIDTH + 2, 30.0)), 1)
        self.assertEqual(len(wv.budget_problems("a.mp4", 1, 1.0, 960, 15.0)), 1)
        self.assertEqual(len(wv.budget_problems("a.mp4", wv.MAX_CLIP_BYTES + 1, 20.0, 1920, 5.0)), 4)

    def test_an_unknown_frame_rate_is_not_a_problem(self):
        self.assertEqual(wv.budget_problems("a.mp4", 1, 1.0, 960, None), [])

    def test_the_total_is_limited_too(self):
        self.assertEqual(wv.total_problems([wv.MAX_CLIP_BYTES] * 6), [])
        self.assertEqual(len(wv.total_problems([wv.MAX_CLIP_BYTES] * 8)), 1)


class Recorder(unittest.TestCase):
    def test_the_gstreamer_command_records_a_pipewire_node_at_a_constant_rate(self):
        src = open(os.path.join(ROOT, "build-aux", "session", "screencast.py"), encoding="utf8").read()
        ns = {}
        start = src.index("def gst_command")
        exec(src[start:src.index("class Screencast")], ns)
        cmd = ns["gst_command"](42, "/x/o.mkv", 30)
        self.assertIn("path=42", cmd)
        self.assertIn("video/x-raw,framerate=30/1", cmd)
        self.assertIn("location=/x/o.mkv", cmd)
        self.assertIn("-e", cmd)  # EOS on SIGINT so the file is finished


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

    def test_the_scenes_record_the_screen_and_do_not_take_stills(self):
        scenes = open(os.path.join(ROOT, "build-aux", "session", "scenarios_wiki_video.py"), encoding="utf8").read()
        self.assertIn("Screencast(", scenes)
        self.assertNotIn("screenshot_to", scenes)

    def test_the_page_script_respects_reduced_motion_and_the_markup_has_no_autoplay(self):
        js = open(os.path.join(ROOT, "wiki", "docs", "assets", "videos.js"), encoding="utf8").read()
        self.assertIn("prefers-reduced-motion: reduce", js)
        self.assertNotIn("http", js.replace("http-equiv", ""))
        for page in ("reading.md", "read-aloud.md", "accessibility.md"):
            text = open(os.path.join(ROOT, "wiki", "docs", page), encoding="utf8").read()
            self.assertNotIn("autoplay", text)
            self.assertIn("data-clip", text)


if __name__ == "__main__":
    unittest.main()
