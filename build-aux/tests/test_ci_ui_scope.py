import importlib.util
import os
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
spec = importlib.util.spec_from_file_location("ui_scope", os.path.join(ROOT, "build-aux", "ci-ui-scope.py"))
us = importlib.util.module_from_spec(spec)
spec.loader.exec_module(us)


def ui(*files, root=ROOT):
    return us.classify(list(files), root)


class Classifier(unittest.TestCase):
    def test_documentation_wiki_and_packaging_skip(self):
        self.assertFalse(ui("docs/ci-gates.md")[0])
        self.assertFalse(ui("wiki/docs/install.md", "wiki/docs/assets/a.png")[0])
        self.assertFalse(ui("README.md", "CHANGELOG.md")[0])
        self.assertFalse(ui("packaging/flathub/com.adhadse.Bookx.yml")[0])
        self.assertFalse(ui("koreader-plugin/main.lua")[0])
        self.assertFalse(ui("crates/bookx-core/README.md")[0])

    def test_other_workflows_and_release_scripts_skip_but_ci_yml_runs(self):
        self.assertFalse(ui(".github/workflows/release.yml", ".github/workflows/nightly.yml")[0])
        self.assertFalse(ui(".github/ISSUE_TEMPLATE/bug.yml", ".github/dependabot.yml")[0])
        self.assertFalse(ui("build-aux/flatpak.sh", "build-aux/release-check.sh", "build-aux/wiki.sh", "build-aux/ci-changes.sh")[0])
        self.assertTrue(ui(".github/workflows/ci.yml")[0])

    def test_translations_data_and_the_app_run(self):
        self.assertTrue(ui("po/de.po")[0])
        self.assertTrue(ui("data/com.adhadse.Bookx.desktop")[0])
        self.assertTrue(ui("src/ui/reader.rs")[0])
        self.assertTrue(ui("src/style.css")[0])
        self.assertTrue(ui("build.rs")[0])
        self.assertTrue(ui("build-aux/ui-tests.sh")[0])
        self.assertTrue(ui("build-aux/ui-shard-weights.json")[0])
        self.assertTrue(ui("build-aux/session/harness.py")[0])

    def test_every_crate_the_app_reaches_runs_and_the_reason_names_the_file(self):
        ok, why = ui("crates/bookx-core/src/opf.rs")
        self.assertTrue(ok)
        self.assertIn("crates/bookx-core/src/opf.rs", why)
        for crate in ("bookx-text", "bookx-formats", "bookx-cli", "bookx-mcp"):
            self.assertTrue(ui(f"crates/{crate}/src/lib.rs")[0], crate)
        self.assertTrue(ui("crates/bookx-serve/tests/fixtures/x.lua")[0])

    def test_dependency_files_run(self):
        self.assertTrue(ui("Cargo.lock")[0])
        self.assertTrue(ui("Cargo.toml")[0])
        self.assertTrue(ui("crates/bookx-pdf/Cargo.toml")[0])

    def test_unknown_paths_and_an_empty_change_run(self):
        ok, why = ui("something/new/thing.bin")
        self.assertTrue(ok)
        self.assertIn("unknown path", why)
        self.assertTrue(ui()[0])
        self.assertTrue(ui("")[0])

    def test_one_relevant_file_among_many_skipped_ones_runs(self):
        self.assertTrue(ui("docs/a.md", "wiki/x.md", "po/fr.po")[0])

    def test_the_skip_reason_lists_the_areas(self):
        ok, why = ui("docs/a.md", "packaging/x")
        self.assertFalse(ok)
        self.assertIn("docs, wiki, packaging", why)

    def test_a_crate_the_app_does_not_depend_on_skips(self):
        with tempfile.TemporaryDirectory() as d:
            def w(path, text):
                os.makedirs(os.path.dirname(os.path.join(d, path)), exist_ok=True)
                with open(os.path.join(d, path), "w") as fh:
                    fh.write(text)
            w("Cargo.toml", '[package]\nname = "bookx"\nversion = "1"\n[dependencies]\nbookx-core = { path = "crates/bookx-core" }\n')
            w("crates/bookx-core/Cargo.toml", '[package]\nname = "bookx-core"\nversion = "1"\n')
            w("crates/bookx-cli/Cargo.toml", '[package]\nname = "bookx-cli"\nversion = "1"\n[dependencies]\nbookx-core = { path = "../bookx-core" }\n')
            w("crates/bookx-tool/Cargo.toml", '[package]\nname = "bookx-tool"\nversion = "1"\n[dependencies]\nbookx-core = { path = "../bookx-core" }\n')
            self.assertTrue(ui("crates/bookx-core/src/a.rs", root=d)[0])
            self.assertTrue(ui("crates/bookx-cli/src/a.rs", root=d)[0])  # the scenarios call it
            ok, why = ui("crates/bookx-tool/src/a.rs", root=d)  # nothing the app or the scenarios use depends on it
            self.assertFalse(ok)
            self.assertIn("the app does not depend on it", why)

    def test_the_real_graph_has_the_app_crates(self):
        dirs, outside = us.ui_crate_dirs(ROOT)
        self.assertIn("crates/bookx-core", dirs)
        self.assertIn("crates/bookx-text", dirs)
        self.assertEqual(outside, set(), "a crate the app does not reach: add a test for it")


class Wrapper(unittest.TestCase):
    def run_sh(self, **env):
        out = subprocess.run([os.path.join(ROOT, "build-aux", "ci-changes.sh")], env={**os.environ, **env},
                             check=True, capture_output=True, text=True).stdout
        return dict(line.split("=", 1) for line in out.splitlines())

    def test_docs_only_skips_the_ui(self):
        r = self.run_sh(FILES="docs/a.md wiki/docs/b.md")
        self.assertEqual((r["rust"], r["ui"]), ("false", "false"))
        self.assertTrue(r["ui_reason"].startswith("skipped"))

    def test_a_source_change_runs_it_and_says_why(self):
        r = self.run_sh(FILES="src/ui/reader.rs")
        self.assertEqual(r["ui"], "true")
        self.assertIn("src/ui/reader.rs", r["ui_reason"])

    def test_forcing_runs_it_even_for_docs(self):
        r = self.run_sh(FILES="docs/a.md", FORCE_UI="true")
        self.assertEqual(r["ui"], "true")
        self.assertIn("forced", r["ui_reason"])

    def test_anything_but_a_pull_request_runs_everything(self):
        r = self.run_sh(EVENT="push")
        self.assertEqual((r["rust"], r["ui"]), ("true", "true"))


if __name__ == "__main__":
    unittest.main()
