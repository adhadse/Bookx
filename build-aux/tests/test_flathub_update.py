"""Tests for build-aux/flathub-update.py.  Run: python3 -m unittest discover -s build-aux/tests -v"""
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("flathub_update", HERE.parent / "flathub-update.py")
fu = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fu)

OLD_COMMIT = "0" * 40
NEW_COMMIT = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"

# odd spacing, comments, CRLF-free; tag-pattern must not be mistaken for tag
MANIFEST = """\
# hand written by the owner   (keep me)
id: com.adhadse.Bookx
runtime: org.gnome.Platform
finish-args:
  # window
  - --share=ipc
  - --socket=wayland   # trailing comment
  - "--filesystem=xdg-documents"
  - --talk-name=org.gtk.vfs.*

modules:
  - name: espeak-ng
    sources:
      - type: archive
        url: https://github.com/espeak-ng/espeak-ng/archive/refs/tags/1.52.0.tar.gz
  - name: bookx
    sources:
      - type: git
        url:   https://github.com/adhadse/Bookx.git
        tag:    v0.2.0      # bumped by flathub-update
        commit:   "{old}"
        x-checker-data:
          type: git
          tag-pattern: ^v(\\d+\\.\\d+\\.\\d+)$
      - cargo-sources.json
""".format(old=OLD_COMMIT)

METAINFO = """<?xml version="1.0"?>
<component><releases>
<release version="0.3.1" date="2026-12-01"/><release version="0.3.0" date="2026-11-22"/>
</releases></component>"""

CHANGELOG = """# Changelog

## [Unreleased]

## [0.3.1] - 2026-12-01
### Fixed
- a thing

## [0.3.0] - 2026-11-22
- first
"""

DEV = json.dumps({"finish-args": ["--share=ipc", "--socket=wayland", "--filesystem=xdg-documents",
                                  "--talk-name=org.gtk.vfs.*"]})


class UpdateManifest(unittest.TestCase):
    def test_only_the_two_values_change(self):
        new, old_tag, old_commit = fu.update_manifest_text(MANIFEST, "v0.3.1", NEW_COMMIT)
        self.assertEqual((old_tag, old_commit), ("v0.2.0", OLD_COMMIT))
        before, after = MANIFEST.splitlines(), new.splitlines()
        self.assertEqual(len(before), len(after))
        changed = [(b, a) for b, a in zip(before, after) if b != a]
        self.assertEqual(len(changed), 2)
        self.assertEqual(changed[0][1], "        tag:    v0.3.1      # bumped by flathub-update")
        self.assertEqual(changed[1][1], f'        commit:   "{NEW_COMMIT}"')

    def test_everything_else_is_byte_identical(self):
        new, *_ = fu.update_manifest_text(MANIFEST, "v0.3.1", NEW_COMMIT)
        again, *_ = fu.update_manifest_text(new, "v0.2.0", OLD_COMMIT)
        self.assertEqual(again, MANIFEST)

    def test_crlf_and_unquoted_commit_survive(self):
        text = MANIFEST.replace(f'"{OLD_COMMIT}"', OLD_COMMIT).replace("\n", "\r\n")
        new, *_ = fu.update_manifest_text(text, "v0.3.1", NEW_COMMIT)
        self.assertIn(f"commit:   {NEW_COMMIT}\r\n", new)
        self.assertNotIn("\n\n", new.replace("\r\n", ""))
        self.assertEqual(new.count("\r\n"), text.count("\r\n"))

    def test_refuses_a_second_tag_line(self):
        with self.assertRaises(fu.Refused):
            fu.update_manifest_text(MANIFEST + "      - type: git\n        tag: v1.0.0\n", "v0.3.1", NEW_COMMIT)

    def test_refuses_a_second_commit_line(self):
        with self.assertRaises(fu.Refused):
            fu.update_manifest_text(MANIFEST + "        commit: abc\n", "v0.3.1", NEW_COMMIT)

    def test_refuses_when_a_line_is_missing(self):
        for key in ("tag:    v0.2.0", "commit:   "):
            text = "\n".join(l for l in MANIFEST.splitlines() if key.strip() not in l) + "\n"
            with self.assertRaises(fu.Refused):
                fu.update_manifest_text(text, "v0.3.1", NEW_COMMIT)

    def test_refuses_a_second_bookx_url(self):
        text = MANIFEST + "        url: https://github.com/adhadse/Bookx\n"
        with self.assertRaises(fu.Refused):
            fu.update_manifest_text(text, "v0.3.1", NEW_COMMIT)

    def test_refuses_without_the_bookx_url(self):
        with self.assertRaises(fu.Refused):
            fu.update_manifest_text(MANIFEST.replace("adhadse/Bookx", "someone/Else"), "v0.3.1", NEW_COMMIT)

    def test_tag_and_commit_are_validated(self):
        for bad in ("0.3.1", "v0.3", "v0.3.1-rc1", "v0.3.1; rm -rf", "", "latest"):
            with self.assertRaises(fu.Refused, msg=bad):
                fu.update_manifest_text(MANIFEST, bad, NEW_COMMIT)
        for bad in ("abc", NEW_COMMIT[:39], NEW_COMMIT + "0", NEW_COMMIT.upper(), "g" * 40, ""):
            with self.assertRaises(fu.Refused, msg=bad):
                fu.update_manifest_text(MANIFEST, "v0.3.1", bad)


class Checks(unittest.TestCase):
    def test_finish_args_and_diff(self):
        args = fu.finish_args_from_yaml(MANIFEST)
        self.assertEqual(args, ["--share=ipc", "--socket=wayland", "--filesystem=xdg-documents",
                                "--talk-name=org.gtk.vfs.*"])
        self.assertEqual(fu.diff_finish_args(fu.finish_args_from_json(DEV), args), [])
        dev = fu.finish_args_from_json(json.dumps({"finish-args": ["--share=ipc", "--device=all"]}))
        notes = fu.diff_finish_args(dev, args)
        self.assertIn("only in the development manifest: --device=all", notes)
        self.assertIn("only in the Flathub manifest: --socket=wayland", notes)

    def test_metainfo_release(self):
        self.assertTrue(fu.metainfo_has_release(METAINFO, "0.3.1"))
        self.assertFalse(fu.metainfo_has_release(METAINFO, "0.3.2"))

    def test_changelog_excerpt(self):
        self.assertEqual(fu.changelog_excerpt(CHANGELOG, "0.3.1"), "### Fixed\n- a thing")
        self.assertEqual(fu.changelog_excerpt(CHANGELOG, "0.3.0"), "- first")
        self.assertEqual(fu.changelog_excerpt(CHANGELOG, "9.9.9"), "")

    def test_flathub_json_opt_out(self):
        self.assertTrue(fu.flathub_json_status('{"disable-external-data-checker": true}').startswith("ok"))
        self.assertTrue(fu.flathub_json_status('{"only-arches": ["x86_64"]}').startswith("WARNING"))
        self.assertIn("not valid", fu.flathub_json_status("{"))

    def test_body_mentions_permissions(self):
        same = fu.build_body("v0.3.1", NEW_COMMIT, "x", [], "v0.2.0")
        self.assertIn("Permissions: unchanged", same)
        diff = fu.build_body("v0.3.1", NEW_COMMIT, "x", ["only in the Flathub manifest: --device=all"], "v0.2.0")
        self.assertIn("WARNING: permissions differ", diff)
        self.assertIn("--device=all", diff)
        self.assertIn("releases/tag/v0.3.1", same)


class RepositoryFiles(unittest.TestCase):
    """The real draft manifest and the development manifest, as the script reads them."""

    ROOT = HERE.parent.parent

    def test_the_draft_has_one_bookx_source_and_the_same_permissions(self):
        draft = (self.ROOT / "packaging/flathub/com.adhadse.Bookx.yml").read_text()
        new, *_ = fu.update_manifest_text(draft, "v9.9.9", NEW_COMMIT)
        self.assertIn("tag: v9.9.9", new)
        dev = fu.finish_args_from_json((self.ROOT / fu.DEV_MANIFEST).read_text())
        self.assertTrue(dev)
        self.assertEqual(fu.diff_finish_args(dev, fu.finish_args_from_yaml(draft)), [])

    def test_the_flathub_build_leaves_out_machine_translated_languages(self):
        draft = (self.ROOT / "packaging/flathub/com.adhadse.Bookx.yml").read_text()
        self.assertIn("--features bookx/reviewed-locales-only", draft)
        self.assertIn("CARGO_FEATURES=bookx/reviewed-locales-only", draft)


class EndToEnd(unittest.TestCase):
    """A throw-away git repository with a tag, the real script, --dry-run and a real run."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.repo = root / "src"
        (self.repo / "data").mkdir(parents=True)
        (self.repo / "build-aux").mkdir()
        (self.repo / "data/com.adhadse.Bookx.metainfo.xml").write_text(METAINFO)
        (self.repo / "build-aux/com.adhadse.Bookx.json").write_text(DEV)
        (self.repo / "CHANGELOG.md").write_text(CHANGELOG)
        (self.repo / "Cargo.lock").write_text("version = 4\n")
        git = ["git", "-C", str(self.repo), "-c", "user.name=t", "-c", "user.email=t@t"]
        subprocess.run(git[:3] + ["init", "-q"], check=True)
        subprocess.run(git + ["add", "."], check=True)
        subprocess.run(git + ["commit", "-q", "-m", "x"], check=True)
        subprocess.run(git + ["tag", "-a", "v0.3.1", "-m", "annotated"], check=True)
        self.commit = subprocess.run(git + ["rev-parse", "HEAD"], check=True, capture_output=True,
                                     text=True).stdout.strip()
        self.manifest = root / "flathub" / "com.adhadse.Bookx.yml"
        self.manifest.parent.mkdir()
        self.manifest.write_text(MANIFEST)

    def run_script(self, *extra, tag="v0.3.1"):
        return fu.main(["--manifest", str(self.manifest), "--repo", str(self.repo), "--tag", tag, *extra])

    def test_annotated_tag_resolves_to_the_commit(self):
        self.assertEqual(fu.resolve_commit(self.repo, "v0.3.1"), self.commit)
        with self.assertRaises(fu.Refused):
            fu.resolve_commit(self.repo, "v9.9.9")

    def test_dry_run_changes_nothing(self):
        self.assertEqual(self.run_script("--dry-run"), 0)
        self.assertEqual(self.manifest.read_text(), MANIFEST)

    def test_real_run_edits_the_manifest_and_writes_the_body(self):
        body = self.manifest.parent / "body.txt"
        gen = self.manifest.parent / "gen.py"
        gen.write_text("import sys, json\nopen(sys.argv[sys.argv.index('-o') + 1], 'w').write(json.dumps([{'type': 'x'}]))\n")
        self.assertEqual(self.run_script("--body-file", str(body), "--cargo-generator", str(gen)), 0)
        self.assertIn(f'commit:   "{self.commit}"', self.manifest.read_text())
        self.assertIn("tag:    v0.3.1      # bumped", self.manifest.read_text())
        self.assertEqual(json.loads((self.manifest.parent / "cargo-sources.json").read_text()), [{"type": "x"}])
        text = body.read_text()
        self.assertIn("Permissions: unchanged", text)
        self.assertIn("- a thing", text)

    def test_missing_release_entry_fails_with_exit_2(self):
        (self.repo / "data/com.adhadse.Bookx.metainfo.xml").write_text(METAINFO.replace('"0.3.1"', '"0.3.0b"'))
        self.assertEqual(self.run_script("--dry-run"), 2)
        self.assertEqual(self.manifest.read_text(), MANIFEST)

    def test_bad_tag_fails_before_anything_is_touched(self):
        self.assertEqual(self.run_script("--dry-run", tag="main"), 2)
        self.assertEqual(self.manifest.read_text(), MANIFEST)

    def test_check_flathub_json_mode(self):
        f = Path(self.tmp.name) / "flathub.json"
        f.write_text('{"only-arches": ["x86_64"]}')
        self.assertEqual(fu.main(["--check-flathub-json", str(f)]), 1)
        f.write_text('{"only-arches": ["x86_64"], "disable-external-data-checker": true}')
        self.assertEqual(fu.main(["--check-flathub-json", str(f)]), 0)


if __name__ == "__main__":
    unittest.main()
