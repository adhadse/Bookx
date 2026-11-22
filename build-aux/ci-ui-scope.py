#!/usr/bin/env python3
"""Does a change need the Broadway UI scenarios?  `ui=true|false` plus the reason, decided by code from the changed files.

    ci-ui-scope.py [--root DIR] FILE...        (or the file list on stdin with `-`)

Prints two lines: `ui=true|false` and `ui_reason=...` (the first matching path, or the areas that were skipped).

The rule is "run unless the change provably cannot reach what the scenarios see":

* the scenarios run the built app (`bookx`) and the helper programs of the workspace they call (`bookx-cli`, the example
  programs of `bookx-tts`, `bookx-plugin`, `bookx-comic`, `bookx-pdf`). Every workspace crate those depend on, through
  normal, build or dev dependencies, can change what a scenario sees: that set is READ FROM THE Cargo.toml FILES, not
  listed by hand, so a new crate is in it as soon as the app depends on it. Any changed file under such a crate counts
  (Markdown files excepted), tests and fixtures included;
* the app itself (`src/`, `build.rs`, `Cargo.toml`, `Cargo.lock`, the toolchain), `data/`, `po/` (translated strings are
  on screen), the scenario scripts and shard files, `build-aux/session/`, and `.github/workflows/ci.yml` count;
* documentation, the wiki, packaging, other workflows, issue templates, the KOReader plugin, release/Flatpak/wiki scripts and
  crates the app does not depend on do not;
* a file that matches nothing counts (fail safe). An empty change counts too: nothing can prove it harmless to a
  reviewer, and the run costs less than a missed regression. Main, tags, nightly and manual runs never come here.
"""
import os
import re
import sys

try:
    import tomllib
except ImportError:  # Python < 3.11
    tomllib = None

ROOTS = ["bookx", "bookx-cli", "bookx-tts", "bookx-plugin", "bookx-comic", "bookx-pdf"]

SKIP = [
    (r"^(docs|wiki|packaging|_developer|mockups|koreader-plugin)/", "docs, wiki, packaging, KOReader plugin"),
    (r"\.md$", "Markdown"),
    (r"^(LICENSE|NOTICE|CHANGELOG|SECURITY|CONTRIBUTING|CODE_OF_CONDUCT)", "licence and project files"),
    (r"^\.github/(ISSUE_TEMPLATE/|PULL_REQUEST_TEMPLATE|dependabot\.yml|FUNDING|CODEOWNERS)", "GitHub templates"),
    (r"^\.github/workflows/(?!ci\.yml$)", "other workflows"),
    (r"^build-aux/(release|flatpak|wiki|pin-screenshots|package-|macos|windows|tarball|check-|i18n|gen-potfiles|coverage|ci-|"
     r"lint-checks|plugin-tests|convert-parity|flathub|make-wiki|screenshot|com\.adhadse|cargo-sources|tests/)", "release, wiki, Flatpak, i18n and CI scripts"),
    (r"^(\.config/|deny\.toml|\.gitignore|\.editorconfig|rustfmt\.toml|clippy\.toml)", "tool settings"),
]
UI = [
    (r"^src/", "the app"),
    (r"^(build\.rs|Cargo\.toml|Cargo\.lock|rust-toolchain(\.toml)?)$", "build and dependencies"),
    (r"^(data|po)/", "data and translations"),
    (r"^build-aux/(ui-tests|ui-shard|session/|coverage-ui)", "the UI scenarios"),
    (r"^\.github/workflows/ci\.yml$", "ci.yml"),
]


def workspace(root):
    """{crate name: (directory, set of workspace crates it depends on)} from the Cargo.toml files."""
    if tomllib is None:
        return None
    files = [os.path.join(root, "Cargo.toml")]
    crates = os.path.join(root, "crates")
    if os.path.isdir(crates):
        files += [os.path.join(crates, d, "Cargo.toml") for d in sorted(os.listdir(crates))]
    out = {}
    for f in files:
        if not os.path.exists(f):
            continue
        with open(f, "rb") as fh:
            data = tomllib.load(fh)
        name = data.get("package", {}).get("name")
        if not name:
            continue
        rel = os.path.relpath(os.path.dirname(f), root)
        deps = set()
        sections = [data] + list(data.get("target", {}).values())
        for sec in sections:
            for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                deps |= set(sec.get(kind, {}))
        out[name] = (rel, deps)
    return out


def ui_crate_dirs(root):
    ws = workspace(root)
    if ws is None:
        return None
    seen, todo = set(), [r for r in ROOTS if r in ws]
    while todo:
        n = todo.pop()
        if n in seen or n not in ws:
            continue
        seen.add(n)
        todo += list(ws[n][1])
    return {ws[n][0] for n in seen if ws[n][0] != "."}, {ws[n][0] for n in ws if n not in seen and ws[n][0] != "."}


def classify(files, root="."):
    """(ui, reason)."""
    files = [f for f in files if f]
    if not files:
        return True, "no changed files (cannot show the change harmless)"
    graph = ui_crate_dirs(root)
    if graph is None:
        return True, "Python without tomllib: the crate graph is unknown"
    in_graph, outside = graph
    skipped = []
    for f in files:
        d = next((d for d in sorted(in_graph | outside, key=len, reverse=True) if f.startswith(d + "/")), None)
        if re.search(r"\.md$", f):
            skipped.append("Markdown")
            continue
        if d is not None and d in in_graph:
            return True, f"{f} (a crate the app depends on)"
        if d is not None:
            skipped.append(f"{d} (the app does not depend on it)")
            continue
        hit = next((why for rx, why in UI if re.search(rx, f)), None)
        if hit:
            return True, f"{f} ({hit})"
        why = next((why for rx, why in SKIP if re.search(rx, f)), None)
        if why is None:
            return True, f"{f} (unknown path: running to be safe)"
        skipped.append(why)
    return False, "no UI-relevant change: " + ", ".join(sorted(set(skipped)))


def main(argv):
    root = "."
    if argv[:1] == ["--root"]:
        root, argv = argv[1], argv[2:]
    files = sys.stdin.read().split("\n") if argv == ["-"] else argv
    ui, why = classify(files, root)
    print(f"ui={'true' if ui else 'false'}")
    print(f"ui_reason={'ran because ' if ui else 'skipped: '}{why}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
