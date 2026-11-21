#!/usr/bin/env python3
"""Prepares an update of the Flathub repository (flathub/com.adhadse.Bookx) for a Bookx release tag.

Deterministic and AI-free: plain text edits and a few checks, nothing else. Pure standard library.
Used by .github/workflows/flathub-update.yml; docs/flathub-automation.md describes the whole flow.

    build-aux/flathub-update.py --manifest FILE --repo . --tag v0.3.1 --dry-run

What it does, in this order:
  1. checks that the tag looks like vX.Y.Z and resolves it to a 40-character commit (`git rev-list -n1`);
  2. checks that data/*.metainfo.xml of the tagged source has a <release version="X.Y.Z"> entry
     (Flathub takes the release notes from there; a missing entry fails with exit code 2);
  3. edits ONLY the `tag:` and `commit:` lines of the Bookx git source in the manifest text. The file is not
     parsed and written back, so comments, quoting and spacing stay byte for byte as the owner wrote them.
     It refuses (exit 2) unless each line occurs exactly once and the Bookx URL occurs exactly once;
  4. regenerates cargo-sources.json from the tag's Cargo.lock with the official flatpak-cargo-generator
     (--cargo-generator PATH; the workflow pins its commit);
  5. compares `finish-args` of the development manifest (build-aux/com.adhadse.Bookx.json of the tag) with
     those of the Flathub manifest and reports a difference as a warning (it goes into the PR body);
  6. writes the pull request text (--body-file).

`--dry-run` changes no file: it prints the diff of the manifest and what would be done.
`--check-flathub-json FILE` only reports whether flathub.json opts out of Flathub's own update bot, which
must be off while this automation is in use (both would open a pull request for the same tag).
"""
import argparse
import difflib
import json
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

TAG_RE = re.compile(r"^v\d+\.\d+\.\d+$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
BOOKX_URL_RE = re.compile(r"^[ \t]*(?:-[ \t]+)?url:[ \t]*['\"]?https://github\.com/adhadse/Bookx(?:\.git)?['\"]?[ \t]*(?:#.*)?\r?$", re.M)
RELEASE_URL = "https://github.com/adhadse/Bookx/releases/tag/{tag}"
METAINFO = "data/com.adhadse.Bookx.metainfo.xml"
DEV_MANIFEST = "build-aux/com.adhadse.Bookx.json"


class Refused(Exception):
    """The input is not what the script expects: nothing is changed and the job fails."""


def validate_tag(tag: str) -> str:
    if not TAG_RE.match(tag):
        raise Refused(f"the tag must look like v1.2.3, got {tag!r}")
    return tag


def validate_commit(commit: str) -> str:
    if not COMMIT_RE.match(commit):
        raise Refused(f"not a full 40-character lowercase commit hash: {commit!r}")
    return commit


def resolve_commit(repo: Path, tag: str) -> str:
    """The commit a tag points at (an annotated tag is peeled), from the checked-out repository."""
    validate_tag(tag)
    try:
        out = subprocess.run(
            ["git", "-C", str(repo), "rev-list", "-n1", f"refs/tags/{tag}"],
            check=True, capture_output=True, text=True,
        ).stdout.strip()
    except subprocess.CalledProcessError as e:
        raise Refused(f"git cannot resolve the tag {tag} in {repo}: {e.stderr.strip()}") from e
    return validate_commit(out)


def _key_re(key: str) -> "re.Pattern[str]":
    return re.compile(
        rf"^(?P<pre>[ \t]*(?:-[ \t]+)?{key}:[ \t]*)(?P<q>[\"']?)(?P<val>[A-Za-z0-9._/+-]+)(?P=q)"
        rf"(?P<post>[ \t]*(?:#[^\r\n]*)?\r?)$",
        re.M,
    )


def update_manifest_text(text: str, tag: str, commit: str) -> tuple:
    """Returns (new text, old tag, old commit). Only the value of the two lines changes."""
    validate_tag(tag)
    validate_commit(commit)
    urls = len(BOOKX_URL_RE.findall(text))
    if urls != 1:
        raise Refused(f"expected the Bookx git url exactly once in the manifest, found {urls}")
    old = {}
    for key, new in (("tag", tag), ("commit", commit)):
        pat = _key_re(key)
        found = pat.findall(text)
        if len(found) != 1:
            raise Refused(f"expected exactly one `{key}:` line in the manifest, found {len(found)}")
        text = pat.sub(lambda m, new=new: m.group("pre") + m.group("q") + new + m.group("q") + m.group("post"), text)
        old[key] = found[0][2]
    return text, old["tag"], old["commit"]


def _args_from_lines(items: list) -> list:
    return [a.strip().strip("'\"") for a in items]


def finish_args_from_yaml(text: str) -> list:
    """The `finish-args:` list of a manifest, read line by line (no YAML library)."""
    out, inside = [], False
    for line in text.splitlines():
        if re.match(r"^finish-args:\s*(#.*)?$", line):
            inside = True
            continue
        if not inside:
            continue
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = re.match(r"^\s+-\s+(.*?)\s*(?:\s#.*)?$", line)
        if not m:
            break
        out.append(m.group(1))
    return _args_from_lines(out)


def finish_args_from_json(text: str) -> list:
    return _args_from_lines(json.loads(text).get("finish-args", []))


def diff_finish_args(dev: list, flathub: list) -> list:
    """Human readable differences between two finish-args lists (order does not matter)."""
    notes = []
    for a in sorted(set(dev) - set(flathub)):
        notes.append(f"only in the development manifest: {a}")
    for a in sorted(set(flathub) - set(dev)):
        notes.append(f"only in the Flathub manifest: {a}")
    return notes


def metainfo_has_release(xml_text: str, version: str) -> bool:
    root = ET.fromstring(xml_text)
    return any(r.get("version") == version for r in root.iter("release"))


def changelog_excerpt(text: str, version: str) -> str:
    """The section of CHANGELOG.md for a version (without its heading), or '' when there is none."""
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if re.match(rf"^##\s+\[?v?{re.escape(version)}\]?(\s|$)", line):
            start = i + 1
            break
    if start is None:
        return ""
    end = next((j for j in range(start, len(lines)) if lines[j].startswith("## ")), len(lines))
    return "\n".join(lines[start:end]).strip()


def flathub_json_status(text: str) -> str:
    try:
        data = json.loads(text)
    except ValueError as e:
        return f"flathub.json is not valid JSON: {e}"
    if data.get("disable-external-data-checker") is True:
        return "ok: flathub.json has disable-external-data-checker: true (Flathub's own update bot is off)"
    return ('WARNING: flathub.json lacks "disable-external-data-checker": true. Flathub\'s bot would open its own '
            "update pull request for every tag next to this workflow's. Add it (docs/flathub-automation.md).")


def build_body(tag: str, commit: str, excerpt: str, warnings: list, old_tag: str) -> str:
    version = tag[1:]
    lines = [
        f"Update Bookx from {old_tag} to {tag}.",
        "",
        f"Release: {RELEASE_URL.format(tag=tag)}",
        f"Commit: {commit}",
        "",
        "Changed by a script (build-aux/flathub-update.py in the Bookx repository): the tag and commit of the",
        "Bookx source, and cargo-sources.json regenerated from the Cargo.lock of the tag with",
        "flatpak-cargo-generator. Nothing else in the manifest is touched. No AI is involved in this pull request.",
        "",
    ]
    if warnings:
        lines += ["WARNING: permissions differ between the development manifest and this manifest:"]
        lines += [f"  - {w}" for w in warnings]
        lines += ["Check this before merging.", ""]
    else:
        lines += ["Permissions: unchanged (finish-args equal those of the development manifest of the tag).", ""]
    lines += [f"Changelog {version}:", "", excerpt or "(no entry for this version in CHANGELOG.md)", ""]
    return "\n".join(lines)


def regenerate_cargo_sources(generator: Path, cargo_lock: Path, out: Path, cwd: Path) -> int:
    subprocess.run(
        [sys.executable, str(generator), str(cargo_lock), "-o", str(out)],
        check=True, cwd=str(cwd),
    )
    data = json.loads(out.read_text(encoding="utf-8"))
    if not isinstance(data, list) or not data:
        raise Refused(f"{out} is empty after flatpak-cargo-generator")
    return len(data)


def main(argv: list) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--manifest", help="the manifest in the Flathub repository (edited in place)")
    ap.add_argument("--repo", default=".", help="Bookx checkout at the tag (default: .)")
    ap.add_argument("--tag", help="release tag, vX.Y.Z")
    ap.add_argument("--dry-run", action="store_true", help="change no file; print the diff")
    ap.add_argument("--cargo-generator", help="flatpak-cargo-generator.py (official tool); without it cargo-sources.json is left alone")
    ap.add_argument("--cargo-sources", help="output file (default: cargo-sources.json next to the manifest)")
    ap.add_argument("--body-file", help="write the pull request text here")
    ap.add_argument("--check-flathub-json", metavar="FILE", help="only report the opt-out state of flathub.json")
    a = ap.parse_args(argv)

    if a.check_flathub_json:
        msg = flathub_json_status(Path(a.check_flathub_json).read_text(encoding="utf-8"))
        print(msg)
        return 0 if msg.startswith("ok") else 1
    if not a.manifest or not a.tag:
        ap.error("--manifest and --tag are required")

    repo, manifest = Path(a.repo), Path(a.manifest)
    try:
        tag = validate_tag(a.tag)
        commit = resolve_commit(repo, tag)

        meta = repo / METAINFO
        if not metainfo_has_release(meta.read_text(encoding="utf-8"), tag[1:]):
            raise Refused(
                f"{METAINFO} at {tag} has no <release version=\"{tag[1:]}\"> entry. Flathub shows the release "
                "notes from it: add the entry, move the tag (or tag a fixed commit) and publish again."
            )

        with open(manifest, encoding="utf-8", newline="") as f:
            before = f.read()
        after, old_tag, old_commit = update_manifest_text(before, tag, commit)

        dev = finish_args_from_json((repo / DEV_MANIFEST).read_text(encoding="utf-8"))
        flat = finish_args_from_yaml(after)
        if not flat:
            raise Refused("no finish-args found in the Flathub manifest (unexpected layout)")
        warnings = diff_finish_args(dev, flat)

        changelog = repo / "CHANGELOG.md"
        excerpt = changelog_excerpt(changelog.read_text(encoding="utf-8"), tag[1:]) if changelog.exists() else ""
        body = build_body(tag, commit, excerpt, warnings, old_tag)
    except (Refused, OSError, ET.ParseError, ValueError) as e:
        print(f"flathub-update: refused: {e}", file=sys.stderr)
        return 2

    diff = "".join(difflib.unified_diff(
        before.splitlines(True), after.splitlines(True), f"a/{manifest.name}", f"b/{manifest.name}"))
    print(diff or "(manifest already at this tag and commit)", end="" if diff else "\n")
    for w in warnings:
        print(f"WARNING: finish-args differ: {w}", file=sys.stderr)

    sources = Path(a.cargo_sources) if a.cargo_sources else manifest.with_name("cargo-sources.json")
    if a.dry_run:
        print(f"dry run: {manifest} not written; "
              f"{'would regenerate ' + str(sources) if a.cargo_generator else 'cargo-sources.json not requested'}")
        if a.body_file:
            Path(a.body_file).write_text(body, encoding="utf-8")
        return 0

    with open(manifest, "w", encoding="utf-8", newline="") as f:
        f.write(after)
    if a.cargo_generator:
        try:
            n = regenerate_cargo_sources(Path(a.cargo_generator).resolve(), (repo / "Cargo.lock").resolve(),
                                         sources.resolve(), repo)
        except (Refused, subprocess.CalledProcessError, OSError, ValueError) as e:
            print(f"flathub-update: cargo sources failed: {e}", file=sys.stderr)
            return 2
        print(f"wrote {sources} ({n} sources)")
    if a.body_file:
        Path(a.body_file).write_text(body, encoding="utf-8")
    print(f"updated {manifest}: {old_tag} -> {tag}, {old_commit[:12]} -> {commit[:12]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
