# Calibre pain points, and what Bookx does about them

Research for `docs/enhancements.md` item 1. Date: 2026-11-12. Research only: no code was changed.

How this was made: web searches and page reads across the Calibre bug tracker (Launchpad), MobileRead, r/calibre, r/selfhosted, r/kindle, Hacker News, LWN, blog posts, and the issue trackers of Calibre-Web, Calibre-Web-Automated, DeDRM and Flathub. Then the Bookx code and `docs/bookx-feature-roadmap.md` were checked to see what already exists.

Constraints respected (roadmap Part 2): D1 everything in, D2 SQLite via `rusqlite` is fine, D3 PDF later, D4 only formats Calibre supports, D5 plugins after Wave 2, D7 CLI and server are first class, D9 no exclusions (LIT excluded). Rust only, GTK4/libadwaita, tests with every change (`AGENTS.md`).

## How to read this

- **Frequency** is how often the complaint shows up across the sources I found. High means many independent threads over several years. Medium means a handful of threads. Low means one or two sources.
- **Status** (DONE / PARTIAL / NOT YET) is checked against the code and the roadmap progress log (Part 4), not against intentions.
- Effort: S, M, L. Priority: P1 (do next), P2, P3.
- Sources are summarised in my own words. Counts and claims I could not confirm are listed at the end.

## Ranking at a glance

| Rank | ID | Pain point | Frequency | Bookx status | Priority |
|---|---|---|---|---|---|
| 1 | PP-C-1 | Dated, cluttered, hard to discover UI | high | PARTIAL | P1 |
| 2 | PP-C-2 | `metadata.db` fragility and corruption | high | PARTIAL | P1 |
| 3 | PP-C-3 | Network drives, NAS and cloud-synced folders break the library | high | PARTIAL | P1 |
| 4 | PP-C-4 | Metadata download is unreliable and often wrong | high | PARTIAL | P1 |
| 5 | PP-C-5 | Conversion quality and failures | high | PARTIAL | P1 |
| 6 | PP-C-6 | Kindle: format churn, KFX and DRM | high | PARTIAL | P2 |
| 7 | PP-C-7 | Forced `Author/Title` folder layout, files move on edit | high | NOT YET | P1 |
| 8 | PP-C-8 | Governance, bus factor, AI features, telemetry | high (2025-26) | PARTIAL | P2 |
| 9 | PP-C-9 | Plugin breakage on Python/Qt/Calibre upgrades | high | PARTIAL | P2 |
| 10 | PP-C-10 | Not made for several users or processes | high | PARTIAL | P2 |
| 11 | PP-C-11 | Startup and large-library speed | medium | PARTIAL | P1 |
| 12 | PP-C-12 | Kobo: KEPUB, plugins, device sync | medium | PARTIAL | P2 |
| 13 | PP-C-13 | Qt look, Wayland, HiDPI, dark mode | medium | DONE | P3 |
| 14 | PP-C-14 | Content server is basic and open by default | medium | PARTIAL | P2 |
| 15 | PP-C-15 | Duplicate handling | medium | DONE | P3 |
| 16 | PP-C-16 | No automatic backups, restore takes days on big libraries | medium | PARTIAL | P1 |
| 17 | PP-C-17 | No official mobile reader | medium | PARTIAL | P2 |
| 18 | PP-C-18 | Database as a black box, lock-in, hard to migrate | medium | PARTIAL | P1 |
| 19 | PP-C-19 | Installation and update friction | medium | PARTIAL | P2 |
| 20 | PP-C-20 | Weak built-in reader | medium | PARTIAL | P3 |
| 21 | PP-C-21 | Reading progress does not follow you | medium | DONE | P3 |
| 22 | PP-C-22 | Security record and phone-home | low-medium | PARTIAL | P2 |
| 23 | PP-C-23 | Accessibility | low (thin evidence) | PARTIAL | P2 |
| 24 | PP-C-24 | Risky merge and delete, no library undo | low | NOT YET | P2 |
| 25 | PP-C-25 | News recipes: clunky and Python-bound | low | PARTIAL | P3 |

Counts: DONE 3, PARTIAL 19, NOT YET 3 (25 pain points).

---

## PP-C-1 Dated, cluttered, hard to discover UI

- **Description.** The most repeated complaint. The window is dense, the toolbar is icons plus text, many actions hide in nested menus, tooltips act as documentation, and the look does not follow the desktop. People who use it a few times a year say they have to search the web each time. Defenders say it is functional and consistent, and the debate itself is a sign of how loud the topic is.
- **Evidence.**
  - Hacker News "Calibre 8.0" thread, long argument about icons, nested menus and merge-records menus: https://news.ycombinator.com/item?id=43434421
  - LWN review: the interface matches no platform convention, the default layout is "busy", the toolbar has no quit action, the viewer shows no controls until you right-click: https://lwn.net/Articles/1015226/
  - r/selfhosted: a newcomer finds desktop Calibre "extremely confusing": https://www.reddit.com/r/selfhosted/comments/1pvmq5r/is_desktop_calibre_still_the_king_of_ebook/
  - XDA on moving to Booklore because Calibre feels dated and bloated: https://www.xda-developers.com/this-self-hosted-app-fixed-my-messy-ebook-collection-overnight/
- **How often.** High.
- **Bookx today: PARTIAL.** The library is a clean libadwaita grid with a Continue Reading card, cover progress and one search bar (roadmap Part 4, "Earlier work"). Update: the command palette (Ctrl+K / Ctrl+Shift+P, `src/palette.rs`) and a searchable, paged Preferences (the dialog's own search plus the palette's index of every row) are done (`docs/a11y-notes.md`). Remaining gaps: the editor window has no palette yet, and the built-in Preferences search does not match section titles. Design mockups are out of date (roadmap line 528).
- **Proposal.** Keep the three-way split Calibre mixes together: browse, edit, convert. (1) Ship a command palette (Ctrl+K) that lists every action, including the ones hidden in context menus (S/M). (2) Make Preferences one searchable page (X11) (M). (3) Give every destructive or bulk action a plain-language dialog with a preview instead of a submenu (see PP-C-24). (4) Put descriptions in the UI, not in tooltips. (5) Redraw the mockups. Effort M, priority P1.

## PP-C-2 `metadata.db` fragility and corruption

- **Description.** The catalogue is one SQLite file that Calibre writes while you work. Power loss, antivirus, sync tools and crashes corrupt it. Users report that restoring from the per-book OPF files is very slow on large libraries. Calibre-Web writing into the same file made it worse for many.
- **Evidence.**
  - MobileRead: a 60,000-book library whose database got corrupted while adding or editing, with a restore that ran for days: https://www.mobileread.com/forums/showthread.php?t=347713
  - r/calibre: the risk of corruption when Calibre-Web shares the library on a NAS: https://www.reddit.com/r/Calibre/comments/1cky1qf/question_about_calibreweb_on_a_nas_and_the_risks/
  - Calibre-Web issue: library corrupted after managing it with Calibre-Web: https://github.com/janeczku/calibre-web/issues/337
  - Hacker News: network disconnects will most likely corrupt the library: https://news.ycombinator.com/item?id=21156797
- **How often.** High.
- **Bookx today: PARTIAL.** The catalogue is SQLite, committed in one transaction, with a per-book hash diff, so a crash cannot leave half a save (`bookx-store`, roadmap "Decision 2026-11-12"). `SqliteStore::backup_to`, `restore_from`, `check_integrity`, `check_file` exist (`crates/bookx-store/src/store.rs` lines 804-872) with tests, but a search of `src/` and `crates/bookx-cli/src` finds no caller. The roadmap says "backup + integrity check, no scheduler" (Part 4, bookx-store row). Bookx does not write a per-book OPF sidecar, so if the database file is lost the only copy of the metadata is the backup.
- **Proposal.** (1) Wire `check_integrity` at startup and show a recoverable error dialog ("restore the last snapshot"), never a crash (S). (2) Add `bookx-cli library check|backup|restore` and a Library menu entry (S). (3) Keep a rolling set of snapshots (see PP-C-16). (4) Optionally write a small per-book sidecar (`metadata.json`/OPF) so the catalogue can be rebuilt from the folder in seconds, with the rebuild running on all cores and showing progress (M). Tests: kill-during-write fuzz of the commit, rebuild-from-sidecar round trip, perf budget for rebuild of 10,000 books. Effort M, priority P1.

## PP-C-3 Network drives, NAS and cloud-synced folders

- **Description.** Calibre's own FAQ tells people not to keep the library on a network drive, yet that is where many keep it (NAS, Synology, SMB, Resilio, Dropbox-like sync). Corruption and "database locked"-style trouble follow. Docker and CasaOS users hit permission and path problems too.
- **Evidence.**
  - Calibre FAQ entry on networked drives (referenced by the MobileRead thread above): https://manual.calibre-ebook.com/faq.html
  - "Calibre, Calibre-Web with a network share": https://aroundmyroom.com/2018/07/14/calibre-calibre-web-with-a-network-share/
  - It's FOSS community, library inside a synced folder: https://itsfoss.community/t/calibre-pain-plus-more-chars-to-make-it-fifteen/9080
  - Cloudron forum, library on a CIFS mount: https://forum.cloudron.io/topic/10010/calibre-not-able-to-write-on-mounted-cifs-volume
- **How often.** High.
- **Bookx today: PARTIAL.** SQLite has the same weakness on network file systems. What Bookx has is the proper alternative: a headless server (`bookx-cli serve`, OPDS, web page, KOSync, `CatalogWatcher` following the database via `PRAGMA data_version`, roadmap Part 4) so the library can live on one machine. It does not detect a network or synced folder, and does not tell the user.
- **Proposal.** (1) At library open, detect network and FUSE file systems (statfs type) and sync-client markers (`.dropbox`, `.stfolder`, `.sync`) and show a one-time libadwaita banner explaining the risk and pointing to Share Library / `serve` (S). (2) Take an advisory lock so two Bookx processes cannot write the same library without noticing (S). (3) Add a "read-only library" open mode for a second reader of the same folder (M). (4) Docs page "Running Bookx on a NAS". Effort S-M, priority P1.

## PP-C-4 Metadata download is unreliable and often wrong

- **Description.** Bulk downloads return wrong books, sources break when a site changes or blocks repeated requests, and the Amazon source regularly fails. The usual workaround is to type the ISBN by hand. Quality of tags and genres is also criticised.
- **Update 2026-11-13 (metadata track):** bulk *Fetch Metadata…* with a review table (now with the source of each field), per-source health with backoff that survives restarts, an on-disk response cache with short TTLs, fetch at import (opt-in, empty fields only), provenance and undo (covers included) are built (see `docs/pain-points-ebook-managers.md` PP-G-1). Still open: live-network tests.
- **Evidence.**
  - r/calibre "Metadata download suddenly sucks": https://www.reddit.com/r/Calibre/comments/1te1mo7/metadata_download_suddenly_sucks/
  - MobileRead "Calibre keeps putting wrong information on books", bulk download described as iffy and getting worse: https://www.mobileread.com/forums/showthread.php?t=355640
  - MobileRead, Amazon and Fantastic Fiction sources not downloading: https://www.mobileread.com/forums/showthread.php?t=371313
  - Launchpad bug on an error in the Amazon source: https://bugs.launchpad.net/bugs/2091542
- **How often.** High.
- **Bookx today: PARTIAL.** The edit dialog fetches from Open Library, Google Books and Wikidata, merges with confidence and offers fill-empty or replace, plus cover (`src/metadata_fetch.rs`, roadmap Part 4). The engine in `bookx-sources` has ranking, field-priority merge and backoff, with mock-only tests (no live tests). Bulk fetch and a review screen are not in the UI (AGENTS.md: only the edit dialog uses it).
- **Proposal.** (1) Bulk "Fetch metadata for selection" that shows a review table (book, current vs proposed, confidence, source) and applies only ticked rows (M). (2) ISBN/identifier first, title+author second, and say which one matched (S). (3) Per-source health indicator plus a local cache and polite rate limiting, so one bad source never blocks the rest (S). (4) Never scrape sites whose terms forbid it (roadmap S6 note). (5) Optional live-network tests behind `NET=0`. Effort M, priority P1.

## PP-C-5 Conversion quality and failures

- **Description.** Output layout is often broken after conversion, PDF input is the worst case, defaults surprise (comics turned grey), and users end up looking for pristine EPUBs instead. Calibre's own FAQ says to treat PDF as hard. Many people want to avoid conversion altogether.
- **Update 2026-11-12 (metadata track):** *Check Before Sending…* runs the validator before converting, and every *Convert To* writes `<output>.report.txt` (target, sizes, findings of source and result, a note about fonts/SVG in non-EPUB targets). Still open: before/after preview in the reader, a messy-book golden corpus, the "send as is" default.
- **Evidence.**
  - anarc.at, conversion "doesn't work very well" and layouts break: https://anarc.at/software/desktop/calibre/
  - Hacker News: Calibre is fine only if you configure it, with a thread about why its EPUB conversion is criticised: https://news.ycombinator.com/item?id=20334541
  - MobileRead, AZW3 to EPUB lost letters, fixed by going through MOBI: https://www.mobileread.com/forums/showthread.php?t=355255
  - LWN, PDF from InDesign converts to awful formatting, comics default to black and white: https://lwn.net/Articles/1015226/
  - How-To Geek, formatting is unpredictable: https://www.howtogeek.com/reasons-i-convert-ebooks-with-calibre-before-sending-them-to-my-kindle/
- **How often.** High.
- **Bookx today: PARTIAL.** Writers for EPUB 2/3, KEPUB, FB2, HTMLZ, TXTZ, MOBI, AZW3/KF8, DOCX, RTF and more exist, checked against Calibre as an oracle, with 11 transforms, presets and progress/cancel (`bookx-convert`, roadmap). Known differences from Calibre are listed (no inline TOC page, fonts/SVG dropped, one fragment per file). Not tried on a real Kindle. No PDF input (D3). The EPUB validator has 87 rules (`bookx_editing::validate`).
- **Proposal.** (1) Make "do not convert" the default path: if the target device takes EPUB/KEPUB, send as is (S). (2) After every conversion run the validator and show a short report (warnings for dropped fonts/SVG, missing TOC, huge files) in the Convert dialog (S). (3) A before/after preview in the reader so the user sees the result before saving (M). (4) A golden-corpus regression suite of "messy real books" (messy EPUBs, comics, HTML-heavy) to guard against layout regressions (M). (5) Preserve comic colour by default, grey only for the e-ink presets (S). Effort M, priority P1.

## PP-C-6 Kindle: format churn, KFX and DRM

- **Description.** Amazon keeps changing the rules. Send to Kindle dropped MOBI/AZW in 2022 and accepts EPUB; USB download of purchased books was closed in early 2025; KFX needs plugins; DRM-removal plugins break after Amazon or Calibre updates. People get "my Kindle won't take books from Calibre" because Kindles do not read plain EPUB over USB.
- **Update 2026-11-12 (metadata track):** the preflight has Kindle and e-mail rules (language required, DRM refused with a plain message, 25 MB mail limit) with one-click fixes. Update 2026-11-13: DRM is now detected at import (Adobe, Kindle/KFX, Kobo, Apple, LCP) and explained; Bookx never removes it. Still not tried on a real Kindle.
- **Evidence.**
  - MobileRead, cannot convert KFX any longer, best route is downloading to a Kindle and using DeDRM + KFX Input: https://www.mobileread.com/forums/showthread.php?t=366766
  - LWN notes Amazon closed the USB download route in February 2025: https://lwn.net/Articles/1015226/
  - Calibre-Web issue copying Amazon's notice that MOBI/AZW sending ends in 2022: https://github.com/janeczku/calibre-web/issues/2393
  - MobileRead, Kindle suddenly not accepting books from Calibre: https://www.mobileread.com/forums/showthread.php?t=351233
  - r/kindle on the Send to Kindle change to EPUB: https://www.reddit.com/r/kindle/comments/vjjcdc/changes_to_epub_and_mobi/
- **How often.** High.
- **Bookx today: PARTIAL.** Send by e-mail is in the UI (SMTP via lettre, Preferences → E-mail). Send to device over USB converts to AZW3 for Kindle (`src/devices.rs`). MOBI/AZW3 are readable and writable. DRM books are rejected with a clear message (`crates/bookx-formats/src/mobi.rs` lines 304-307). Not tried on a real Kindle. KFX is a Calibre plugin, not core, so under D4 it is not owed. Removing DRM is out of scope and I recommend it stays so.
- **Proposal.** (1) For "Send by e-mail to Kindle", send EPUB (what Amazon now accepts) after validating it, and explain in the dialog why (S). (2) A "Kindle rejected my book" checklist driven by the validator (broken TOC, bad zip, huge images, missing cover) (M). (3) A friendly DRM message that names the cause and the lawful options (buy DRM-free, ask the publisher), without links to removal tools (S). (4) Test on real hardware and record the result in the docs (S, needs a device). Effort M, priority P2.

## PP-C-7 Forced `Author/Title` folder layout; files move on every edit

- **Description.** Calibre stores every book at `Author/Title (id)/…`, adds `cover.jpg` and `metadata.opf` beside it, and renames and moves files when metadata changes. The FAQ says plainly this will not change. Users who browse the folder from KOReader, Syncthing, git-annex or a file manager find it heavy; users with an existing folder tree cannot keep it.
- **Evidence.**
  - Calibre FAQ, the folder-structure answer: https://manual.calibre-ebook.com/faq.html
  - anarc.at: 355 authors and 952 books show the pain, with a script to flatten the tree: https://anarc.at/software/desktop/calibre/
  - r/calibre "Moving to Calibre" quoting the same answer: https://www.reddit.com/r/Calibre/comments/vyv9eu/moving_to_calibre/
  - r/selfhosted, Calibre-based services are opinionated about how books are stored: https://www.reddit.com/r/selfhosted/comments/1rslu16/list_of_self_hosted_book_services/
- **How often.** High.
- **Bookx today: NOT YET.** `crates/bookx-core/src/library.rs` (module doc, lines 1-9) copies the same shape: `<root>/Author/Title (id)/Title - Author.epub` plus `cover.jpg`. `reorganize` (about line 598) moves the files on every metadata edit and ignores errors from `rename` (`let _ =`), which can leave a book split across two folders. The preference text says "Calibre-style folder tree" (`src/ui/preferences.rs:31`). The template language already exists in `bookx-sources` (L24 subset) but is not used for paths (roadmap line 527: "template-based save paths" still open).
- **Proposal.** (1) Add a library layout setting: *Bookx tree* (today), *Flat* (`Author - Title.ext`), *Template* (uses the L24 template engine), and *Linked* (books stay where the user put them, only the catalogue knows them) (M/L). (2) Never move files on metadata edit unless layout is not Linked, and show "files will be renamed" before bulk edits (S). (3) Make `reorganize` transactional: plan all moves, apply, roll back on first error, surface the error (S). (4) A "change layout" wizard with a dry run (M). Tests: layout property tests (no collisions, unicode, long names), rollback test, UI scenario. Effort M-L, priority P1.

## PP-C-8 Governance, bus factor, AI features, telemetry

- **Description.** One person makes the decisions and writes most of the code (LWN counted over twice as many commits as the next 67 people combined, and found no governance document). In late 2025 AI features were added to the viewer and editor; opt-in only, but a fork with AI removed appeared within days and others followed. Calibre also reports usage statistics from the upstream build.
- **Evidence.**
  - LWN, "Catching up with calibre": sole decision maker, usage statistics sent on update check: https://lwn.net/Articles/1015226/
  - LWN, AI "discussion" feature added: https://lwn.net/Articles/1049886/
  - Hacker News on the Clbre fork with AI stripped: https://news.ycombinator.com/item?id=46177714 and the repository https://github.com/grimthorpe/clbre
  - Another fork on Codeberg that removes AI and update checks: https://codeberg.org/rereading/arcalibre
  - Charlie Stross on the fork: https://wandering.shop/@cstross/115684588990137011
- **How often.** High in 2025-26 (loud, but a narrower audience than the UI complaints).
- **Bookx today: PARTIAL.** Principle X15 (telemetry-free, offline-first, no accounts) is in the roadmap, and there is no network call at startup that I found. But there is no written policy, and Bookx itself currently has one owner, so the same bus-factor critique applies.
- **Proposal.** (1) `GOVERNANCE.md` and `PRIVACY.md`: no telemetry, no AI features that send book text anywhere, network only on explicit user action (S). (2) A test that fails if the app opens a socket at startup (S). (3) CONTRIBUTING plus a plugin-first stance (D5) so extras live outside the core. (4) Keep the crate boundaries documented so a second maintainer can own a crate. Effort S, priority P2.

## PP-C-9 Plugin breakage on Python, Qt and Calibre upgrades

- **Description.** Plugins are Python and depend on Calibre internals, so each major release or Python/Qt move breaks some. DeDRM and KFX Input are the best known, and their trackers fill with "not working after update". The Python 3 port was also a long, visible disruption.
- **Evidence.**
  - MobileRead, DeDRM version obsolete and not working with Calibre 7: https://www.mobileread.com/forums/showthread.php?p=4270123
  - DeDRM issue tracker, failure on Calibre 7.25: https://github.com/apprenticeharper/DeDRM_tools/issues/2392
  - MobileRead, plugin widget error "takes 2 positional arguments but three were given" after an update: https://www.mobileread.com/forums/showthread.php?t=361670
  - anarc.at on the Python 3 delay and Debian removal: https://anarc.at/software/desktop/calibre/
- **How often.** High.
- **Bookx today: PARTIAL.** `bookx-plugin` is a sandboxed WASM host (wasmi) with a versioned ABI v1, manifest and adapters, and the app lists and enables plugins in Preferences (roadmap Part 4). That removes the Python/Qt coupling. I did not find a compatibility policy or a test that keeps old plugins working across Bookx releases.
- **Proposal.** (1) Document ABI versioning and a "supported for N releases" promise, with `min_abi`/`max_abi` in the manifest (S). (2) Keep a folder of prebuilt reference plugins in `crates/bookx-plugin/tests` that every release must still load (S). (3) A plugin error never takes the app down: failures show a banner naming the plugin (S). Effort S, priority P2.

## PP-C-10 Not made for several users or processes

- **Description.** The Calibre desktop app assumes one user. People try to share one library between desktop Calibre and Calibre-Web or a content server and run into locking, overwritten changes and permissions. Reading position and per-user state are not first class.
- **Evidence.**
  - r/calibre: Calibre is not designed for multi-user access, something will break eventually: https://www.reddit.com/r/Calibre/comments/r968us/basic_questions_about_calibreweb/
  - Calibre-Web issue on file locking between Calibre and Calibre-Web: https://github.com/janeczku/calibre-web/issues/1841
  - Cloudron forum, web-app changes overwritten when syncing from the desktop library: https://forum.cloudron.io/topic/10210/how-best-to-update-sync-my-calibre-web-and-my-local-calibre-library
  - Calibre-Web issue on a second Kobo device breaking sync: https://github.com/janeczku/calibre-web/issues/2230
- **How often.** High.
- **Bookx today: PARTIAL.** One SQLite database serves the app, CLI and server; the server follows commits (`CatalogWatcher`). Share Library needs a password for LAN access. Per-user accounts, per-user reading state and permissions (roadmap P10 mentions them) are not in the progress log.
- **Proposal.** (1) Define the supported topologies in docs: one writer (app or `serve`) plus any number of readers (S). (2) Per-user accounts for the server, with per-user progress, shelves and download permission (L). (3) Concurrency test: app and `serve` writing at once on the same library (S). Effort L, priority P2.

## PP-C-11 Startup and large-library speed

- **Description.** Reports range from minutes to start (an old, extreme case, probably a bad machine) to slowness with very large libraries or a lot of sort levels. The upstream developer's position is that 100,000 books search fine and the cause is local, so the evidence is mixed.
- **Evidence.**
  - KBoards, a start taking about seven minutes: https://www.kboards.com/threads/why-is-calibre-sooo-slow.26813/
  - r/calibre, slow with more than 200,000 books, with advice to check resource use: https://www.reddit.com/r/Calibre/comments/kg5mv6/calibre_slow_with_large_libraries_200000_books/
  - Launchpad bug "Searching Extremely Slow", closed with the developer saying 100,000 books search quickly: https://bugs.launchpad.net/bugs/1862972
  - Calibre's own tweaks file warns that each sort level costs time on big libraries: https://github.com/kovidgoyal/calibre/blob/master/resources/default_tweaks.py
- **How often.** Medium.
- **Bookx today: PARTIAL.** The library grid is virtualised (`TypedGridView` in `src/ui/library.rs`). Budgets exist for search, sort, catalogue load, save of one edit among 10,000, reopen and parallel import (`crates/bookx-core/tests/perf_budgets.rs`, `crates/bookx-store/tests/perf_budgets.rs`). Nothing is measured at 100,000 books, for cold start of the GUI, or for cover-scroll frame rate (the roadmap itself lists "reader/scroll budgets TODO").
- **Proposal.** (1) Budgets at 100,000 books for load, search, `text:` search and tag-browser counts (S). (2) A GUI cold-start budget measured in a UI scenario (time to first window with a 10,000-book library) (S). (3) Lazy cover loading with an on-disk thumbnail cache, tested at 10,000 covers (M). Effort S-M, priority P1, since "instant by contract" is the headline of the Bookx pitch.

## PP-C-12 Kobo: KEPUB, plugins and device sync

- **Description.** On Kobo, Calibre needs extra plugins for KEPUB output and for shelves and metadata updates. Users see side-loaded KEPUB files that show up in the file list but not in the library, or sync that silently skips books. A well-known publisher warns that Calibre's own transfer changes its KEPUB files in surprising ways.
- **Evidence.**
  - MobileRead, side-loaded KEPUB shows on the Kobo but not in the library: https://www.mobileread.com/forums/showthread.php?t=364735
  - MobileRead, Kobo/Calibre not syncing everything: https://www.mobileread.com/forums/showthread.php?t=340132
  - LWN, Standard Ebooks warning about Calibre and KEPUB: https://lwn.net/Articles/1015226/
  - Calibre-Web-Automated issue, Kobo Clara BW cannot download an EPUB: https://github.com/crocodilestick/Calibre-Web-Automated/issues/975
- **How often.** Medium.
- **Bookx today: PARTIAL.** A native KEPUB writer exists, and Send to device detects Kobo, converts to KEPUB and copies it (`src/devices.rs`, tests `sends_a_kepub_to_a_kobo`). The roadmap states "no Kobo DB write" (Part 4, Send to device row), so the Kobo does not learn of the book, shelves or read state until its own import runs.
- **Proposal.** Write the required rows into `.kobo/KoboReader.sqlite` with `rusqlite` (allowed by D2) so that sent books appear in the library at once, with shelf (collection) support and optional read-state read-back (L). Always back up the device database first and refuse to touch it if the schema version is unknown. Tests need fixture databases for several firmware versions. Effort L, priority P2.

## PP-C-13 Qt look, Wayland, HiDPI and dark mode

- **Description.** Calibre ships its own palette and does not follow the system theme by default, so dark mode needs a setting or an environment variable. On Wayland and with fractional scaling there are redraw and scaling reports. Several Flatpak and distro issues date back years.
- **Evidence.**
  - Manjaro forum, Calibre not redrawing under Wayland and KDE: https://forum.manjaro.org/t/calibre-rendering-issues-in-wayland/148617
  - Red Hat bug, Qt apps including Calibre scaling badly on Wayland: https://bugzilla.redhat.com/show_bug.cgi?id=1381828
  - Flathub issue "Make calibre use the system theme": https://github.com/flathub/com.calibre_ebook.calibre/issues/34
  - Guide to changing the default theme when it does not follow the system: https://www.adamsdesk.com/posts/calibre-theme-light-dark-mode/
- **How often.** Medium.
- **Bookx today: DONE (by construction).** GTK4 + libadwaita follows the system and supports Wayland natively; `adw::StyleManager` sets System/Light/Dark (`src/app.rs:64-67`); the Flatpak manifest uses `--socket=wayland` with X11 fallback (`build-aux/com.adhadse.Bookx.json`). Caveat: the reader draws text itself with cairo (`src/ui/canvas.rs`), and fractional-scale sharpness of that canvas is not covered by any test I found.
- **Proposal.** Add a UI scenario that runs at scale 2 under Broadway and a manual checklist for fractional scaling on Wayland (S). Priority P3.

## PP-C-14 Content server is basic and open by default

- **Description.** The built-in server is useful but plain: no format conversion, limited management, and the default is open to anyone who can reach the port. Some users put it behind a reverse proxy because they do not trust the built-in authentication. Reading position is not kept server-side in Calibre-Web either.
- **Evidence.**
  - LWN, default configuration wide open and the interface "pared-down": https://lwn.net/Articles/1015226/
  - anarc.at, runs it behind Apache with its own authentication: https://anarc.at/software/desktop/calibre/
  - Calibre-Web issue, last-read position not stored on the server: https://github.com/janeczku/calibre-web/issues/1797
- **How often.** Medium.
- **Bookx today: PARTIAL.** Own hardened hyper server: limits, deadlines, connection caps, auth-failure throttle, optional HTTPS in the CLI (`--tls-cert/--tls-key`), a password required for LAN in the app (roadmap Part 4). Gaps: no TLS in the app's Share Library, response bodies are buffered in memory (roadmap line 484), no per-user accounts.
- **Proposal.** (1) TLS option in Share Library with a self-signed certificate generator and fingerprint display (M). (2) Stream responses (S). (3) Reverse-proxy and `systemd` unit docs (S). Effort M, priority P2.

## PP-C-15 Duplicate handling

- **Description.** Calibre checks title and author when adding and asks; the real work needs a plugin. Bugs appear where auto-add and duplicate detection interact with other plugins, and users worry about accidental loss when merging.
- **Evidence.**
  - Launchpad bug, auto-add with duplicate detection interferes with file-type plugins: https://bugs.launchpad.net/bugs/2003906
  - MobileRead, a user believing Calibre deleted "duplicates" of different books: https://www.mobileread.com/forums/showthread.php?t=344366
  - Calibre-Web feature request for duplicate upload detection: https://github.com/janeczku/calibre-web/issues/526
  - Calibre-Web-Automated 4.0 added automatic duplicate handling, which shows the demand: https://github.com/crocodilestick/Calibre-Web-Automated/discussions/941
- **How often.** Medium.
- **Bookx today: DONE.** Duplicate finder with keep/merge (metadata union, highlights, bookmarks, furthest progress) in `src/duplicates.rs` and `src/ui/duplicates.rs` (roadmap line 508), `Library::find_duplicate` at import, and detection logic in `bookx-sources` (L7). I did not verify how well matching works on messy titles or ISBN-less books.
- **Proposal.** Add a messy-title corpus test (subtitles, series prefixes, "The" first, translated titles) and an ISBN/identifier first pass (S). Make merge reversible via the undo proposed in PP-C-24. Priority P3.

## PP-C-16 No automatic backups; slow restore on big libraries

- **Description.** The usual advice is to back up `metadata.db` with an external tool. Users ask for periodic automatic copies inside Calibre, and the built-in recovery rebuilds the whole catalogue slowly.
- **Evidence.**
  - MobileRead request for a rolling backup of `metadata.db` every few minutes: https://www.mobileread.com/forums/showthread.php?t=347713
  - r/calibre NAS thread, where backups are what users reach for: https://www.reddit.com/r/Calibre/comments/1cky1qf/question_about_calibreweb_on_a_nas_and_the_risks/
  - aroundmyroom write-up of database trouble on network drives and the recovery path: https://aroundmyroom.com/2018/07/14/calibre-calibre-web-with-a-network-share/
- **How often.** Medium.
- **Bookx today: PARTIAL.** `backup_to`/`restore_from` exist and are tested but nothing in the app or CLI calls them (grep of `src/` and `crates/bookx-cli/src`); the roadmap says "no scheduler". `bookx-cli library dump` writes the whole catalogue as JSON, which is a manual export.
- **Proposal.** Automatic snapshots: on close and every N minutes while changing, keep the last 5 plus one per day for a week, in `<library>/.bookx-backups/`. A "Restore from backup…" dialog listing snapshots with book counts and dates. CLI `library backup|restore|snapshots`. Use SQLite's online backup so it is safe while the app runs. Tests: retention rules as a pure function, restore round trip, UI scenario. Effort S-M, priority P1.

## PP-C-17 No official mobile reader

- **Description.** Calibre has no reading app for phones. The download page points to third-party companion apps, and forum answers say the same. Users end up using OPDS apps or the content server in a mobile browser.
- **Evidence.**
  - Calibre download page, listing mobile options: https://calibre-ebook.com/download
  - r/calibre "Does Calibre have a phone app?": https://www.reddit.com/r/Calibre/comments/1xerz4c/does_calibre_have_a_phone_app/
  - MobileRead, there is no Calibre reader for Android or iOS, only a companion tool: https://www.mobileread.com/forums/showthread.php?p=4138609
- **How often.** Medium.
- **Bookx today: PARTIAL.** OPDS server, web page and KOSync work with phone readers (KOReader, Librera, etc.). Adaptive layout for phones (X5) and touch mode exist (`src/gestures.rs`); not tried on a real touch screen (roadmap line 505). No installable web app (PWA) or browser reading mode is confirmed.
- **Proposal.** (1) Test the OPDS feed against 3-4 common apps and publish the list (S). (2) A responsive in-browser reader for the web page with progress sync through the existing KOSync endpoint (L). (3) Test the GTK app on Phosh/postmarketOS hardware (S, needs a device). Effort L, priority P2.

## PP-C-18 Database as a black box; lock-in and migration

- **Description.** People find the catalogue opaque, want their folders to be the truth, and find it hard to leave or to run other tools next to Calibre. Alternative tools (Kavita, Booklore, CWA) read Calibre's data but lose fields such as series or date added. Alternatives also churn: one that people migrated to is reported as defunct.
- **Evidence.**
  - r/selfhosted "Those of you not using Calibre": biggest gripe is the database being a black box: https://www.reddit.com/r/selfhosted/comments/1clh94h/those_of_you_not_using_calibre_what_is_you_book/
  - r/selfhosted list of services, Calibre-based ones are opinionated about storage: https://www.reddit.com/r/selfhosted/comments/1rslu16/list_of_self_hosted_book_services/
  - anarc.at: Kavita does not carry over series and date added: https://anarc.at/software/desktop/calibre/
  - A write-up of migrating from Booklore back to Calibre-Web: https://www.bentasker.co.uk/posts/blog/general/migrating-from-booklore-to-calibre-web-automated.html
- **How often.** Medium.
- **Bookx today: PARTIAL.** `SqliteStore::import_calibre` reads a Calibre `metadata.db` (tested with 12 cases in `crates/bookx-store/tests/calibre.rs`), but no code in `src/` or `bookx-cli` calls it, so a Calibre user cannot import from the GUI or the CLI yet. `library dump` gives a JSON export. There is no export back to Calibre (roadmap L10 says "and export back").
- **Proposal.** (1) "Import a Calibre library…" in the Welcome screen and `bookx-cli library import-calibre <dir>` with a dry run, progress, a report of unmapped fields (custom columns, plugboards) and a choice of copy or link (M). This is the fastest on-ramp (roadmap L10). (2) Export to a Calibre-compatible folder with `metadata.db` and OPF, for people who want a way back (M). (3) Documented schema for `bookx-library.db` (S). Effort M, priority P1.

## PP-C-19 Installation and update friction

- **Description.** On Linux the project recommends a download-and-run installer script and discourages distro packages; the Flatpak is third-party. New releases come about every two weeks, so distro packages go stale and device support can lag. Debian once had to hold back an old version because of the Python 3 port.
- **Evidence.**
  - LWN, installer script, no official Flatpak, releases every two weeks: https://lwn.net/Articles/1015226/
  - anarc.at, Debian carried an old version during the Python 3 port: https://anarc.at/software/desktop/calibre/
  - Flathub issue about the unofficial Flatpak not matching the system: https://github.com/flathub/com.calibre_ebook.calibre/issues/34
- **How often.** Medium.
- **Bookx today: PARTIAL.** A Flatpak manifest exists (`build-aux/com.adhadse.Bookx.json`, GNOME 49 runtime), `./run.sh --no-run` installs locally, and metainfo is present. There is no published release process in the repo that I found (roadmap X7/X16: release channels, AppImage, deb/rpm still open).
- **Proposal.** CI that builds the Flatpak on every tag, publishes to Flathub, and attaches a static `bookx-cli` binary; no self-updater, since the Flatpak handles it and the update check must be opt-in (see PP-C-8). Effort M, priority P2.

## PP-C-20 Weak built-in reader

- **Description.** People say the viewer is not for reading: jump-to-page is a multi-step dialog instead of a slider, page changes can feel slow, opening panels reflows the text, and no controls show by default.
- **Evidence.**
  - Hacker News comment on the viewer's go-to-page steps, slow rendering and bookmark panel reflow: https://news.ycombinator.com/item?id=43434421
  - LWN, the viewer shows no controls until right-click: https://lwn.net/Articles/1015226/
- **How often.** Medium (lower among people who only use Calibre as a manager).
- **Bookx today: PARTIAL.** The reader has its own canvas with hyphenation, highlight styles, dictionary popup, stats, tap zones and e-ink mode. A page/progress slider, book map and skim (R12/R25/R27) exist as data in `bookx-lookup` but are not in the UI. Knuth-Plass is not in the canvas (roadmap line 527).
- **Proposal.** Slider with chapter ticks and a live preview label, a back/forward location stack (R26), and a perf budget for page turn. Effort M, priority P3.

## PP-C-21 Reading progress does not follow you

- **Description.** Calibre's viewer keeps progress locally; web front ends store it in the browser. People ask for server-side position and sync with KOReader or Kobo, and a popular fork added KOReader sync because of it.
- **Evidence.**
  - Calibre-Web issue on keeping the last-read position across devices: https://github.com/janeczku/calibre-web/issues/1797
  - Calibre-Web-Automated README, built-in KOReader sync: https://github.com/crocodilestick/calibre-web-automated
- **How often.** Medium.
- **Bookx today: DONE.** KOSync-compatible server is part of Share Library (roadmap P3/P12, `src/sharing.rs`, `bookx-serve`).
- **Proposal.** Sync of highlights and vocabulary is not covered; add it later (sidecar import/export, R32). Priority P3.

## PP-C-22 Security record and phone-home

- **Description.** A Debian security contact lists years of issues: code execution via news or plugin updates, file access via crafted files, XSS, embedded libraries, a long-running mount-handling bug. Separately, upstream builds report usage statistics.
- **Evidence.**
  - anarc.at, long list of Debian bug numbers: https://anarc.at/software/desktop/calibre/
  - LWN on the installation UUID and statistics: https://lwn.net/Articles/1015226/
- **How often.** Low-medium (mostly old, from a small number of authors).
- **Bookx today: PARTIAL.** Hardened server and WASM sandbox for plugins (roadmap Part 4). I did not verify what the reader does with scripts, remote URLs and SVG in EPUBs, and the e-mail password is stored in the settings file (roadmap line 513).
- **Proposal.** (1) Tests that the reader never runs scripts or fetches remote resources from a book (S). (2) Store passwords in the Secret Service through `oo7` or `libsecret` instead of the settings file (M). (3) `cargo audit`/`cargo deny` in CI (S). Effort M, priority P2.

## PP-C-23 Accessibility

- **Description.** Evidence is thin. The sources I could find speak of a non-native look and hidden controls rather than screen-reader failures. The Qt app does not follow platform conventions (LWN). I did not find a specific, well-supported Calibre accessibility bug thread in this pass, so treat this as a risk to check, not a confirmed top complaint.
- **Evidence.**
  - LWN: bespoke GUI that matches no platform convention, viewer controls hidden: https://lwn.net/Articles/1015226/
  - Hacker News: discoverability and tooltip-only documentation: https://news.ycombinator.com/item?id=43434421
- **How often.** Low (in what I found).
- **Update: PARTIAL, see `docs/a11y-notes.md`** (named controls with an audit, page text for Orca, keyboard-only reading, high-contrast palette). Old state: **NOT YET.** `src/` had no accessible-name/role settings for custom widgets (search for `accessible` only matches the EPUB accessibility editor tool). The reader draws text on a cairo canvas (`src/ui/canvas.rs`), which exposes nothing to a screen reader. R64 (screen reader, high contrast, dyslexia fonts, reduced motion) is not in the progress log. The EPUB accessibility checker for books is done (E19), which is a different thing.
- **Proposal.** (1) Accessible labels for every icon-only button and the cover grid items (S). (2) Expose reader text to AT-SPI, or provide a "text view" mode that uses a real `GtkTextView` for screen-reader users (L). (3) Respect reduced-motion and high-contrast settings from libadwaita (S). (4) UI test that every button has a name. Effort L, priority P2.

## PP-C-24 Risky merge and delete, no library undo

- **Description.** Merging records is a menu of three similar entries whose result depends on selection order, and people say they are careful because a mistake is hard to undo. Delete in Calibre goes to the system trash on desktop, but there is no undo for metadata edits.
- **Evidence.**
  - Hacker News, merge-records submenu criticised, with a suggestion to use a comparison table: https://news.ycombinator.com/item?id=43434421
  - MobileRead, user fears that Calibre deletes different books as "duplicates": https://www.mobileread.com/forums/showthread.php?t=344366
- **How often.** Low.
- **Bookx today: NOT YET.** Undo exists only in the editor text buffer and editor checkpoints (`src/ui/editor/mod.rs`, `src/ui/editor/tools.rs`). L37 (library-level undo) is in the roadmap but not in the progress log. The duplicate finder merges with a defined rule but no undo.
- **Proposal.** Record each library operation (edit, bulk edit, merge, delete, layout move) in an `operations` table with the previous values; show "Undone: …" toasts with an Undo button (libadwaita `AdwToast`); keep deletes in a library trash for 30 days. Tests: undo/redo round trip as pure functions over the store. Effort M, priority P2.

## PP-C-25 News recipes are clunky and Python-bound

- **Description.** Calibre ships hundreds of recipes, but they are Python and tied to the site's markup, so they break; the output piles up unread books on a device and needs manual transfer.
- **Evidence.**
  - anarc.at, news generation was clunky and the books went unread: https://anarc.at/software/desktop/calibre/
  - LWN on recipes needing the `BasicNewsRecipe` API for custom sources: https://lwn.net/Articles/1015226/
- **How often.** Low.
- **Bookx today: PARTIAL.** Feed to news book works (`src/web_import.rs`, `bookx-sources` has recipes and schedules); no scheduling UI, no recipe editor, no Wallabag (roadmap line 509).
- **Proposal.** Declarative recipe files (TOML) with a "test this recipe" button, a schedule that sends the result to the device on connect, and auto-delete of old issues. Effort M, priority P3.

---

## Top 10 things Bookx should do next

1. **Wire backup, restore and integrity check into the app and CLI, with automatic rolling snapshots** (PP-C-2, PP-C-16). The code exists and is tested; nothing calls it. Effort S-M.
2. **Import a Calibre library from the GUI and `bookx-cli`**, with a dry run and a report of unmapped fields (PP-C-18). `import_calibre` exists and has 12 tests, so this is mostly UI. Effort M.
3. **Library layout choice: flat, template, linked-in-place, with transactional moves** (PP-C-7). This is the one place Bookx copies Calibre's most-resented decision, and `reorganize` ignores rename errors today. Effort M-L.
4. **Network-folder and sync-client detection, an advisory lock, and a read-only mode, plus a "run on a NAS" doc that points to `bookx-cli serve`** (PP-C-3, PP-C-10). Effort S-M.
5. **Bulk metadata fetch with a review table** and per-source health (PP-C-4). Effort M.
6. **Conversion report and "send as-is" defaults**: validate after each conversion, prefer EPUB/KEPUB when the device takes it, preview before saving (PP-C-5, PP-C-6). Effort M.
7. **Library-level undo and a trash** (PP-C-24), which also makes duplicate merge and layout changes safe. Effort M.
8. **Command palette and one searchable Preferences page**, and redraw the mockups (PP-C-1, PP-C-20). Effort M.
9. **Kobo database write** so sent books appear at once with shelves (PP-C-12). Effort L, needs fixtures per firmware.
10. **Performance budgets at 100,000 books and for GUI cold start**, plus accessible names on every control and a screen-reader path for the reader (PP-C-11, PP-C-23). Effort S for the budgets, L for the reader.

Runners-up: `GOVERNANCE.md`/`PRIVACY.md` and a no-network-at-startup test (PP-C-8); plugin ABI compatibility promise (PP-C-9); Secret Service for stored passwords (PP-C-22); TLS in Share Library (PP-C-14).

## Claims I could not verify

- Several search results gave only a title and a snippet; I did not read the full thread. These are PP-C-3 (aroundmyroom, It's FOSS, Cloudron), PP-C-4 (all four links), PP-C-9 (DeDRM issue, plugin widget thread), PP-C-10 (Calibre-Web issues 1841, 2230, Cloudron), PP-C-12 (all four), PP-C-13, PP-C-15, PP-C-17, PP-C-18 (r/selfhosted threads, Bentasker), PP-C-21, PP-C-25 (LWN comment only). Full pages I read: the Hacker News thread 43434421, anarc.at, LWN 1015226, and MobileRead 347713.
- Reddit pages could not be fetched (the fetch returned only the site name), so r/calibre, r/selfhosted and r/kindle claims rest on search snippets.
- The Calibre FAQ's network-drive and folder-structure answers: quoted through the MobileRead thread and anarc.at, not read directly.
- Whether the "seven-minute start" on KBoards is from an old Calibre version; the post is old and the date was not checked.
- Frequency labels are my judgement from the number of independent sources found; I did not count posts in a systematic way, and search engines favour popular threads.
- The Booklore "defunct" claim comes from a single blog title (Bentasker); I did not confirm it.
- The AI feature timeline (version numbers and dates) is from LWN and search snippets; I did not check Calibre's changelog.
- That Amazon closed the USB download route "in February 2025": LWN says so; I did not read The Verge article it links.
- Accessibility (PP-C-23): I found no direct Calibre accessibility bug reports in this pass. It may exist on Launchpad or MobileRead, and a targeted search is still owed.
- Bookx side: I did not run the app or the tests. Statuses come from reading source and the roadmap. I did not check that bulk fetch is absent from the library UI beyond reading `AGENTS.md` and the roadmap, that no command palette exists beyond searching for it in the roadmap progress log, or that the reader never runs scripts in EPUBs. Real Kindle, real Kobo and real touch-screen behaviour is untested by the project itself (roadmap lines 465, 505).
- Hardware-dependent claims about Qt/HiDPI in Calibre 8/9 (Qt 6) were not re-tested; the cited Wayland and HiDPI reports are older and may be fixed upstream.
