#!/usr/bin/env python3
"""May this commit be released?  The decision of the `gates` job of release.yml, testable offline.

    release-gate.py --repo OWNER/REPO --sha SHA [--skip]

Pushes to main do not run the full CI (docs/ci-gates.md): the pull request that was merged did, on a branch that branch
protection forced to be up to date with main, so its tree is the tree that main got. A commit may be released when ONE of
these holds, checked in this order:

 1. skip (the emergency input `skip_gates`): always, with a warning;
 2. the `required` check of CI succeeded on this very commit (a direct push, a manual run on it, a rerun);
 3. the latest completed `Nightly` run on this commit succeeded (the full suite ran on main at this commit);
 4. the commit is the merge of a pull request whose head commit has a successful `required` check AND whose head tree is
    identical to the tree of this commit (a squash merge of an up-to-date branch). A different tree means something
    changed between the test and the merge: nothing proved the result green.

Anything else fails with a message that says what was found. A `required` check that exists on the commit but did not
succeed fails at once: a red check is not "unknown".

Exit status 0 = release, 1 = refuse. The decision (`decide`) is a pure function of what the GitHub API and git said.
"""
import argparse
import json
import subprocess
import sys


def latest(runs, name):
    """Conclusion of the newest check run called `name`, or None."""
    mine = [r for r in runs if r.get("name") == name]
    if not mine:
        return None
    mine.sort(key=lambda r: r.get("started_at") or "")
    return mine[-1].get("conclusion") or "pending"


def decide(sha, skip, checks_on_sha, nightly_conclusions, pulls, head_checks, tree_of):
    """(ok, message).

    checks_on_sha: check runs of `sha`; nightly_conclusions: conclusions of completed Nightly runs on `sha`;
    pulls: pull requests of the commit (dicts with number, merged_at, merge_commit_sha, head_sha);
    head_checks: {head sha: check runs}; tree_of: function ref -> tree id (or None when unknown).
    """
    if skip:
        return True, "skip_gates: CI was not checked"
    here = latest(checks_on_sha, "required")
    if here == "success":
        return True, f"the `required` check of CI succeeded on {sha[:12]}"
    if here is not None:
        return False, f"the `required` check on {sha[:12]} is `{here}`: CI is not green for this commit"
    if nightly_conclusions and nightly_conclusions[0] == "success":
        return True, f"the latest Nightly run on {sha[:12]} succeeded (the full suite ran on main at this commit)"
    merged = [p for p in pulls if p.get("merged_at") and p.get("merge_commit_sha") == sha]
    if not merged:
        return False, (
            f"{sha[:12]} has no `required` check, no successful Nightly run and is not the merge commit of a pull request "
            "(a direct push?). Tag a commit that came in through a pull request, or run the workflow with skip_gates "
            "for an emergency"
        )
    problems = []
    for p in merged:
        head = p["head_sha"]
        res = latest(head_checks.get(head, []), "required")
        if res != "success":
            problems.append(f"pull request #{p['number']}: the `required` check on its head {head[:12]} is `{res or 'missing'}`")
            continue
        t_tag, t_head = tree_of(sha), tree_of(head)
        if not t_tag or not t_head:
            problems.append(f"pull request #{p['number']}: could not read the tree of {sha[:12]} or of its head {head[:12]}")
            continue
        if t_tag != t_head:
            problems.append(
                f"pull request #{p['number']}: its head {head[:12]} was green, but its tree ({t_head[:12]}) differs from the tree of "
                f"{sha[:12]} ({t_tag[:12]}): the branch was not up to date when it was merged, so the merged result was never tested"
            )
            continue
        return True, (
            f"pull request #{p['number']}: `required` succeeded on its head {head[:12]} and the head tree equals the tree of {sha[:12]}"
        )
    return False, "; ".join(problems)


# ---- the real world -------------------------------------------------------------------------------------------------

def gh(path):
    out = subprocess.run(["gh", "api", path], check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def git_tree(ref):
    r = subprocess.run(["git", "rev-parse", f"{ref}^{{tree}}"], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--sha", required=True)
    ap.add_argument("--skip", action="store_true")
    a = ap.parse_args(argv)
    if a.skip:
        ok, msg = decide(a.sha, True, [], [], [], {}, git_tree)
    else:
        checks = gh(f"repos/{a.repo}/commits/{a.sha}/check-runs?per_page=100").get("check_runs", [])
        nightly = [
            r.get("conclusion")
            for r in gh(f"repos/{a.repo}/actions/workflows/nightly.yml/runs?head_sha={a.sha}&status=completed&per_page=10").get("workflow_runs", [])
        ]
        pulls, head_checks = [], {}
        for p in gh(f"repos/{a.repo}/commits/{a.sha}/pulls"):
            pulls.append({"number": p["number"], "merged_at": p.get("merged_at"),
                          "merge_commit_sha": p.get("merge_commit_sha"), "head_sha": p["head"]["sha"]})
            if p.get("merged_at"):
                head_checks[p["head"]["sha"]] = gh(f"repos/{a.repo}/commits/{p['head']['sha']}/check-runs?per_page=100").get("check_runs", [])
                # the head commit is not in a normal clone of main: fetch the pull request ref
                subprocess.run(["git", "fetch", "--quiet", "origin", f"pull/{p['number']}/head"], check=False)
        ok, msg = decide(a.sha, False, checks, nightly, pulls, head_checks, git_tree)
    print(("OK: " if ok else "REFUSED: ") + msg)
    if not ok:
        print(f"::error::{msg}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
