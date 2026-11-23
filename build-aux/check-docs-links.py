#!/usr/bin/env python3
"""Checks the relative markdown links of the internal documentation.

    build-aux/check-docs-links.py                 # docs/*.md (living), docs/README.md, README.md
    build-aux/check-docs-links.py --strict-paths  # also: every `docs/....md` named in backticks exists

What is checked
  * every `[text](target)` whose target is not a URL: the file exists, and a `#anchor` names a heading
    of the target file (GitHub's slug rules);
  * the public wiki (`wiki/docs`) never links into the internal `docs/` folder with a relative path
    (use a GitHub URL there);
  * with --strict-paths, a path like `docs/foo.md` in backticks must exist (these are the pointers people
    forget to fix when a file moves).

`docs/archive/` and `docs/outdated/` are frozen: their text is never rewritten, so their links are not
checked (a banner on line 1 says where the living document is).
Exit status 1 when something is wrong. No network, no dependencies.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LINK = re.compile(r"(?<!\\)!?\[(?:[^\]\\]|\\.)*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
FENCE = re.compile(r"^\s*(```|~~~)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
CODE_SPAN = re.compile(r"`[^`\n]*`")
DOC_PATH = re.compile(r"`((?:docs/)[A-Za-z0-9_./-]+\.md)(?:#[^`]*)?`")


def slug(text):
    """GitHub's heading anchor: lowercase, drop punctuation, spaces to hyphens."""
    text = re.sub(r"<[^>]+>", "", text)
    text = re.sub(r"`", "", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = text.strip().lower()
    out = []
    for ch in text:
        if ch.isalnum() or ch in "-_":
            out.append(ch)
        elif ch == " ":
            out.append("-")
    return "".join(out)


def lines_outside_code(text):
    """Yields (line number, line with inline code removed, raw line) for lines outside fenced blocks."""
    fenced = False
    for n, raw in enumerate(text.splitlines(), 1):
        if FENCE.match(raw):
            fenced = not fenced
            continue
        if fenced:
            continue
        yield n, CODE_SPAN.sub("", raw), raw


def anchors(text):
    seen = {}
    found = set()
    for _n, _line, raw in lines_outside_code(text):
        m = HEADING.match(raw) if raw.startswith("#") else None
        if not m:
            continue
        s = slug(m.group(2))
        k = seen.get(s, 0)
        seen[s] = k + 1
        found.add(s if k == 0 else f"{s}-{k}")
    return found


def is_external(target):
    return re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", target) is not None or target.startswith("//")


def check_file(path, root, anchor_cache, wiki=False):
    problems = []
    text = open(path, encoding="utf8").read()
    base = os.path.dirname(path)
    for n, line, _raw in lines_outside_code(text):
        for m in LINK.finditer(line):
            target = m.group(1)
            if is_external(target):
                continue
            file_part, _, frag = target.partition("#")
            dest = path if file_part == "" else os.path.normpath(os.path.join(base, file_part))
            rel = os.path.relpath(path, root)
            if wiki:
                # build-aux/wiki.sh checks the wiki's own links and anchors; here only the split matters
                if os.path.relpath(dest, root).startswith("docs" + os.sep):
                    problems.append(f"{rel}:{n}: the public wiki links into the internal docs/ ({target}); use a GitHub URL")
                continue
            if not os.path.exists(dest):
                problems.append(f"{rel}:{n}: broken link ({target})")
                continue
            if frag and dest.endswith(".md") and os.path.isfile(dest):
                if dest not in anchor_cache:
                    anchor_cache[dest] = anchors(open(dest, encoding="utf8").read())
                if frag.lower() not in anchor_cache[dest]:
                    problems.append(f"{rel}:{n}: no heading for the anchor ({target})")
    return problems


def strict_paths(path, root):
    problems = []
    text = open(path, encoding="utf8").read()
    for n, _line, raw in lines_outside_code(text):
        for m in DOC_PATH.finditer(raw):
            if not os.path.exists(os.path.join(root, m.group(1))):
                problems.append(f"{os.path.relpath(path, root)}:{n}: `{m.group(1)}` does not exist")
    return problems


def living_files(root):
    files = [os.path.join(root, "README.md")]
    docs = os.path.join(root, "docs")
    for name in sorted(os.listdir(docs)):
        if name.endswith(".md"):
            files.append(os.path.join(docs, name))
    return [f for f in files if os.path.isfile(f)]


def wiki_files(root):
    out = []
    for d, _dirs, names in os.walk(os.path.join(root, "wiki", "docs")):
        out += [os.path.join(d, n) for n in names if n.endswith(".md")]
    return sorted(out)


def run(root, strict=False):
    cache = {}
    problems = []
    for f in living_files(root):
        problems += check_file(f, root, cache)
        if strict:
            problems += strict_paths(f, root)
    for f in wiki_files(root):
        problems += check_file(f, root, cache, wiki=True)
    return problems


def main(argv):
    strict = "--strict-paths" in argv
    problems = run(ROOT, strict)
    for p in problems:
        print(p, file=sys.stderr)
    if problems:
        print(f"check-docs-links: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print(f"check-docs-links: {len(living_files(ROOT))} files, links and anchors fine")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
