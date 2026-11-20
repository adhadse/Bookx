#!/usr/bin/python3
"""Tests of the pure parts of the session harness (run first by session-tests.sh)."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    import harness
except ImportError as e:  # no PIL / PyGObject: session-tests.sh skips itself before it gets here
    print("skipping harness self-test:", e)
    sys.exit(0)


class CropCentring(unittest.TestCase):
    def setUp(self):
        try:
            import scenarios_wiki
        except ImportError as e:
            self.skipTest(str(e))
        self.centred_box = scenarios_wiki.centred_box

    def test_padding_is_equal_when_there_is_room(self):
        self.assertEqual(self.centred_box((300, 200, 500, 400), 16, (1280, 800)), (284, 184, 516, 416))

    def test_a_box_near_the_left_edge_stays_centred(self):
        x0, y0, x1, y1 = self.centred_box((6, 200, 400, 500), 16, (1280, 800))
        self.assertEqual((x0, 400 - 0 + (6 - x0)), (0, 406))  # 6 px of room: 6 px on both sides
        self.assertEqual(x1 - 400, 6 - x0)

    def test_a_box_near_the_top_bar_stays_centred(self):
        _, y0, _, y1 = self.centred_box((300, 40, 500, 300), 16, (1280, 800))
        self.assertEqual(y0, 36)
        self.assertEqual(y1, 304)

    def test_minsize_grows_both_sides_equally(self):
        x0, _, x1, _ = self.centred_box((500, 300, 600, 400), 0, (1280, 800), (300, 0))
        self.assertEqual((x0, x1), (400, 700))

    def test_minsize_is_limited_by_the_nearer_edge(self):
        x0, _, x1, _ = self.centred_box((50, 300, 150, 400), 0, (1280, 800), (400, 0))
        self.assertEqual((x0, x1), (0, 200))


class RealComic(unittest.TestCase):
    def setUp(self):
        import importlib.util
        import tempfile
        spec = importlib.util.spec_from_file_location("make_real_comic", os.path.join(os.path.dirname(HERE_DIR), "make-real-comic.py"))
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def test_a_cached_page_with_the_wrong_hash_is_dropped(self):
        name, url, digest = self.m.PAGES[0]
        with open(os.path.join(self.tmp.name, name), "wb") as f:
            f.write(b"not the page")
        self.assertIsNone(self.m.page_bytes(name, url, digest, self.tmp.name, download=False))
        self.assertFalse(os.path.exists(os.path.join(self.tmp.name, name)))

    def test_without_pages_and_without_network_nothing_is_built(self):
        out = os.path.join(self.tmp.name, "x.cbz")
        self.assertFalse(self.m.build(out, os.path.join(self.tmp.name, "cache"), download=False))
        self.assertFalse(os.path.exists(out))

    def test_cbz_has_pages_and_comicinfo(self):
        import hashlib
        import zipfile
        cache = os.path.join(self.tmp.name, "cache")
        os.makedirs(cache)
        fake = []
        pages = []
        for name, url, _ in self.m.PAGES:  # the build accepts exactly the pinned hashes: patch them for fake pages
            data = name.encode() * 3
            with open(os.path.join(cache, name), "wb") as f:
                f.write(data)
            fake.append((name, url, hashlib.sha256(data).hexdigest()))
            pages.append(data)
        self.m.PAGES = fake
        out = os.path.join(self.tmp.name, "x.cbz")
        self.assertTrue(self.m.build(out, cache, download=False))
        names = zipfile.ZipFile(out).namelist()
        self.assertEqual(names, [f"p00{i}.jpg" for i in range(1, 6)] + ["ComicInfo.xml"])

    def test_pinned_pages_are_hashed_and_https(self):
        for name, url, digest in self.m.PAGES:
            self.assertTrue(url.startswith("https://upload.wikimedia.org/"))
            self.assertEqual(len(digest), 64)


HERE_DIR = os.path.dirname(os.path.abspath(__file__))


class RegistryDetection(unittest.TestCase):
    def test_the_failure_seen_in_a_real_run(self):
        log = "Gtk-WARNING **: Could not activate remote peer 'org.a11y.atspi.Registry': GDBus.Error:Timeout was reached"
        self.assertTrue(harness.registry_failure_in(log))

    def test_registry_not_activatable(self):
        self.assertTrue(harness.registry_failure_in(
            "The name org.a11y.atspi.Registry was not provided by any .service files"))

    def test_normal_logs_are_not_a_failure(self):
        self.assertFalse(harness.registry_failure_in(""))
        self.assertFalse(harness.registry_failure_in("Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display"))
        self.assertFalse(harness.registry_failure_in("atspi: registered org.a11y.atspi.Registry"))


if __name__ == "__main__":
    unittest.main()
