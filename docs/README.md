# Internal documentation

Index of the `docs/` folder: what each living document is for, where open work is tracked and where finished work went.

Status: living | Last reviewed: 2026-11-23 | Owner: @adhadse

## Contents

- [Where does a new document go](#where-does-a-new-document-go)
- [Remaining](#remaining)
- [Living](#living)
- [Finished, still pointed to](#finished-still-pointed-to)
- [Archive and outdated](#archive-and-outdated)

## Where does a new document go

Two kinds of documentation are kept apart (`AGENTS.md` §1):

| Kind | Where | Contents |
|---|---|---|
| Public wiki | `wiki/` (https://bookx.adhadse.com) | what users and plugin authors read: how to do it, what to expect, limits. Changes with every user-visible change; goes live with a release |
| Internal notes | `docs/` (this folder) | decisions, runbooks, research, caveats, performance numbers, progress. Never on the wiki |

Inside `docs/`:

1. **Open work** goes into [remaining.md](remaining.md), one row per item with status, priority, pointer and who can do it. Do not start a new notes file for a to-do list.
2. A **bug** goes into [bugs.md](bugs.md) with its root cause, fix and test; an **idea or request** into [enhancements.md](enhancements.md).
3. A **runbook or reference that people use now** is a file in `docs/` with a title, a one-line purpose and a `Status: living | Last reviewed: <date> | Owner: <who>` line (and a table of contents when longer than 150 lines).
4. Work that is **finished** (a plan carried out, a research, a wave or QA log, the notes of a feature that shipped) moves to [archive/](archive/README.md) with `git mv`, gets a first-line banner `> Archived <date>: <reason>. Living document: <link>` and a row in `archive/README.md`. Do not rewrite its text.
5. A document that is **no longer required** (superseded, abandoned, wrong now) moves to [outdated/](outdated/README.md) the same way.
6. Fix every pointer to a moved file. `build-aux/check-docs-links.py` checks the relative links and anchors of `docs/` and `README.md` and runs in `build-aux/release-check.sh`; `build-aux/check-docs-links.py --strict-paths` also checks the docs paths written in backticks.

## Remaining

| File | Purpose |
|---|---|
| [remaining.md](remaining.md) | the single list of open work by area, with status, priority (P1 beta.3, P2 before 1.0, P3 later), pointer and who can do it, plus the owner-only list |

## Living

### Process and release

| File | Purpose |
|---|---|
| [releasing.md](releasing.md) | the exact procedure for a release: version, tag, workflows in order, release assets, how to check them |
| [release-0.3.0.md](release-0.3.0.md) | the 0.3.0 checklist for the owner (pre-flight, tag, verify, notes, announce, after) |
| [release-plan.md](release-plan.md) | channel decision (GitHub Releases first), cadence, signals reviewers look for, exit criteria for Flathub, release log |
| [ci-gates.md](ci-gates.md) | the workflows, the gates a pull request must pass, caching, coverage |
| [security.md](security.md) | security model, vulnerability handling, per-change checklist and roadmap |
| [security-log.md](security-log.md) | monthly security record: advisories, dependency alerts, fuzz hours |
| [hardware-test-plan.md](hardware-test-plan.md) | what to run on real devices before a release |
| [translations-hosting.md](translations-hosting.md) | where translations are hosted and reviewed (decision record) |
| [flathub-submission.md](flathub-submission.md) | Flathub submission steps (deferred channel) |
| [flathub-automation.md](flathub-automation.md) | Flathub update automation after acceptance (deferred channel) |

### Bugs, requests, roadmap

| File | Purpose |
|---|---|
| [bugs.md](bugs.md) | found bugs with root cause, fix and guarding test |
| [enhancements.md](enhancements.md) | the owner's enhancement requests and what became of each |
| [bookx-feature-roadmap.md](bookx-feature-roadmap.md) | the feature roadmap and the owner's decisions (Part 2) |
| [roadmap-status.md](roadmap-status.md) | audit of the code against the roadmap rows (DONE, PARTIAL, MISSING) |
| [library-sync-design.md](library-sync-design.md) | what real multi-device library sync would take (design note, not built, owner decision open) |

### Platforms and packaging

| File | Purpose |
|---|---|
| [packaging-notes.md](packaging-notes.md) | Flatpak, deb/rpm, translations and diagnostics caveats |
| [macos-port.md](macos-port.md) | honest state of the Homebrew work |
| [windows-port.md](windows-port.md) | honest state of the winget and Scoop work |

### Wiki, tests and performance

| File | Purpose |
|---|---|
| [wiki-notes.md](wiki-notes.md) | how the public wiki is built and deployed, what the owner sets up by hand, weak pages |
| [wiki-theme.md](wiki-theme.md) | the wiki colour palettes |
| [test-notes.md](test-notes.md) | real-session tests, Broadway tests, harness findings |
| [test-books.md](test-books.md) | the public-domain books used by tests and hardware checks |
| [perf.md](perf.md) | performance budgets and how to run them |
| [performance.md](performance.md) | performance at scale: measurements, fixes, what is left |

### Per-area notes

| File | Purpose |
|---|---|
| [plugin-development.md](plugin-development.md) | writing plugins and patches (ABI, manifest, permissions) |
| [mcp-notes.md](mcp-notes.md) | the MCP server: decisions and caveats |
| [koreader-annotation-sync.md](koreader-annotation-sync.md) | KOReader highlight and note sync over Wi-Fi: protocol, design, status |
| [koreader-plugin-testing.md](koreader-plugin-testing.md) | installing and testing the Bookx Sync plugin for KOReader |
| [annotation-sync-notes.md](annotation-sync-notes.md) | annotation and progress portability: notes and caveats |
| [highlight-sync-rca.md](highlight-sync-rca.md) | root causes of the highlight colour and style bugs, the fix and the repair |
| [sync-ux-notes.md](sync-ux-notes.md) | why the Sync window and Preferences look the way they do |
| [device-ui-notes.md](device-ui-notes.md) | the Devices window: component map, rules, dev hooks |
| [backup-notes.md](backup-notes.md) | backup destinations and secret storage: decisions, caveats, untested |
| [networking-notes.md](networking-notes.md) | listeners and reverse proxy |
| [audiobook-notes.md](audiobook-notes.md) | audiobooks: design, tests, caveats |
| [comic-notes.md](comic-notes.md) | comic reader: design, caveats |
| [metadata-notes.md](metadata-notes.md) | metadata review, series, works, preflight |
| [a11y-notes.md](a11y-notes.md) | accessibility, command palette, Preferences search, RTL |
| [convert-parity.md](convert-parity.md) | converter parity with Calibre (a script rewrites parts of it) |
| [validator-notes.md](validator-notes.md) | how the EPUB validator is checked against epubveri |
| [editor-ui-design.md](editor-ui-design.md) | editor layout design (the mockups point to it) |
| [tts-notes.md](tts-notes.md) | read aloud: progress notes and numbers |

## Finished, still pointed to

These documents describe finished work. They stay in `docs/` only because `enhancements.md` or a source comment points to their current path; they move to `archive/` when those pointers can change (item DOC-3 in [remaining.md](remaining.md)).

| File | What it is |
|---|---|
| [remaining-work.md](remaining-work.md) | snapshot of the end of wave 11 (completion record in Part 1); superseded by remaining.md |
| [pain-points-calibre.md](pain-points-calibre.md) | research: Calibre pain points |
| [pain-points-ebook-managers.md](pain-points-ebook-managers.md) | research: pain points of ebook managers and readers |
| [tts-research.md](tts-research.md) | research: read-aloud models |
| [pdf-research.md](pdf-research.md) | research: PDF decision and status |
| [shortcuts-research.md](shortcuts-research.md) | research: shortcuts of other readers |
| [kobo-real-device-report.md](kobo-real-device-report.md) | report of the first real Kobo run |
| [editor-sourceview.md](editor-sourceview.md) | the editor on GtkSourceView (done) |

## Archive and outdated

- [archive/](archive/README.md): completed research, plans, wave and QA logs and design notes of features that shipped. History, not maintained.
- [outdated/](outdated/README.md): superseded, abandoned or no longer true. Do not follow.
