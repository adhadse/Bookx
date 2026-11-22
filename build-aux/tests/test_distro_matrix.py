import os
import stat
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(ROOT, "build-aux", "distro-matrix.sh")

# a fake container engine: prints PASS, or FAIL when the image reference contains a word of $FAKE_FAIL
FAKE = """#!/usr/bin/env bash
image=""
for a in "$@"; do image="$a"; done
case "$*" in *"bash /inner.sh"*) image=$(printf '%s\\n' "$@" | tail -n 3 | head -n 1) ;; esac
for w in $FAKE_FAIL; do
  case "$image" in *"$w"*) echo "RESULT FAIL fake failure for $image"; exit 0 ;; esac
done
echo "RESULT PASS bookx-cli fake"
"""


def run(files=("a.deb", "a.rpm", "a-linux-x86_64.tar.gz"), fail="", only=None):
    with tempfile.TemporaryDirectory() as d:
        rel = os.path.join(d, "rel")
        os.mkdir(rel)
        for f in files:
            open(os.path.join(rel, f), "w").close()
        bin_dir = os.path.join(d, "bin")
        os.mkdir(bin_dir)
        podman = os.path.join(bin_dir, "podman")
        with open(podman, "w") as fh:
            fh.write(FAKE)
        os.chmod(podman, os.stat(podman).st_mode | stat.S_IEXEC)
        env = dict(os.environ, PATH=bin_dir + ":/usr/bin:/bin", FAKE_FAIL=fail, JOBS="4")
        if only:
            env["ONLY"] = only
        return subprocess.run([SCRIPT, rel], capture_output=True, text=True, env=env)


class DistroMatrix(unittest.TestCase):
    def test_all_pass(self):
        r = run()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("every supported image passed", r.stdout)

    def test_a_supported_failure_fails_the_release(self):
        r = run(fail="ubuntu:25.10")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("FAILED", r.stdout)

    def test_a_best_effort_failure_is_reported_but_not_fatal(self):
        r = run(fail="tumbleweed")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("best-effort", r.stdout)
        self.assertIn("FAIL", r.stdout)

    def test_a_missing_package_file_is_an_error(self):
        r = run(files=("a.deb", "a.rpm"))
        self.assertEqual(r.returncode, 2)

    def test_only_selects_images(self):
        r = run(only="fedora:43")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("ubuntu", r.stdout)


if __name__ == "__main__":
    unittest.main()
