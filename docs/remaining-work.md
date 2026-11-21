# Remaining work: status at the end of wave 11 (2026-11-21)

Internal, not for the wiki. One document: **Part 1 = completed**, **Part 2 = not completed**. "Completed" lines were checked against
the tree (file, test, workflow or doc exists) and against the wave 11 agents' reports. Sources: `git log`, `docs/*notes*.md`, `src/`, `crates/`.
Wave 11 is **about to be committed** (HEAD is still `40b357a`; the tree is modified and untracked). Details per area: `docs/library-t3-notes.md`,
`serve-sources-notes.md`, `comic-notes.md`, `audiobook-notes.md`, `editor-*-notes.md`, `windows-ui-notes.md`, `ci-gates.md`, `test-notes.md`.

## Release 0.3.0 readiness (2026-11-22)

Runbook for the owner: `docs/release-0.3.0.md`. Nothing is committed, tagged or pushed yet.

| Item | Status |
|---|---|
| Version 0.3.0 (workspace, 18 crates, `Cargo.lock`), CHANGELOG `[0.3.0]`, metainfo release notes, README and install page | done |
| `build-aux/release-check.sh`, `cargo-sources.json` (ours = official generator, same 584 crates) | done (passes; `actionlint` not installed, one skip) |
| `cargo deny check` (advisories, bans, licenses, sources) | done, all ok (cargo-deny 0.21) |
| `cargo audit` | done: 0 vulnerabilities, 7 "unmaintained" notices, all already justified in `deny.toml` |
| NOTICE.md covers non-crate assets (5 action icons, app icon, mockups, legacy prototype) | done; owner confirms the origin of the icons and decides on `mockups/legacy` + `_legacy` (GNOME-derived art, see runbook §10) |
| Release build, CLI smoke test (init/add/list/check/convert/backup), headless GUI start and clean exit | done (release 2m13s, `bookx` 81 MiB, `bookx-cli` 39 MiB, unstripped) |
| `desktop-file-validate`, `appstreamcli validate --pedantic`, `msgfmt -c` (4 po files) | done (only the unavoidable `cid-contains-uppercase-letter`; po headers incomplete, Weblate fills) |
| Tarball, `.deb`, `.rpm` built locally and listed | done (48 MB / 30 MB / 36 MB; no man pages or shell completions exist) |
| Flatpak on GNOME 51 + rust-stable 26.08: build, `bookx-cli` inside the sandbox, `flatpak-builder-lint` manifest/appstream | done; repo lint shows only the screenshot-mirroring notice of local builds |
| GUI of the Flatpak started by hand once | needs owner |
| Public files: CONTRIBUTING, CODE_OF_CONDUCT, issue forms, PR template, CODEOWNERS; repo description and topics | done (description and topics applied with `gh`) |
| Secrets/personal-data sweep (tree and history) | done: no tokens or keys; `docs/kobo-real-device-report.md` redacted (owner's book list, hashes, user name); test fixtures hold only `/home/alice`-style paths |
| Workflows moved to the `gnome-51` image and `flatpak/flatpak-github-actions@v6.8` | done, **never run on GitHub** |
| `pin-screenshots.sh v0.3.0`, commit, tag, push, watch `release.yml` | needs owner |
| Verify artifacts (cosign, attestation), install tests on other distributions | needs owner |
| GitHub social preview image upload, announcement | needs owner |
| Flathub: account, AI disclosure, hand-written manifest and PR, domain token on adhadse.com | needs owner |
| Weblate project, branch protection with required `coverage` check, tag protection | needs owner |
| Full gates (`cargo test --workspace`, `coverage.sh`, `ui-tests.sh`, `session-tests.sh`, `i18n`) | not run by this pass (other agents are editing `src/`); owner runs them in runbook §1 |

## Top 10 next priorities

| # | What | Why | Who |
|---|---|---|---|
| 1 | **First sync of KOReader plugin 0.3.1 on the Kobo** and the 7-step live-update checklist (A.1) | the whole sync story is untested on hardware | owner |
| 2 | **Run the GitHub workflows once** (ci with coverage, release signing, wiki, i18n, package-*), then enable branch protection with the coverage check (command in `docs/ci-gates.md`) | nothing has ever run on GitHub; gates are paper until then | owner |
| 3 | **X18 performance**: windowed layout, first chapter off the main thread, 100k-book cold start, perf budgets | biggest open product risk for large books and libraries | agent |
| 4 | **Real ed25519 plugin signature verification** (needs a crate decision) + trust UI on top of the existing sheet | the signature format exists, verification does not (G10) | owner decides, agent |
| 5 | **Flathub submission** (hand-written manifest and PR by the owner because of the AI policy; GNOME 51; tagged release; domain verification) | main distribution channel | owner |
| 6 | **Test the new secrets paths by hand**: master-password dialogs, GNOME Keyring, Flatpak portal; rotate passwords from old plaintext configs | security fix is unit-tested but never tried by a human | owner |
| 7 | **Real-file tests for CBR (unrar/7z) and DJVU (ddjvu)**, RAR5/DjVu | both are subprocess paths never run on real archives | owner + agent |
| 8 | **R1 real table layout, positioned text, hanging punctuation, `word-break`, fixed-layout spreads (R46/R51)** | largest reader fidelity gap left | agent (L) |
| 9 | ~~Screens for features that only exist in CLI/engine~~ **built (wave 12)**: per-user accounts, Hardcover, S4 recipe editor + Basic-auth site accounts, lookup order / custom web sites / history. What is left (bookmark/other engine-only rows, the owner's real-service checks): `docs/screens-notes.md` | | agent |
| 10 | **Screenshot capture run and `.po` refresh**, then 0.3.x release (`release-check.sh`, pin screenshots), plus a session scenario for live updates and the new dialogs | wiki screenshots stale, msgids drifted, release blocked | agent + owner |

## How to read

- **Size**: S under a day, M 1-3 days, L about a week or more.
- **Who**: `agent` = doable in this repo without hardware or accounts; `owner` = needs devices, accounts, a decision or a GitHub/DNS setting; `both` = agent builds, owner verifies.
- **Verified** = exists in the repo. **Not verified on hardware** stays in Part 2 A.

## Counts (done vs open per area)

| Area | Done | Open | Notes |
|---|---|---|---|
| HIG backlog rows (213 unique ids, `human_interface_backlog.md`) | **213** | 0 | **SYN-S5** (retaken `preferences-sync-*` shots checked 2026-11-22) and **WIN-L16** (Preferences 820x600, session scenarios `qa_prefs_900_*` box (0, 0, 900, 568) inside a 900x600 window) closed. Mnemonics (5.6.1) decided as later (D11). Wave 11 closed WIN-L13, X6, X7, WIN-E6, XC-5, XC-7, LIB-L6 (and X9, 5.7.5 list) |
| Roadmap rows (`roadmap-status.md`, **fully re-audited 2026-11-22**) | **126 DONE** | 46 PARTIAL, **2 MISSING** (C7 PDF output, P21 remote reader control), 3 DEFERRED, 1 REPLACED, 0 ENGINE ONLY | 178 table rows; 24 PARTIAL or MISSING rows closed since the 2026-11-21 sample |
| Calibre editor rows | 255 DONE | 3 PARTIAL (CE-357, 401, 402), 0 MISSING, 7 NOT PLANNED | 265 rows, recounted by script 2026-11-22 (the two `MISSING` rows were decisions: now NOT PLANNED) |
| Pain points (calibre / general) | 13 / 19 DONE | 12 / 14 PARTIAL | recounted 2026-11-22 from the ranking tables (the 2026-11-20 lines were behind their own tables) |
| `docs/bugs.md` | 27 rows up to id 26 fixed, id 28 fixed | 2 open: **27** (no keyring: opening Preferences asks for a master password because the MCP page makes its token), **29** (cosmetic accessible name of *Show All*) | found by the new session scenarios; plus verification gaps (E) |
| Security roadmap | G1-G9, G12, G14, G15, G16, P1.1/P1.2 first cut, P1.3 shared limits, G5 SAFETY comments (9 `unsafe` uses, all commented, recounted 2026-11-22), plaintext-secrets fix, GitHub settings, signing workflow, plugin capability sheet + pin by hash (G10 part) | G10 (ed25519 verification), G11, G13 (SBOM, 18 of 44 workflow actions not pinned by SHA, first signed release), P1.1 runs, P2.4, P2.5, P3, P4 | F, `docs/security.md` §4.2 |
| Tests | **628** `scenario` blocks in 30 `build-aux/ui-tests*.sh` suites (was 568), 31 files in `build-aux/session/`, 13 fuzz targets, 13 `evil_*` test files; unit-test count not recounted (full `cargo test` list not run) | hardware, CI and gaps in G | |
| Coverage gate | logic tier 92.9 %, UI tier 80 % (Broadway), baseline `build-aux/coverage-baseline.json` | never run on GitHub; branch protection not applied | `docs/ci-gates.md` |
| Translations | de, es, fr, hi: all full (3735 of 3735 strings each on 2026-11-22, `build-aux/i18n-status.sh`), **all machine-translated, unreviewed** | native review | `po/STATUS.md` |

---

# PART 1: COMPLETED

## 1.1 Sync, KOReader plugin, live updates

| What | Evidence |
|---|---|
| KOReader plugin 0.3.0 then **0.3.1** (device name, auto sync, "Waiting for Wi-Fi", diagnostics with Lua traceback, unmatched list); **0.3.1 installed on the owner's Kobo, first sync not confirmed** | `koreader-plugin/bookx.koplugin/bookxsync.lua`, `koreader-plugin/tests/run.lua`, `wiki/docs/koreader-sync.md`, `build-aux/package-plugin.sh`, `plugin-tests.sh` |
| `/bookx` endpoint, server and login remembered, Host/Origin guard; **KOSync key Argon2id-hashed server side** | `crates/bookx-serve/tests/server_integration.rs`, `build-aux/check-sync-endpoint.sh` |
| Sync activity UX | `src/ui/sync_activity.rs`, `docs/sync-ux-notes.md`, `ui-tests-sync.sh` |
| **Live reader updates on sync**: event bus, reader / sidebar / library refresh, "device is ahead" banner, edit-conflict banner | `docs/koreader-annotation-sync.md` (Live updates), `docs/sync-ux-notes.md`, `ui-tests-koannot.sh`; real-session scenarios `live_sync_highlight`, `live_sync_ahead_banner`, `sync_activity_window` (`scenarios_wave11.py`, passed 2026-11-22) |
| Real-data accuracy report and runbook (Kobo, 406 highlights) | `docs/kobo-koreader-sync-runbook.md`, `kobo-real-device-report.md`, `koreader-annotation-sync.md` |

## 1.2 Highlights, bookmarks, notes

| What | Evidence |
|---|---|
| Highlight colour/style model with sync and one-time **repair** | `crates/bookx-serve/tests/highlight_looks.rs`, `look_repair.rs`, `docs/highlight-sync-rca.md`, `ui-tests-hlsync.sh` |
| Bookmarks import (KOReader / Kobo), Notes filter, jump-to-highlight | `src/ui/notes.rs`, `ui-tests-koannot.sh`, `ui-tests-anchor.sh`, `docs/annotation-sync-notes.md` |
| Joplin export, copy with citation (Ctrl+Shift+C) | `ui-tests-reader-ux.sh`, `docs/ux-gaps-notes.md` |

## 1.3 Reader typography and behaviour (wave 11, T1)

| What | Evidence |
|---|---|
| Widows/orphans, small caps, text weight, contrast, word spacing, kerning | `crates/bookx-text`, `ui-tests-reader-ux.sh` |
| Book CSS fonts and sizes with per-book switches | `bookx-text` CSS subset, reader settings |
| Page-turn animation, auto-turn timer, evening schedule | reader settings, `ui-tests-reader-ux.sh` |
| Skim preview, TOC fold, highlight handles, link hints, page-turner keys, auto-archive, whole-word / regex in-book search | `ui-tests-reader-ux.sh`, `docs/ux-gaps-notes.md` |

## 1.4 Library and store (wave 11, T3)

| What | Evidence |
|---|---|
| **Custom columns** (store schema, `#label` search, Calibre mapping, CLI) | `crates/bookx-store`, `docs/library-t3-notes.md` |
| Search language: parentheses, AND/NOT, regex, dates | `crates/bookx-core/src/query*`, unit tests |
| Add by ISBN, empty record, multi-book archives; filename guess; search & replace across fields | `docs/library-t3-notes.md`, `ui-tests-lib8.sh` |
| Send shelf, tag tree, related books, author/series notes, marked books, folder watcher, FTS status/rebuild, jobs for device sends and web adds, EPUB catalogue, cover from PDF, trim 2:3 | same notes, `ui-tests-lib8.sh`, `ui-tests-works.sh` |

## 1.5 Formats, comics, audiobooks (wave 11, T4/T5)

| What | Evidence |
|---|---|
| Comics: next/previous volume, series from folders, per-library direction, ignore ComicInfo | `crates/bookx-comic`, `docs/comic-notes.md`, `ui-tests-comics.sh` |
| Audiobook ↔ e-book position (local, manual); pairing performance fix | `crates/bookx-audio`, `docs/audiobook-notes.md`, `ui-tests-audiobook.sh` |
| Conversion queue and options dialog | `docs/library-t3-notes.md` |
| **CBR via `unrar`/`7z`** and **DJVU via `ddjvu`** subprocesses (**untested with real archives**) | `crates/bookx-comic`, `bookx-formats` |

## 1.6 Serve, devices, sources (wave 11, T6/T8)

| What | Evidence |
|---|---|
| Trusted proxy, allowed hosts, sub-path settings; QR code (`qr.rs`) | `crates/bookx-serve/src/qr.rs`, `hostguard`, `docs/networking-notes.md` |
| Wallabag import (`wallabag.rs`), per-feed full/summary, source priority UI, name-order toggle, Wikipedia as a book | `crates/bookx-sources`, `docs/serve-sources-notes.md` |
| PWA web client, per-user accounts (**CLI only, no screen**), Check Before Send, Hardcover client (**no screen**) | `crates/bookx-serve/src/{pwa,accounts,hardcover}.rs` |

## 1.7 Editor, app, plugins (wave 11, T7/T9/T10)

| What | Evidence |
|---|---|
| Preview colours, preview click selects code | `docs/editor-preview-notes.md`, `ui-tests-editor-preview.sh` |
| Plugin trust sheet, signature **format** (verification not possible: no ed25519 crate), run plugin over library from the CLI | `crates/bookx-plugin`, `docs/plugin-development.md` |
| CLI: catalog, polish, fonts, saved-search, fetch-news | `crates/bookx-cli` |
| Settings import/export, safe mode, debug log, `--doc` and Open With forwarding | `src/{settings_io,safe_mode,debug_log}.rs` |
| MCP resource subscriptions | `crates/bookx-mcp`, `docs/mcp-notes.md` |

## 1.8 Backups (D9) and TTS extras

| What | Evidence |
|---|---|
| Incremental content-addressed repository (book files, covers, catalogue, notes journal), encryption, verify, retention, restore UI | `crates/bookx-backup/src/repo/`, `src/ui/backup.rs`, `docs/backup-notes.md`, `wiki/docs/backup-and-restore.md` |
| Wave 11: repository lock and take over, **collections in the journal**, chunked WebDAV upload, change backup passphrase (GUI only) | `docs/backup-notes.md`, `ui-tests-backup9.sh` |
| TTS speed above 2x | `crates/bookx-tts`, `docs/tts-notes.md` |

## 1.9 Devices window and GNOME HIG waves 6-11 (rows addressed: 213 of 213)

| Area | Rows | Addressed | Main results |
|---|---|---|---|
| Library and navigation (LIB) | 58 | 58 | Library menu, Trash with Undo, Health sidebar, Online Catalogues destination (D12), **LIB-L6 Needs Attention selection** |
| Reader (RDR) | 26 | 26 | one main menu, jump buttons, header caps |
| Editor (EDT) | 31 | 31 | Undo toasts, checkpoints, one Preferences, Problems panel |
| Preferences and system (PRF) | 27 | 27 | twelve pages, MCP page, privacy |
| Sync (SYN) | 13 | 13 | four-step checklist; SYN-S5 closed 2026-11-22 |
| Windows from hamburgers (WIN) | 42 | 42 | **WIN-L13 rows + one window, WIN-E3/E6** closed in wave 11; WIN-L16 closed 2026-11-22 |
| Cross-cutting (X, XC) | 16 | 16 | **X6, X7, X9, XC-5, XC-7** (scale 1.5x scenarios `win11_scale_*`) closed in wave 11 |

Devices window redesign and compact grid: `src/ui/devices.rs`, `devices_list.rs`, `docs/device-ui-notes.md`, `ui-tests-win8.sh`. The HIG 5.7.5 verify list was run.

## 1.10 Secrets (wave 11)

| What | Evidence |
|---|---|
| **No secret in a plain file**: encrypted vault with master password, keyring, session-only mode, scrub migration of old configs; MCP token not on disk | `crates/bookx-secrets`, `src/{config,secrets}.rs`, `ui-tests-secrets.sh` |

## 1.11 Security

| What | Evidence |
|---|---|
| G6 `safe_external_url`, G7 no `sh -c`, G8 SSRF guard, G9 `hostguard`, G14, G15 | `docs/security.md`, `crates/bookx-serve/tests/hardening.rs` |
| Wave 11: **G5** SAFETY comments, **P1.3** shared `limits` module, new evil tests, Calibre import path traversal fix | `docs/security-log.md`, `crates/*/src/limits.rs`, `crates/*/tests/evil_*.rs` |
| 13 fuzz targets, replayed on seeds | `fuzz/fuzz_targets/` |
| GitHub settings; cosign keyless signing and provenance in `release.yml` (never ran) | `docs/security.md` §11, §13, `.github/dependabot.yml` |

## 1.12 Tests, coverage, CI

| What | Evidence |
|---|---|
| Vacuous-check lint, flake fixes, dialog and i18n lints | `build-aux/lint-checks.sh`, `src/ui/i18n_lint.rs`, `docs/qa-wave5.md` |
| **Coverage gate**: cargo-llvm-cov + nextest + diff-cover, logic 92.9 %, UI 80 % via Broadway, baseline file, CI job and nightly UI coverage | `build-aux/{coverage,coverage-ui}.sh`, `coverage-gate.py`, `coverage-baseline.json`, `docs/ci-gates.md` |

## 1.13 Translations

| What | Evidence |
|---|---|
| de, es, **fr, hi complete** (machine-translated, unreviewed), test-enforced | `po/*.po`, `po/STATUS.md`, `build-aux/i18n-status.sh`, `.github/workflows/i18n.yml` |
| Weblate chosen, no translator credits for AI text | `docs/translations-hosting.md`, `wiki/docs/translating.md` |

## 1.14 Distribution scaffolding (written, **never built or run** on the target platform)

| What | Evidence |
|---|---|
| Flathub draft, `flathub.json`, `prepare.sh`, bundle workflow | `packaging/flathub/`, `docs/flathub-submission.md`, `.github/workflows/package-flatpak.yml` |
| Homebrew, winget, Scoop, MSYS2 scaffolds and bundle scripts | `packaging/{homebrew,winget,scoop,msys2}/`, `build-aux/{macos,windows}-bundle.sh`, `docs/macos-port.md`, `windows-port.md` |

## 1.15 README, wiki, docs

| What | Evidence |
|---|---|
| README rewritten (generated light/dark screenshots, checked by script) | `README.md`, `build-aux/check-readme-images.py` |
| Wiki "night paper" dark theme with footer link; Q&A rewrite | `docs/wiki-theme.md`, `build-aux/check-wiki-*.py`, `wiki/docs/troubleshooting.md` |
| CHANGELOG `[Unreleased]`, accessibility page, internal notes per area (`docs/*-notes.md`) | `CHANGELOG.md` |

---

# PART 2: NOT COMPLETED

## A. Needs the owner's hardware or accounts

| Id | What | Size | Who |
|---|---|---|---|
| A.1 | **Plugin 0.3.1 first sync** and the **7-step live-update checklist** (below) | M | owner |
| A.2 | Kobo native highlight colour numbers (`Color` 0..3 yellow/pink/blue/green from community docs); KOReader >= 2024 reading Bookx's notes file | S | owner |
| A.3 | TTS and audiobooks on a real sound card (pause/drain, prefetch, voice download, MPRIS, `pacat`/`pw-cat` latency, 3x, part-boundary clicks); aarch64 never run | M | owner |
| A.4 | WebDAV / Nextcloud / NAS / Google Drive backups (chunked upload too); **GNOME Keyring and Flatpak secrets portal**; **try the master-password dialogs by hand**; **rotate passwords that were in old plaintext config copies** | M | owner |
| A.5 | Flatpak sandbox: `/run/media`, MTP via GVfs, voices, portal, network places, `bookx://`, Open With | M | owner |
| A.6 | **Flathub submission**: manifest and PR hand-written by the owner (AI-policy blocker), GNOME 51 runtime, tagged release, domain verification, machine-translated translations policy | M | owner |
| A.7 | Apple Developer ID / notarization, Microsoft signing, a Mac and a Windows machine for the package scaffolds | L | owner |
| A.8 | Kindle, PocketBook, Boox, other Kobo models, touch and e-ink per `hardware-test-plan.md` | M | owner |
| A.9 | **MCP with a real assistant**; Anki/Readwise/OPDS/Hardcover against real services; **real RAR5 and DjVu files** for the new CBR/DJVU paths | S | owner |
| A.10 | Weblate project creation; run the GitHub workflows once (ci incl. coverage, release signing, wiki, i18n, package-*); **enable branch protection with the coverage check** (command in `docs/ci-gates.md`) | S | owner |

**Checklist A.1 (plugin 0.3.1 and live updates)**

1. Bookx: `./run.sh`; Preferences → Sync: OPDS server on, password, *KOReader sync*, *Sync highlights and notes*; note `http://<LAN IP>:8080/bookx` (firewall allows 1025-65535, no AP isolation). KOReader *Progress sync* pointed at `/bookx` for step 6.
2. Copy `koreader-plugin/bookx.koplugin` (0.3.1, `build-aux/package-plugin.sh`) to `.adds/koreader/plugins/`; restart KOReader fully (already installed on the owner's Kobo).
3. Tools → More tools → **Bookx Sync** → About / Diagnostics shows 0.3.1.
4. Enter server, user, password, device name; **Save**; **Sync now**: "Sent N, removed 0, received M". On a panic copy the traceback (bug 17).
5. Devices window shows the Kobo with versions; Notes list highlights once each, source "KOReader (device name)".
6. Live updates with the reader open (plugin sends about 30 s after creating a highlight): (a) the highlight appears in Bookx about 1 s after the push; (b) colour change; (c) delete; (d) bookmark; (e) **"device is ahead" banner** when the Kobo is further; (f) **edit-conflict banner**; (g) Open Book toast.
7. Wi-Fi off, highlight, Wi-Fi on: *Waiting for Wi-Fi*, then sync.

## B. Owner decisions

| Id | Question | Recommended | Who |
|---|---|---|---|
| B1 | Pages source = Actions and DNS for `bookx.adhadse.com` (domain and `CNAME` already fine; confirm the Pages setting) | confirm | owner |
| B3 | Per-user accounts screen and permissions scope: **screen built** (`ui/accounts.rs`); scope is the tags of a book; collections other than tags (shelves) are not offered because the server only knows tags | owner decides on shelves | owner |
| B4 | Multi-device library sync (`library-sync-design.md`; backup is one-way) | decide | owner |
| B5 | Calibre wireless stays deferred (D10); confirm drop | confirm | owner |
| B7 | AppImage yes/no, aarch64 yes/no | decide | owner |
| B8 | Who reviews es/de/fr/hi natively | decide | owner |
| B9 | ed25519 crate for plugin signature verification (none vendored today) | allow one audited crate | owner |

B2 (custom columns) and B6 (passphrase change in place) are **resolved**: custom columns built, passphrase change built (GUI; CLI missing, see D).

**Decided (do not ask again)**

| Date | Decision |
|---|---|
| 2026-11-21 | OPDS server is the shared server, *Online Catalogues* the client list (D10); mnemonics later (D11); catalogues a sidebar destination (D12); WIN-R9 closed (D13); no label ends with an ellipsis (D14); wave-9 defaults confirmed (D15); wave 11 "decided by default" rows (Duplicates radios, Vocabulary short dialogs, New Book title dialog) |
| 2026-11-21 | Translations: Weblate on hosted.weblate.org, no translator credits for AI text |
| 2026-11-21 | Release signing: cosign keyless + provenance; GitHub private reporting, advisory owner @adhadse |
| 2026-11 | Flathub is the first-class channel; AppImage not provided; wireless device = KOReader only (D10) |

## C. Remaining HIG rows

| Row | What | Size | Who |
|---|---|---|---|
| ~~WIN-L16~~ | closed 2026-11-22 (verified: dialog 820x600, session scenarios `qa_prefs_900_*`) | - | done |
| ~~SYN-S5~~ | closed 2026-11-22 (`preferences-sync-*.png` retaken, viewed) | - | done |
| 5.6.1 | Access keys (`_` mnemonics), decided **later** (D11) | M | agent, later |

## D. Features not built (re-checked 2026-11-21)

| Track | Item | What is missing | Size | Who |
|---|---|---|---|---|
| X18 perf | windowed layout, first chapter off the main thread, 100k cold start, perf budgets (scroll fps, memory) | **built 2026-11-22** (sliced + windowed layout, Select All / selection at 100k, parallel catalogue load, budgets; numbers in `docs/performance.md`). Left: scroll fps in a real session at 100k, PDF open in the app, lazy `BookItem`s, memory (600 MB at 100k) | S | agent |
| X5 | reader at phone size (360 px) | not done | M | agent |
| T1 text | R1 real table layout, positioned text, hanging punctuation, `word-break`, emphasis dots, vertical tables; R46/R51 fixed-layout spreads; R18 SMIL; R31 editable highlight text | | L | agent |
| T2 lookup | R38, R40, R41: dictionary priority, custom web lookup, history **UI built** (`ui/lookup_settings.rs`); stats calendar export; PDF book map | | S-M | agent |
| T3 library | L17 bookshelf / cover-flow view; interactive cover crop; catalogue sections; column colouring rules; tag drag-drop; notes on tags/publishers editor; saved searches with OR + parentheses; RAR in the multi-book archive picker; MCP custom columns | | S-M each | agent |
| T4 formats | OCR hook, PDF output, LRF, AZW4, KF8 flows, DOCX TOC fields; Opus playback (no decoder dependency) | | L | agent |
| T6/T8 | S4 recipe editor UI + news accounts (**built**, HTTP Basic only), Hardcover screen (**built**, not tried against the real service), per-user accounts screen (**built**); direct MTP (gvfs only); Kindle native reading state; FTP/SSH | | S-L | agent |
| T7 editor | per-family preview fonts, "lines above sync point", vertical-text markers in preview, editor plugin panels | | S-M | agent |
| T10 plugins | real ed25519 verification (B9); OCR hook | | M | agent |
| enh 4 | device sync plan unification (`devices::full_sync_plan` with `device_sync`), device writes | | L | agent |
| enh 7 | CLI commands for backup passphrase change and take over; per-byte progress; GTK add/edit scenario | | S-M | agent |
| enh 9 TTS | MP3/OGG export (no pure-Rust encoder), Kokoro Japanese/Chinese, Orpheus tier, `cpal` sink (optional feature skipped) | | M | agent |
| i18n | live re-translation, RTL, native review (B8) | | M | agent + owner |
| later | mnemonics (D11) | | M | agent |
| NOT PLANNED | CE-016/038/228/229/261/281/404 | by decision | - | none |

## E. Known bugs and verification gaps (all rows of `docs/bugs.md` up to id 26 and id 28 are `fixed`; 27 and 29 are open)

| Id | What | Size | Who |
|---|---|---|---|
| bug 27 / 29 | open (found 2026-11-22 by the session scenarios): the master-password prompt at the first opening of Preferences without a keyring (make the MCP token lazy), the accessible name of *Show All* | S | agent |
| bug 15/17 | Plugin never confirmed inside KOReader on the owner's Kobo (A.1) | M | owner |
| bug 12 / 22 | Kobo colour numbers and Bookx-written KOReader notes layout unverified | S | owner |
| bug 12 (race) | Popup race fix relies on a 90 ms delay, checked on one machine | S | agent |
| ~~bug 16~~ | First 1500-paragraph chapter layout (0.6-0.9 s) on the main thread: **fixed** (8 ms slices, 540 ms stall before, at most 54 ms after) | - | done |
| qa-wave5 | `Gtk-WARNING` in `library_hero_*` resize; `calibre_export` fails while real Calibre runs | S | agent |
| grid_reflow_1400x900 | At 1024 px the details panel does not narrow the grid; assertion relaxed | S | agent |
| audiobook-notes | Opus not playable; up to 50 ms silence from AAC priming; first scan of a huge folder on the UI thread | M | agent |

## F. Security roadmap (after the wave)

| Id | Item | Size | Who |
|---|---|---|---|
| P1.1 | Nightly `cargo fuzz` job and real fuzz hours (13 targets replayed on seeds only); targets for audiobook tags, StarDict, Calibre `metadata.db` | M | agent + owner |
| G10 | Plugin signature **verification** (format and trust sheet done), pin by hash, "unverified plugin" banner | M | agent (B9) |
| G11 | Sandbox for non-Flatpak installs (bwrap/Landlock) | L | agent |
| G13 | First signed release (workflow never ran), SBOM, pin actions by SHA, reproducible builds | M | owner + agent |
| P2.4 / P2.5 | Backup crypto second review; `cargo deny` re-run (not installed), `cargo vet`, "unmaintained" notices | S-M | owner / agent |
| P3 / P4 | Threat-model review cadence, incident runbook; auth by default for servers, MCP write audit view, allowed-hosts field in Preferences (settings exist, check UI) | L | owner / agent |

## G. Test and CI gaps

| Item | Size | Who |
|---|---|---|
| `ci.yml` (incl. coverage), `release.yml`, `wiki.yml`, package workflows never ran on GitHub; `actionlint` skipped; branch protection not applied | S | owner |
| `session-tests.sh` never in CI; ~~no session scenario for live updates and the new dialogs~~ **added and passed 2026-11-22** (`scenarios_wave11.py`: live updates, banner, activity window, master-password dialogs, Custom Columns, Settings Backup preview, plugin trust sheet, safe mode; the file choosers of Export/Import Settings are the portal's and are not driven, the hooks open the real preview); `tts_player_card_real` and 3+ concurrent sessions flaky (inotify) | M | owner + agent |
| Recheck: `koreader_annotations.rs::the_real_plugin_registers_and_syncs_against_a_real_server` (14 vs 15 highlights) | S | agent |
| No scenario: touch/long-press, `show_action` toast, `book_formats::merge` undo, metadata provenance UI | M | agent |
| Real mouse-drag scenarios call handlers directly; network fixtures (`NET=0` only in CI); perf budgets for scroll fps, 100k cold start | M | agent |
| `.po` refresh for new msgids from wave 11 before the next release (`build-aux/i18n.sh`) | S | agent |

## H. Release, packaging, docs, wiki gaps

| Item | Size | Who |
|---|---|---|
| Release 0.3.x: tag, `release-check.sh`, `pin-screenshots.sh`; `.deb`/`.rpm` never built | M | owner + agent |
| Metainfo OARS empty, no donation URL, translated metainfo not refreshed | S | agent/owner |
| macOS/Windows packages never compiled or run: workspace never built for Windows (`std::os::unix` in `bookx-backup`, `bookx-tts`), no `cpal` sink | L | owner + agent |
| Stale wiki screenshots: `library-main-menu-*`, `preferences-sync-*`, `library-device-*`, statistics/saved-searches/trackers/audiobooks/citations/catalogues/health/vocabulary, `reader-menu-*`, `book-map-*`, `reader-goto-*`, `editor-book-menu-*`, `editor-checkpoints-*`, `editor-preferences-*`; none yet for Sync window rows, Import/Export Calibre, About, reader Properties, new wave 11 dialogs | M | agent (capture run) |
| Mockups not updated for new windows; `wiki/docs/editing.md` duplicate sections; run `build-aux/wiki.sh` for the wave 11 wiki pages; wiki "Hardware tested so far" stays until A is done | M | agent |

## I. Stale docs to clean

| File | Stale statement | Fix |
|---|---|---|
| `human_interface_backlog.md` | ~~WIN-L16 `open`; SYN-S5 `open (lead)`~~ closed 2026-11-22; sub-table duplicates remain | dedupe if wanted |
| `roadmap-status.md` | ~~last full audit 2026-11-13~~ re-audited 2026-11-22, counts in section 'Counts' | done |
| `security.md` §4.1/§4.2 | ~~G1-G3, G12 listed as gaps; "20 unsafe uses"~~ updated and recounted 2026-11-22 (9 `unsafe`, all with SAFETY comments; G10 partly done, G11 and G13 open) | done |
| `bookx-feature-roadmap.md` Part 4 | "Not done" list superseded | pointer to this file |
| `calibre-wireless-device-plan.md` | status "not implemented" | note "deferred (D10)" |
| `pain-points-*.md` | ~~counts of 2026-11-20~~ refreshed 2026-11-22 | done |
| `_developer/docs.md` | check dev hooks added in wave 11 (highlight repair, `app:text-scale`, `sync:prefs`, `dialog:state count=`) are listed | check |
