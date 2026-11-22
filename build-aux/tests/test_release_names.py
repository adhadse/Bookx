import os
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(ROOT, "build-aux", "release-names.sh")


def run(d, version="0.3.0-beta.1", arch=None):
    cmd = [SCRIPT, d, version] + ([arch] if arch else [])
    return subprocess.run(cmd, capture_output=True, text=True)


def touch(d, *names):
    for n in names:
        open(os.path.join(d, n), "w").close()


RAW = [
    "bookx-0.3.0-beta.1-linux-x86_64.tar.gz",
    "bookx-0.3.0-beta.1-linux-x86_64.tar.gz.sha256",
    "bookx_0.3.0~beta.1-1_amd64.deb",
    "bookx-0.3.0-beta.1-1.x86_64.rpm",
    "Bookx-0.3.0-beta.1.flatpak",
]
WANT = [f"Bookx-0.3.0-beta.1-linux-x86_64.{e}" for e in ("deb", "flatpak", "rpm", "tar.gz")]


class ReleaseNames(unittest.TestCase):
    def test_every_file_gets_the_same_scheme(self):
        with tempfile.TemporaryDirectory() as d:
            touch(d, *RAW)
            r = run(d)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(sorted(os.listdir(d)), WANT)

    def test_running_twice_changes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            touch(d, *RAW)
            run(d)
            r = run(d)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(sorted(os.listdir(d)), WANT)

    def test_a_missing_kind_fails_instead_of_a_partial_release(self):
        with tempfile.TemporaryDirectory() as d:
            touch(d, *[n for n in RAW if not n.endswith(".rpm")])
            r = run(d)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn(".rpm", r.stderr)

    def test_two_packages_of_one_kind_fail(self):
        with tempfile.TemporaryDirectory() as d:
            touch(d, *RAW, "other.deb")
            self.assertNotEqual(run(d).returncode, 0)

    def test_arch_is_part_of_the_name(self):
        with tempfile.TemporaryDirectory() as d:
            touch(d, *RAW)
            self.assertEqual(run(d, arch="aarch64").returncode, 0)
            self.assertIn("Bookx-0.3.0-beta.1-linux-aarch64.flatpak", os.listdir(d))


if __name__ == "__main__":
    unittest.main()
