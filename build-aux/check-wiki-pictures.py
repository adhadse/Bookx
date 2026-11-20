#!/usr/bin/env python3
"""Checks the pictures of the public wiki against build-aux/wiki-pictures.txt (standard library only):

  * every picture of the manifest exists as assets/<name>-light.png and -dark.png and is shown by its page, in both styles
    (`![..](assets/x-light.png#only-light)` and `![..](assets/x-dark.png#only-dark)`) with alt text;
  * every picture a page shows is in the manifest for that page, so a scene (build-aux/session/scenarios_wiki*.py) takes it;
  * no asset is unused (no orphans);
  * every scene named in the manifest's scene list exists in the scenario files and every picture's scene is run by
    build-aux/wiki-screenshots.sh;
  * the feature pages each have the pictures they should (a minimum per page);
  * the assets stay below the size budget (25 MB).

  build-aux/check-wiki-pictures.py [--budget-mb 25]
"""
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DOCS = os.path.join(ROOT, "wiki", "docs")
ASSETS = os.path.join(DOCS, "assets")
MANIFEST = os.path.join(ROOT, "build-aux", "wiki-pictures.txt")
SHOTS = os.path.join(ROOT, "build-aux", "wiki-screenshots.sh")
SCENES = os.path.join(ROOT, "build-aux", "session")
MIN_PICTURES = {"library.md": 8, "reading.md": 8, "editing.md": 8, "preferences.md": 6, "getting-started.md": 1,
                "read-aloud.md": 2, "devices-and-sharing.md": 2, "news-and-web.md": 1, "backup-and-restore.md": 1,
                "annotations.md": 1, "shortcuts.md": 1, "privacy.md": 1, "ai-assistants.md": 1,
                "plugins-and-patches.md": 1, "progress-sync.md": 1}


def manifest():
    pages = {}
    for line in open(MANIFEST, encoding="utf8"):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        page, _, names = line.partition(":")
        pages[page.strip()] = names.split()
    return pages


def main():
    budget = 25.0
    if "--budget-mb" in sys.argv:
        budget = float(sys.argv[sys.argv.index("--budget-mb") + 1])
    problems = []
    pages = manifest()
    listed = {n for v in pages.values() for n in v}
    # what each page shows
    shown = {}
    for f in sorted(os.listdir(DOCS)):
        if not f.endswith(".md"):
            continue
        text = open(os.path.join(DOCS, f), encoding="utf8").read()
        light = re.findall(r"!\[([^\]]*)\]\(assets/([a-z0-9-]+)-light\.png#only-light\)", text)
        dark = {m for m in re.findall(r"assets/([a-z0-9-]+)-dark\.png", text)}
        # HTML blocks (the landing page): <img class="only-light" src="assets/x-light.png">
        for name in re.findall(r'<img[^>]*src="assets/([a-z0-9-]+)-light\.png[^"]*"', text):
            shown.setdefault(f, []).append(name)
        for alt, name in light:
            shown.setdefault(f, []).append(name)
            if not alt.strip():
                problems.append(f"{f}: the picture {name} has no alt text")
            if name not in dark:
                problems.append(f"{f}: the picture {name} is shown in the light style only")
    # captions: a screenshot pair followed directly by an italic line puts the caption in the same paragraph as the
    # (hidden) second picture, so it runs on and wraps unevenly. Captions belong in <figure><figcaption>.
    for f in sorted(os.listdir(DOCS)):
        if not f.endswith(".md"):
            continue
        lines = open(os.path.join(DOCS, f), encoding="utf8").read().split("\n")
        for i, line in enumerate(lines[:-1]):
            if re.match(r"^\s*!\[[^\]]*\]\(assets/[a-z0-9-]+-dark\.png#only-dark\)\s*$", line):
                nxt = lines[i + 1]
                if re.match(r"^\s*\*[^ *]", nxt) or re.match(r"^\s*\*\*[^*]+\*[^*]", nxt):
                    problems.append(f"{f}:{i + 2}: an italic caption directly under a picture; use "
                                    f'<figure markdown="1"> ... <figcaption markdown="1">...</figcaption></figure>')
        text = "\n".join(lines)
        for m in re.finditer(r"<figure\b[^>]*>(.*?)</figure>", text, re.S):
            body = m.group(1)
            if "-light.png#only-light" in body and ("-dark.png#only-dark" not in body):
                problems.append(f"{f}: a figure shows the light picture without the dark one")
            if "<figcaption" not in body:
                problems.append(f"{f}: a figure without a figcaption")
    for page, names in pages.items():
        for n in names:
            for style in ("light", "dark"):
                if not os.path.exists(os.path.join(ASSETS, f"{n}-{style}.png")):
                    problems.append(f"{page}: assets/{n}-{style}.png is missing (run build-aux/wiki-screenshots.sh)")
            if n not in shown.get(page, []):
                problems.append(f"{page}: the manifest lists {n} but the page does not show it")
    for page, names in shown.items():
        for n in names:
            if n not in pages.get(page, []):
                problems.append(f"{page}: shows {n}, which build-aux/wiki-pictures.txt does not list for this page "
                                f"(no scene takes it)")
    for page, minimum in MIN_PICTURES.items():
        have = len(shown.get(page, []))
        if have < minimum:
            problems.append(f"{page}: {have} pictures, at least {minimum} expected")
    # orphans: an asset no page shows
    used = {n for v in shown.values() for n in v}
    for f in sorted(os.listdir(ASSETS)):
        m = re.match(r"^([a-z0-9-]+)-(light|dark)\.png$", f)
        if m and m.group(1) not in used:
            problems.append(f"assets/{f} is not used by any page (orphan)")
    # the scene script knows every picture (it fails when one was not taken)
    sh = open(SHOTS, encoding="utf8").read()
    if "wiki-pictures.txt" not in sh:
        problems.append("build-aux/wiki-screenshots.sh does not read wiki-pictures.txt")
    # every picture is taken by some scene: its name is a string in a scenario file (wshot, dialog_shot, popover_shot, or a
    # table of names; the theme and preferences pictures are built from a key, so their keys are checked)
    literals = set()
    src_all = ""
    for f in os.listdir(SCENES):
        if f.startswith("scenarios") and f.endswith(".py"):
            src_all += open(os.path.join(SCENES, f), encoding="utf8").read()
    literals.update(re.findall(r'"([a-z0-9-]+)"', src_all))
    taken = {n for n in listed if n in literals}
    taken.update(f"reader-theme-{k}" for k in ("light", "sepia", "dark") if f'"{k}"' in src_all)
    ext = open(os.path.join(SCENES, "scenarios_wiki_ext.py"), encoding="utf8").read()
    taken.update(f"preferences-{slug}" for slug in re.findall(r'\("[^"]+", "([a-z]+)"\)', ext.split("PREFS_PAGES")[1].split("]")[0]))
    for n in sorted(listed):
        if n not in taken:
            problems.append(f"no scene takes the picture {n}")
    total = sum(os.path.getsize(os.path.join(ASSETS, f)) for f in os.listdir(ASSETS)) / 1e6
    if total > budget:
        problems.append(f"the assets are {total:.1f} MB, over the {budget:.0f} MB budget")
    print(f"wiki pictures: {len(listed)} listed, {sum(len(v) for v in shown.values())} shown on pages, "
          f"assets {total:.1f} MB (budget {budget:.0f} MB)")
    for p in problems:
        print("  - " + p)
    print(f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
