import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "check-docs-links.py")
spec = importlib.util.spec_from_file_location("check_docs_links", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def tree(files):
    root = tempfile.mkdtemp()
    for rel, text in files.items():
        p = os.path.join(root, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf8") as f:
            f.write(text)
    return root


class Slugs(unittest.TestCase):
    def test_github_rules(self):
        self.assertEqual(mod.slug("Part 2: Not done (and why)"), "part-2-not-done-and-why")
        self.assertEqual(mod.slug("The `x` file"), "the-x-file")
        self.assertEqual(mod.slug("A / B"), "a--b")

    def test_duplicate_headings_get_a_suffix(self):
        self.assertEqual(mod.anchors("# A\n## A\n"), {"a", "a-1"})

    def test_code_in_a_heading_counts(self):
        self.assertEqual(mod.anchors("## The `a.b` file (v1)\n"), {"the-ab-file-v1"})

    def test_headings_in_code_fences_do_not_count(self):
        self.assertEqual(mod.anchors("```\n# Not a heading\n```\n# Real\n"), {"real"})


class Links(unittest.TestCase):
    def test_good_links_pass(self):
        root = tree({
            "README.md": "[a](docs/a.md) [b](docs/a.md#title) [web](https://x.example/y.md)\n",
            "docs/a.md": "# Title\n[back](../README.md) [self](#title)\n",
        })
        self.assertEqual(mod.run(root), [])

    def test_missing_file_is_reported(self):
        root = tree({"README.md": "[a](docs/gone.md)\n", "docs/a.md": "# T\n"})
        problems = mod.run(root)
        self.assertEqual(len(problems), 1)
        self.assertIn("README.md:1", problems[0])

    def test_missing_anchor_is_reported(self):
        root = tree({"README.md": "[a](docs/a.md#nope)\n", "docs/a.md": "# T\n"})
        self.assertIn("no heading", mod.run(root)[0])

    def test_links_in_code_are_ignored(self):
        root = tree({"README.md": "`[a](docs/gone.md)`\n```\n[b](docs/gone.md)\n```\n", "docs/a.md": "# T\n"})
        self.assertEqual(mod.run(root), [])

    def test_archive_and_outdated_are_frozen(self):
        root = tree({
            "README.md": "# R\n",
            "docs/archive/old.md": "[x](missing.md)\n",
            "docs/outdated/old.md": "[x](missing.md)\n",
        })
        self.assertEqual(mod.run(root), [])

    def test_the_wiki_must_not_link_into_docs(self):
        root = tree({
            "README.md": "# R\n",
            "docs/a.md": "# T\n",
            "wiki/docs/page.md": "see [notes](../../docs/a.md) and the docs folder\n",
        })
        self.assertIn("public wiki links into the internal docs/", mod.run(root)[0])

    def test_a_wiki_link_inside_the_wiki_is_fine(self):
        root = tree({"README.md": "# R\n", "docs/a.md": "# T\n", "wiki/docs/a.md": "[b](b.md#x) docs\n", "wiki/docs/b.md": "# B\n"})
        self.assertEqual(mod.run(root), [])


class StrictPaths(unittest.TestCase):
    def test_a_moved_file_named_in_backticks_is_found(self):
        root = tree({"README.md": "# R\n", "docs/a.md": "See `docs/moved.md` and `docs/a.md`.\n"})
        self.assertEqual(mod.run(root), [])
        problems = mod.run(root, strict=True)
        self.assertEqual(len(problems), 1)
        self.assertIn("docs/moved.md", problems[0])


if __name__ == "__main__":
    unittest.main()
