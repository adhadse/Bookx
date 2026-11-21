# Flathub update automation

Internal (not for the wiki). After Flathub accepted Bookx, every release needs a pull request in
`flathub/com.adhadse.Bookx`. `.github/workflows/flathub-update.yml` opens it. It is a custom workflow in the sense of the
Flathub docs (<https://docs.flathub.org/docs/for-app-authors/updates>), allowed as long as it is not auto-merged.

Rules this setup keeps (Flathub policy, read 2026-11-22):

- **Deterministic, no AI.** `build-aux/flathub-update.py` edits two lines of text and runs the official
  `flatpak-cargo-generator`. Nothing generates prose or manifest content. Say in the submission PR (or on Matrix) that
  update pull requests come from this workflow; the pull request text says so too.
- **The initial submission is not automated.** The owner writes the manifest and opens the `flathub/flathub` pull
  request by hand (`docs/flathub-submission.md`). This workflow only works on the existing
  `flathub/com.adhadse.Bookx` repository.
- **No auto-merge.** The workflow only opens the pull request. Flathub builds a test build for it; the owner installs
  it, tests it and merges.
- **Flathub's own checker is switched off** (`disable-external-data-checker`) so that a tag does not produce two pull requests.

## One-time setup (owner)

1. **Wait until the Flathub repository exists** (submission accepted, invitation accepted).
2. **Create a fine-grained personal access token** (GitHub → Settings → Developer settings → Fine-grained tokens):
   - Resource owner: the user that owns the Flathub write access (you), or the `flathub` organisation if it lets you.
   - Repository access: *Only select repositories* → `flathub/com.adhadse.Bookx`.
   - Permissions: **Contents: Read and write**, **Pull requests: Read and write**; Metadata read is added automatically. Nothing else.
   - Expiry: one year; put the renewal date in your calendar (an expired token makes the job fail at the checkout).
   - `create-pull-request` pushes the branch `update-vX.Y.Z` to the repository, so the token needs push rights there.
     If Flathub does not give your account write access to the repository itself (it normally does for maintainers),
     fork it and use the fork-based setup of the action instead (not wired here; open an issue to extend the workflow).
3. **Add the secret** to `adhadse/Bookx`: Settings → Secrets and variables → Actions → *New repository secret*,
   name `FLATHUB_UPDATE_TOKEN`. Do not reuse this token anywhere else.
4. **Opt out of Flathub's bot.** In `flathub/com.adhadse.Bookx`, `flathub.json` must contain
   `"disable-external-data-checker": true` (alongside `only-arches` while it is there). Do this in a pull request of
   your own to that repository, merged before the first real run. Without it the workflow warns
   (`::warning::flathub.json lacks …`) and Flathub's bot opens a second, incomplete pull request for each tag.
   Consequence: the `espeak-ng` `anitya` bump no longer happens by itself; check the espeak-ng version when you bump the runtime.
5. **First run as a dry run.** Actions → *flathub-update* → *Run workflow*: tag of the latest release, `dry_run` on
   (default). It needs no secret, opens nothing, and uploads the artifact `flathub-update-<tag>` with
   `flathub-update.diff` (the whole change: manifest and `cargo-sources.json`)
   and `pr-body.txt` (the pull request text). Read both. Fix findings before relying on it.
6. **First real run:** the next published release, or *Run workflow* with `dry_run` off for a tag that already has
   an entry in the manifest older than it. Watch the job once.

Local check of the script at any time (nothing is written with `--dry-run`):

```bash
git clone https://github.com/flathub/com.adhadse.Bookx /tmp/opencode/flathub-repo
python3 build-aux/flathub-update.py --manifest /tmp/opencode/flathub-repo/com.adhadse.Bookx.yml --repo . --tag v0.3.1 --dry-run
python3 build-aux/flathub-update.py --check-flathub-json /tmp/opencode/flathub-repo/flathub.json
python3 -m unittest discover -s build-aux/tests -v        # the script's tests, also run by ci.yml
```

## How an update flows

1. Run `build-aux/release-check.sh`, tag `vX.Y.Z` (annotated), push the tag. The metainfo of that commit must have
   `<release version="X.Y.Z">` (the script fails otherwise) and `CHANGELOG.md` a `## [X.Y.Z]` section (used in the PR text; the job still works without).
2. `release.yml` builds and publishes the GitHub release. (`release.yml` is owned by the release process; this
   workflow only reacts to the **published** release event, pre-releases are skipped.)
3. `flathub-update.yml` starts on `release: published` (a release published by `GITHUB_TOKEN` inside a workflow does
   not trigger other workflows; if `release.yml` ever publishes with the default token, start this one by hand with *Run workflow*).
   It checks out the script, the Bookx tag, `flathub/com.adhadse.Bookx` (default branch `master`) and the pinned
   `flatpak-builder-tools`, then runs the script:
   - `tag` and `commit` (full 40-character hash from `git rev-list -n1 refs/tags/TAG`) are replaced in the Bookx source;
     the text is not re-serialised, so comments and formatting stay. It fails if the line or the Bookx URL is not found exactly once.
   - `cargo-sources.json` is regenerated for the `Cargo.lock` of the tag (official generator; our
     `build-aux/cargo-sources.json` is only for the development manifest. Both are equivalent in effect: same crates and hashes, the official file omits the default `strip-components: 1`).
   - `finish-args` of `build-aux/com.adhadse.Bookx.json` at the tag are compared with those of the Flathub manifest; a
     difference is a warning at the top of the PR text.
4. `peter-evans/create-pull-request` commits only `com.adhadse.Bookx.yml` and `cargo-sources.json` to the branch
   `update-vX.Y.Z` and opens *Update Bookx to vX.Y.Z* against `master`. `flathub.json` is never changed.
5. Flathub's CI builds a **test build** for the pull request (the buildbot comments with an install command; comment
   `bot, build` to start one again). Install it (`flatpak install --user <test repo URL> com.adhadse.Bookx`), open a
   book, read aloud, sync a device, check Preferences. Check the permissions warning.
6. The owner merges by hand. The official build is published one to two hours later; `flatpak update` picks it up.

## Runtime bump (manual, every GNOME release, about twice a year)

The workflow never touches the runtime. When a new GNOME runtime is out (and before the old one is end-of-life):

1. Build and test locally with the new runtime (`build-aux/flatpak.sh`, manifests `runtime-version`, the
   `rust-stable` extension version that matches, `package-flatpak.yml` container tag) in the Bookx repository first.
2. In the Flathub repository open a pull request that changes `runtime-version` and `sdk-extensions` by hand; let the
   Flathub test build pass; merge.
3. Update `packaging/flathub/com.adhadse.Bookx.yml` and the development manifest in Bookx so `release-check.sh` stays green.
4. Also look at `espeak-ng` (`anitya` is off while the Flathub bot is disabled) and bump it by hand if needed.

## Rolling back

- **Before merging:** close the pull request; delete the branch `update-vX.Y.Z` in the Flathub repository. Re-running the
  workflow for the same tag (workflow_dispatch) recreates it.
- **After merging, a bad build is published:** in the Flathub repository revert the merge (a new pull request that
  restores the previous `tag`, `commit` and `cargo-sources.json`), or better, fix forward with a new tag
  `vX.Y.Z+1`. Users who already updated can go back with `flatpak update --commit=<hash> com.adhadse.Bookx`
  (`flatpak remote-info --log flathub com.adhadse.Bookx` lists the hashes), and `flatpak mask` stops updates while you fix it.
  Flathub can also pull a build; ask in the Flathub Matrix room.
- **A wrong tag was released:** never move a published tag. Release the next patch version.

## Failure modes

| Symptom | Cause | What to do |
|---|---|---|
| Job fails in the first step: tag is not vX.Y.Z | a tag like `v0.3.1-rc1`, or a typo in the dispatch input | only `vX.Y.Z` goes to Flathub; fix the input |
| `secret FLATHUB_UPDATE_TOKEN is not set` | setup step 3 not done, or the run is on a fork | add the secret, or run as a dry run |
| Checkout of the Flathub repository fails (404/403) | token expired, wrong repository selected, or insufficient permissions | recreate the token (setup step 2) and replace the secret |
| `refused: … has no <release version="X.Y.Z"> entry` | metainfo of the tag lacks the release | the metainfo is what Flathub shows; fix it, tag a fixed commit (next patch version), publish |
| `refused: expected exactly one tag:/commit: line` or `…url exactly once` | the owner changed the manifest layout (second Bookx source, a `tag:` elsewhere) | adjust the manifest, or extend the script and its tests together |
| `finish-args differ` warning in the pull request | the development manifest of the tag asks for a permission the Flathub manifest does not have (or the reverse) | decide on purpose: edit the Flathub manifest in the pull request branch, or fix the development manifest. Never merge a permission change unseen. |
| `cargo sources failed` | the generator or its dependencies changed or crates.io is unreachable | re-run; if it persists run the generator by hand (`docs/flathub-submission.md`, step 5) and push to the PR branch; to update the pinned generator, change the commit in the workflow |
| Two pull requests for one tag | Flathub's bot is still on | add `disable-external-data-checker` (setup step 4), close the bot's pull request |
| No pull request, job green | nothing changed (the tag is already in the manifest), or a dry run | check the job log |
| No run after publishing | release created by a workflow with `GITHUB_TOKEN`, or the release was a pre-release/draft | start it by hand |
| Flathub test build fails | the build itself (new dependency, runtime change) | read the build log in the pull request; fix in Bookx and tag the next patch version |

## Security notes

- The token can write to the Flathub repository only; it is used by the checkout and by `create-pull-request`, nothing
  else (all other steps run without it; `persist-credentials: false`). Workflow permissions stay `contents: read`.
- Every action is pinned by commit SHA; `flatpak-builder-tools` is pinned to a commit. Its dependencies
  (`aiohttp`, `tomlkit`) are installed from PyPI within the version bounds of the generator's own header, not by hash:
  accepted risk for a generator whose output is reviewed in the pull request (the diff of `cargo-sources.json`
  shows every crate and hash).
- The tag value reaches shell steps only through an environment variable and is validated twice (workflow and script).
- A pull request from this workflow is reviewed like any other: read the diff and the permissions warning before merging.
