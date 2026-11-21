# Remaining work (internal survey, 2026-11-21)

Internal, not for the wiki. Built from a read-only pass over every `docs/*.md`, `AGENTS.md`, `README.md`, `CHANGELOG.md`,
`SECURITY.md`, `NOTICE.md`, `_developer/docs.md` and the "limitation / not yet / untested" statements of `wiki/docs/*.md`,
spot-checked against `src/` and `crates/`. State: after waves 6-9, backup D9, bookmarks and the highlight sync repair.

Columns: **Size** S (under a day), M (1-3 days), L (about a week or more). **Who**: `agent` = can be done in this repo without
hardware or accounts; `owner` = needs the owner's devices, accounts, a decision or a GitHub/DNS setting; `both` = agent builds,
owner verifies. "Stale" = the doc says open, code or tests show it is done (or the other way round).

## Counts

| What | Number |
|---|---|
| HIG backlog rows (`human_interface_backlog.md` §4-§5.7, row-id tables) | about 205 rows, about 150 done; **31 rows still not `addressed`** (see C) |
| of which stale `open` rows in §5.6 (hamburger tables) | 30 rows say `open`, but wave 8/9 did them; real remainder is 2 items (C.hamburger) |
| Roadmap rows (`roadmap-status.md`, audited 2026-11-13, partly stale) | 176 rows: 87 DONE, 59 PARTIAL, 26 MISSING, 3 DEFERRED, 1 ENGINE ONLY (after the ux-gaps pass; waves 6-9 moved more to DONE) |
| Calibre editor rows (`calibre-editor-gap-analysis.md`) | 265: 254 DONE, 4 PARTIAL, 2 MISSING, 5 NOT PLANNED |
| Pain points (calibre / general) | 25: 10 DONE, 15 PARTIAL, 0 NOT YET; 33: 17 DONE, 16 PARTIAL, 0 NOT YET |
| `docs/bugs.md` | 24 rows, all `fixed`; open leftovers are "not verified on a device" notes (E) |
| UI scenarios | about 264 `scenario` blocks in `ui-tests.sh` plus 14 `ui-tests-*.sh` suites; 29 session scenario files |

### Open/partly HIG rows by area (still not `addressed by wave N`)

| Area | Total rows | Not done / partly | Rows |
|---|---|---|---|
| Library and navigation (5.1) | 58 | **2** | LIB-L6 (partly), LIB-E3 (skipped, modal stays) |
| Reader (5.2) | 26 | **0** | WIN-R9 and WIN-R3/R5 are counted under Windows |
| Editor (5.3) | 31 | **0** | (EDT-E21 verified, nothing to do) |
| Preferences and system (5.4) | 27 | **6** | PRF-P4, D7, D17, D18, D19, D23 |
| Sync (5.5) | 13 | **3** | SYN-S5, SYN-I2, SYN-I4 (partly; the rest of I4 judged not needed) |
| Windows opened from hamburgers (5.7) | about 45 | **17** | library 8 (WIN-L1, L4, L8, L13, L15a, L16, L18, L19), reader 3 (R3, R5, R9), editor 3 (E3, E4, E6), cross-cutting X6, X7, X9 |
| Cross-cutting (§4) | 7 | **4** | XC-1 (reader part done, rollout by area), XC-5 (partly), XC-6 (open), XC-7 (partly) |
| Hamburger menus (5.6) | about 45 | **2 real** (30 rows stale) | access keys/mnemonics in all three menus; reader Properties entry (WIN-R9) |

## Top 10 priorities

1. **Run the four checks the wave work left undone** (G.1): fix the 63 vacuous `check "...$(...)..." $?` calls, they silently pass. S, agent.
2. **Verify the KOReader plugin and wireless sync on the owner's Kobo** (A.1). The plugin has never run inside KOReader; the owner reported a panic (bug 17) and no text. M, owner.
3. **Verify highlight colours on a Kobo with native highlights** and the KOReader notes file layout (A.2, highlight-sync-rca). S, owner.
4. **Security P1 batch** (F): `is_external` still allows `ftp:`/`data:` (G6), `sh -c` in the TTS sink (G7), SSRF guard (G8), `Origin`/`Host` check (G9), fuzz targets and the evil corpus. M-L, agent.
5. **Run the GitHub workflows once** (`ci.yml`, `release.yml`, `wiki.yml`) and fix what breaks; set up Pages/DNS for `bookx.adhadse.com` (H.1). S, owner then agent.
6. **Standard keys XC-6**: F9 for library sidebar and editor sidebar, Ctrl+, / Ctrl+? / F10 in the editor window, Delete in library, list in the shortcuts dialog. S-M, agent.
7. **Refresh stale wiki screenshots** (library main menu still says "Share Library", Sync page, device window, Library menu) and `.po` files for wave 9 strings (H). M, agent (capture run).
8. **Real hardware pass for TTS and audiobooks** on a sound card (pause/resume, media keys, 3x speed). S, owner.
9. **Real-service backup checks**: Nextcloud/WebDAV, SMB/NFS NAS, GNOME Online Accounts drive, GNOME Keyring and the Flatpak secrets portal (A.4). M, owner.
10. **Owner decisions in B** (glossary Share vs Library server, reader Properties, mnemonics, OPDS as a sidebar page, confirm all "decided by default" rows). S, owner.

## A. Needs the owner's real hardware or accounts to verify

| Id / source | What | Size | Who |
|---|---|---|---|
| hardware-test-plan K1-K10 | Kindle (AZW3/MOBI, USB, EPUB on new firmware, unicode names, 20 MB book). Never tried on a Kindle. | M | owner |
| hardware-test-plan B3-B9, kobo-real-device-report | Kobo firmware import of Bookx's KEPUB (covers, reading-stats line, notes appendix), shelf write (`ShelfContent` for a not-yet-imported book), other models than Libra Colour | M | owner |
| hardware-test-plan P1-P6, X1-X4 | PocketBook, Boox (MTP/USB, Neoreader), Boox OPDS over Wi-Fi | M | owner |
| hardware-test-plan T1-T8, E1-E5 | Touch screen (tap zones, handles, pinch, OSK) and e-ink panel (refresh interval, ghosting) | M | owner |
| bugs 15, 17; koreader-annotation-sync, kobo-koreader-sync-runbook | Bookx plugin inside KOReader: menu, Wi-Fi sync, pcall wrappers, MD5; owner's panic text and when it happens is still wanted | M | owner |
| bug 12/22, highlight-sync-rca | Kobo `Color` 0..3 = yellow/pink/blue/green is from community docs; owner's Kobo has no native highlights (`Bookmark` empty). KOReader >= 2024 reading Bookx's `bookmarks` notes file: Bookx highlight may open as a plain bookmark (not fixed, needs the device) | S | owner (then agent) |
| sync-ux-notes, device-ui-notes, kobo-real-device-report | Device window flows, auto-sync switch, banner, covers: only simulated devices (`BOOKX_DEVICE_ROOTS`); "connected" state only reflects annotation sync | S | owner |
| backup-notes "Not done", wiki backup page | WebDAV vs a real Nextcloud/ownCloud (MOVE, 413 on big objects, slow small-file uploads); SMB/NFS NAS; Google Drive/OneDrive via GNOME Online Accounts; iCloud via synced folder | M | owner |
| backup-notes, sync-ux-notes, packaging-notes | Real Secret Service: `Oo7Store` never talked to GNOME Keyring or the Flatpak secrets portal; "keyring is unlocked at login" for the sync password | S | owner |
| tts-notes, audiobook-notes | Nothing heard on a real sound card: pause/drain, chapter prefetch, 130 MB voice download through the card, MPRIS media keys and shell widget, `pacat`/`pw-cat` start latency, 3x speed, part boundary clicks; XTTS/Fish servers only against fakes; Kokoro on a slow laptop RTF is an estimate; aarch64 never run | M | owner |
| mcp-notes | No real assistant (Claude Desktop, Claude Code, Cursor, opencode) run against the MCP server; Flatpak access from the host not tested | S | owner |
| packaging-notes | Flatpak sandbox: devices (`/run/media`, MTP via GVfs), voices, secrets portal, network-place backups, `bookx://` forwarding, "Open With" | M | owner |
| roadmap-status R20, S3/P4c | Anki/Readwise against the real services; OPDS against real Project Gutenberg / Standard Ebooks; Hardcover CSV against a real export | S | owner |
| comic-notes, ux-gaps-notes | Comics on a touch screen / e-ink; real comics for panel detection (only generated ones) | S | owner |
| bookx-feature-roadmap D10 | Mobile/Android reader or other KOReader hardware (any device that runs it) | S | owner |

## B. Open owner decisions

From `human_interface_backlog.md` §8 (what is still genuinely open) and other docs. Rows marked "default" were applied in wave 9
with the recommended option and only need a confirmation or a revert.

| Id | Question | Recommended | Who |
|---|---|---|---|
| §8 A, SYN-S4 rest | Noun for the shared library server and its UI: "Share", "Library server", OPDS names (Preferences, wiki, menu) | decide together with the Sync wording pass | owner |
| WIN-R9 | Add reader "Properties" to the Main Menu and which key (reader menu is at 12, so one entry must leave, e.g. Footer Stats) | keep it in the sidebar header and palette | owner |
| LIB-E3 | Online Catalogues (OPDS) as a sidebar page instead of a modal dialog (skipped, needs `opds.rs`) | do it later, large | owner |
| §8 A (mnemonics) | Access keys (`_` mnemonics) in the three hamburgers: every label gets a new msgid per language (not done) | decide whether worth it for 5 languages | owner |
| WIN-L1, WIN-R1, WIN-E6, X3 | "No label ends with an ellipsis" was applied by default; HIG wants an ellipsis for entries that open a chooser | keep the project rule or revert | owner |
| default: LIB-M6, LIB-B3, LIB-B4, LIB-N4 | Add Folder always recursive with count toast; click toggles once selecting; Move to Trash with Undo; three Anki entries plus an options dialog | confirm | owner |
| default: LIB-L2, LIB-T3 | Health pages Editions/Trash (History in Backups); daily goal as switch plus amount | confirm | owner |
| default: PRF-P2, PRF-G5, PRF-G10, PRF-D14, PRF-D12 | Twelve Preferences pages, Dictionaries page, Patches inside Extensions, E-ink mode in General, e-mail titles without "Mail", page named "MCP", Undo toast for Reset All Shortcuts | confirm | owner |
| default: RDR-R2, R3, R90, R5, R61, R40, R1, R92 | Previous/Next Position, 12/7/6 entry reader menus, Ctrl+G stays Go to, popovers scroll, pitch only in Voice Settings, header caps | confirm | owner |
| default: EDT-E4/E6, EDT-E34 | "Reformat Markup", Problems = list and Check = action; narration may run in background, result discarded if the book changed | confirm | owner |
| enhancements 3, wiki-notes | Custom domain, DNS and HTTPS for `bookx.adhadse.com`; Pages source set to Actions | do it | owner |
| security.md G1/G12 | Enable GitHub private vulnerability reporting, create the `security` label, name an advisory owner | do it | owner |
| security.md P2 | Signing tool for releases (cosign or minisign) and key custody | choose | owner |
| packaging-notes | Flathub submission (needs a tagged release URL, hash, stable screenshot URLs, `x-checker-data`); AppImage yes/no; aarch64 yes/no; GNOME 51 runtime move | decide | owner |
| i18n | Hosted Weblate project (wiki says none yet); who reviews es/de/fr/hi natively; translator credits for the About dialog | decide | owner |
| calibre-wireless-device-plan | Calibre wireless protocol stays deferred (D10: wireless = KOReader only). Confirm it can be dropped from the plan | confirm | owner |
| library-sync-design | Multi-device library sync (not implemented; backup is one-way). Build or never? | decide | owner |
| roadmap-status L3/L25 | Custom columns (store schema change, top gap); content-server accounts and permissions (P10, PP-C-10, PP-G-26) | decide scope | owner |
| backup-notes | Passphrase change needs a new repository; accept or build re-encryption | decide | owner |

## C. Remaining HIG backlog rows (by area)

Rows already marked `addressed by wave N` are not listed.

### Library and navigation
| Row | What | Size | Who |
|---|---|---|---|
| LIB-L6 | Shared selection pattern (XC-3: header Cancel and "N selected", hover checkbox) in `organize.rs` / `duplicates.rs` lists (Duplicates, Editions, Needs Attention) | M | agent |
| LIB-E3 | Online Catalogues as a navigation page with a Back button (modal stays today); see B | L | agent after decision |

### Reader
No open rows. (Remaining reader items are in Windows: WIN-R3, R5, R9.)

### Editor
No open rows. Not done by decision: access keys. Stale rows WIN-E3/E4/E6 are in Windows.

### Preferences and system
| Row | What | Size | Who |
|---|---|---|---|
| PRF-P4 | Settings search must cover the Footer Statistics description (`reader_footer.rs`) | S | agent |
| PRF-D7 | Footer Stats group rows (one control per row, info rows) in `reader_footer.rs` | S | agent |
| PRF-D17 | Audiobook player in `a11y_audit_windows` (bookmark delete undo done) | S | agent |
| PRF-D18 | First-run Welcome placeholder wording and buttons (`library.rs`) | S | agent |
| PRF-D19 | Other empty states: `vocab_list.rs`, `notes.rs`, `opds.rs`, `duplicates.rs`, `reader_side.rs` | M | agent |
| PRF-D23 | Section keys in Preferences: entry in the shortcuts dialog and a real-input session scenario | S | agent |

### Sync
| Row | What | Size | Who |
|---|---|---|---|
| SYN-S5 | Refresh `preferences-sync-*.png` (stale) in the capture run | S | agent |
| SYN-I2 | Uniform "open something" rows (`go-next-symbolic`, whole row activatable): plugins folder, patches folder, guide, Device window | S | agent |
| SYN-I4 | Only remaining part is judged not needed (confirmation for "Test on the device"); close the row | S | agent |

### Windows opened from the hamburger menus (5.7)
| Row | What | Size | Who |
|---|---|---|---|
| WIN-L1 | Open Library Folder chooser title and menu entry (`library.rs`) | S | agent |
| WIN-L4 | Statistics window: probably done by LIB-T1..T5 (verify, then close) | S | agent |
| WIN-L8 | Import Calibre Library must not open the Library Health window: chooser then Jobs-panel job like the export (partly done: Calibre group hidden until an import runs) | M | agent |
| WIN-L13 | Sync window: hand-built cards to boxed-list rows (D5); coordinate with the stepper (`stepper.rs`) | M | agent |
| WIN-L15a | Library Health = sidebar plus pages: done by LIB-L1 in wave 9; probably stale, verify and close | S | agent |
| WIN-L16 | Preferences dialog size caps at 900x600 (default 800x560): verify, close | S | agent |
| WIN-L18 | Copy Diagnostics moved out of the menu (LIB-M1); verify it lives in About | S | agent |
| WIN-L19 | About Bookx: `adw::AboutDialog` details, Copy Diagnostics, translator credits | S | agent |
| WIN-R3 | Go to dialog (`reader_goto.rs`): `AlertDialog` with live validation, probably fine; verify | S | agent |
| WIN-R5 | Terms in this Book: menu label in `reader_menu.rs` | S | agent |
| WIN-R9 | Reader Properties menu entry: needs approval (B) | S | owner |
| WIN-E3 | Checkpoints dialog: five controls per row, empty state, delete/revert confirmation or toast | M | agent |
| WIN-E4 | One Preferences for the editor (retire the `adw::PreferencesDialog`); `open_editor_prefs` still exists in `prefs_page.rs`; verify routing, remove wiki refs to `editor-preferences-*` | M | agent |
| WIN-E6 | Other editor menu items wording | S | agent |
| X6 | Dialog heights: Citations footer buttons off-screen at 600 px, others capped at 560 in wave 9 | S | agent |
| X7 | Stacked windows: Sync window to Preferences opens a second window; Vocabulary paste glossary | M | agent |
| X9 | Close affordance / Ctrl+W: remaining `adw::Dialog::new()` in `editor/*`, `reader*.rs`, `library*.rs` | S | agent |
| 5.7.5 verify list | Toast over an open dialog; Esc/Ctrl+W under real input; Import Highlights review dialog screenshot; statistics chart in dark mode; editor Book menu actions disabled when nothing to revert; large-library `al::find` | M | agent |

### Cross-cutting (§4)
| Row | What | Size | Who |
|---|---|---|---|
| XC-1 | Undo toast rollout is done per area; add the dialog-level toast helper (X5: Trackers, Audiobooks, Citations, Saved Searches errors hidden behind a modal dialog) | M | agent |
| XC-5 | Capitalization/wording pass in files other agents owned (`preferences.rs`, `mcp_settings.rs`, `dict_settings.rs`, `reading_ruler.rs`, `tts_settings.rs`, `library_banner.rs`); titles <= 48 characters, no "my" | M | agent |
| XC-6 | F9 sidebar (library, editor), Shift+F9 exists; Ctrl+, / Ctrl+? / F10 in the editor; Ctrl+A, Delete in the library; Search Tips row in shortcuts dialog | M | agent |
| XC-7 | Text scale 1.5x at 900x600 for library table, notes, Health, Devices, Preferences, editor header, reader (only the wave-9 dialogs are covered) | M | agent |

### Hamburger menus (5.6)
| Row | What | Size | Who |
|---|---|---|---|
| 5.6.1 "Access keys" | `_` mnemonics, unique per menu level, plus `po` msgids (decision in B) | M | agent after decision |
| 5.6 stale | 30 rows in 5.6.1/5.6.2 still say `open` although `MAIN_MENU` is 9 entries plus Help and the Library menu exists, `PopoverMenu` is used, names are "Main Menu" | S | agent (doc cleanup, I) |

## D. Roadmap and enhancement features not built

Source `roadmap-status.md` (2026-11-13) and `bookx-feature-roadmap.md`; status after waves 6-9 not re-audited, so some
"MISSING" below may now exist (grep before starting).

| Track | Item | What is missing | Size | Who |
|---|---|---|---|---|
| T1 text | R1 | colours, fonts, tables, positioning from book CSS | L | agent |
| T1 | R3, R23 | widows/orphans, small-caps synthesis; weight, contrast, word spacing, kerning | M | agent |
| T1 | R4/R56, PP-G-21 | tate-chu-yoko edge cases, selection in vertical text | L | agent |
| T1 | R46/R51 | max-width setting, fixed-layout spreads and positioned text | L | agent |
| T1 | R18 | EPUB 3 media overlays (SMIL) | L | agent |
| T1 | R65/R59, R49, R60 | page-turn animation, auto-turn timer, warmth/auto-night schedule | S-M each | agent |
| T1 | R25, R28, R29, R31, R33, R35, R43, R45, R57, R58, R61 | skim preview, TOC collapse, highlight adjust handles, editable highlight text, link hints, copy with citation, per-book overrides, auto archive, remote control | S each | agent |
| T1 | R34/R47, R36 | profile auto triggers, per-book theme and background texture; gesture manager | M | agent |
| T2 lookup | R38, R40, R41 | dictionary priority list UI, custom web lookup, lookup history; stats calendar export; whole word/regex search | S each | agent |
| T2 | R12/R27 | book map page thumbnails (strip of rendered pages), PDF book map | M | agent |
| T2 | R20/R32, F4 | Joplin/Obsidian templates, colour legend/filter | S | agent |
| T3 store/library | L3/L25 | custom columns (definitions, editing, display, `#col:` search, composite) | L | agent after decision |
| T3 | L14, L15, L16 | add by ISBN, empty record, archives with several books; filename guess in import UI; search-and-replace across fields | M | agent |
| T3 | L17-L22 | bookshelf/cover flow, tag browser hierarchy, quickview, author/series notes, marked books, full Calibre search syntax (dates, regex, parentheses) | M-L | agent |
| T3 | L5 | "send this shelf" to a device | S | agent |
| T3 | L12, L33 | background folder watcher (`notify`), full-text index status/rebuild UI | S | agent |
| T3 | L27 | jobs for device sends, manual backup window, add from the web, device cover sync | M | agent |
| T3 | L28, L13 | EPUB/AZW3 catalogue; crop UI and cover extraction from PDF | M | agent |
| T4 formats | C4/C7/C18 | PDF output, OCR, LRF; real RAR (CBR), DJVU, AZW4; Calibre DOCX TOC field, KF8 flows | L | agent |
| T4 | PP-G-24 | comics: next volume in folder, series from folder names, per-library direction, "ignore ComicInfo" | M | agent |
| T4 | PP-G-25 | one position across e-book and audiobook, Opus playback, position sync | L | agent |
| T5 convert | C10, C20 | conversion queue limiting parallel jobs; UI for per-format options beyond presets | M | agent |
| T6 serve | P10, PP-C-10, PP-G-26/27 | content server accounts and permissions per library, reverse-proxy prefix, PWA/offline web client | L | agent after decision |
| T6 | P7, P14, P17 | direct MTP, Kindle native reading state, FTP/SSH | L | agent |
| T6 | P19, P20 | Wallabag, QR code share | S-M | agent |
| T6 | networking-notes | "trusted proxy addresses" setting for the auth throttle; `OpdsConfig::prefix` really used | S | agent |
| T6 | P5/P13 | Calibre wireless protocol (deferred, D10) | L | owner decision |
| T7 editor | CE-357/401/402 | cursor width, preview colour/lines overrides, per-family fonts | S | agent |
| T7 | editor-preview-notes | click in preview selects the element; vertical-text markers | S | agent |
| T7 | editor-plugins-design | plugin dialogs/panels, store, signing, run on whole library | L | agent |
| T7 | CE-016/038/228/229/261/281/404 | NOT PLANNED by decision (docks, copy between instances, ORIGINAL_FMT compare, inspector, grammar, per-type toolbars) | - | no work |
| T8 sources | S4, S5, S6, S8 | recipe editor UI and accounts, per-feed full-article, per-field source priority UI, Wikipedia as a book | M | agent |
| T8 | metadata-notes | language-aware name order toggle; Check-before-send inside the Device window; editions as a level above books (work id); Hardcover API sync | M | agent |
| T9 app | X5, X10, X12 | phone-size adaptive reader/prefs; settings import/export; CLI gaps (catalog, fetch-news, polish, fonts, saved searches, custom columns) | M | agent |
| T9 | X18, performance.md | scroll fps/memory budgets, windowed layout (start in the middle of a chapter), first-chapter layout 0.6-0.9 s off the main thread, cold start at 100,000 books, Select All at 100,000 | M-L | agent |
| T9 | X19 | debug-log export, safe mode | S | agent |
| T9 | X20 | `--doc`, real launcher test for `bookx://`, Open With forwards to the running instance | S | agent |
| T10 plugins | X3 | plugin trust/signing, capability sheet at install (see F) | M | agent |
| X2 CLI | mcp-notes | `bookx-cli` not in the Flatpak; MCP `resources/subscribe`, `listChanged` not built | S | agent |
| enh 7 backup | backup-notes | per-library lock in the place, journal records for collections, chunked WebDAV upload, per-byte progress, GTK add/edit dialog scenario, passphrase re-encryption | M | agent |
| enh 4 | device sync plan | merge `devices::full_sync_plan` with `device_sync` plan; apply plan to the Kobo DB/KOReader (device writes) | L | agent |
| enh 9 TTS | tts-notes | MP3/OGG export, Kokoro Japanese/Chinese, Orpheus tier, `cpal` sink, speed above 2 in the engines | M | agent |
| enh 1/2 | pain points | 100k-book budgets are done; work entity above books, offline web client, multi-user | L | agent after decision |
| i18n X6/X17 | packaging-notes | about 700 English literals, native review of es/de/fr/hi, fr/hi only 191 starter strings, live re-translation, strftime dates, no RTL catalogue | M | agent, review by owner |

## E. Known bugs (`docs/bugs.md`)

All 24 rows are `fixed`. What remains are verification gaps and the open items found in QA notes:

| Id | What | Size | Who |
|---|---|---|---|
| bug 15 / 17 | Plugin never run inside KOReader; owner's panic text still unknown | M | owner |
| bug 12 (second) / 22 | Kobo colour numbers and Bookx-written KOReader notes layout unverified | S | owner |
| bug 12 (completion) | Vulkan popup race fix relies on a 90 ms delay; only checked on one machine, timing dependent | S | agent |
| bug 16 | First 1500-paragraph chapter layout (0.6-0.9 s) still on the main thread | M | agent |
| qa-wave5 "Open" | `Gtk-WARNING snapshot GtkScrolledWindow without allocation` in `library_hero_*` resize; library title/subtitle not re-translated at run time | S | agent |
| qa-wave5 | `calibre_export` scenario fails while a real Calibre runs (calibredb refuses a second client); needs a skip or an isolated profile | S | agent |
| comic-notes | CBR (RAR) is rejected with a message | L | agent |
| audiobook-notes | Opus not playable; up to 50 ms silence from AAC priming; first scan of a huge linked folder on the UI thread | M | agent |
| editor-preview-notes | `wiki/docs/editing.md` has some sections twice (earlier merge) | S | agent |

## F. Security roadmap (`docs/security.md` §7)

Done since the audit (the §4.2 table is stale): G1 `SECURITY.md`, G2 this model, G3 `cargo deny` CI job and Dependabot
(`.github/`), G12 checklist in `AGENTS.md`. Still open:

| Id | Item | Size | Who |
|---|---|---|---|
| P1.1 | `cargo fuzz` targets (zip container, OPF/XML, XHTML, CSS, PDF, KOReader sidecar, `Crypto::open_bytes`, `Manifest`) with seed corpus and a nightly job; no `fuzz/` directory exists | L | agent |
| P1.2 | `tests/evil/` corpus (zip bomb, 100k-entry zip, deep nesting, billion laughs, truncated, `..`, PDF link loop, 1 GB image) running under 30 s | M | agent |
| P1.3 | One shared `Limits` type threaded through `bookx-core`, `bookx-editing` and PDF (not only `bookx-formats`) | M | agent |
| G6 | `is_external` (`crates/bookx-core/src/util.rs:106`) still accepts `ftp://` and `data:`; narrow to http(s)/mailto | S | agent |
| G7 | `BOOKX_TTS_PLAYER` runs through `sh -c` (`crates/bookx-tts/src/sink.rs:129`); use argv | S | agent |
| G8 | SSRF guard in `Fetcher` for metadata, OPDS and news URLs (no private/loopback/link-local unless the user typed it) | M | agent |
| G9 | `Origin`/`Host` check on content-server mutations; trusted-proxy setting (networking-notes) | M | agent |
| G5 | `// SAFETY:` comment on every `unsafe` in `bookx` (4 found by grep; the doc says 20) and a review rule | S | agent |
| G10 | Plugin trust: signature or pin-by-hash, plain-language capability sheet at install, "unverified plugin" banner | M | agent |
| G11 | Sandbox for non-Flatpak installs (bwrap/Landlock wrapper or portal-only access) | L | agent |
| G13 | Sign releases (cosign/minisign), SBOM (`cargo cyclonedx`), verify Flatpak provenance | M | owner (key) + agent |
| P2.4 | Backup crypto review by a second reviewer (backup-notes: "needs a second reviewer"); Argon2id parameters, tamper tests done, key rotation not | M | owner |
| P2.5 | Dependency policy: maximum age, seven "unmaintained" notices (`chrono`, `ttf-parser`, `smartstring`, `zvariant`, ...) | S | agent |
| §6 | `docs/security-log.md` monthly log, `security` label, owner, metrics | S | owner |
| P3 | Quarterly threat-model review, incident runbook, annual external review | ongoing | owner |
| P4 | Servers authenticated by default, per-user scoping, TLS guidance, audit-log view for MCP writes in the UI | L | agent |
| bugs/security | `bookx-serve` MCP token confirmation (PRF-D14), port insensitive while running | S | agent |

## G. Test, CI and tooling gaps

| Id | Item | Size | Who |
|---|---|---|---|
| G.1 | **Vacuous check pattern**: `check "text $(cmd) ..." $?` passes whenever the substitution succeeds because `$?` is read after it. 63 call sites: `ui-tests.sh` (13), `ui-tests-ux-gaps.sh` (16), `ui-tests-comics.sh` (17), `ui-tests-reader8.sh` (15), `ui-tests-bigbook.sh` (2). Fix: capture `rc=$?` first, then `check "..." $rc`; add a lint in `ui-tests.sh` that greps for the pattern | S-M | agent |
| G.2 | No `actionlint` run (release-check prints "skipped"); workflows have never run on GitHub | S | owner (first run) + agent |
| G.3 | Real-session tests (`session-tests.sh`) never run in CI (need GNOME Shell) | M | owner |
| G.4 | Flaky: `tts_player_card_real` (3 sessions at once), `comic_progress` (load), session starts with 3+ at once (inotify limit), AT-SPI registry failure retry unproven | M | agent |
| G.5 | No scenario for the main editor window at 900x600 listed in HIG §1 (check `ui-tests-editor8.sh` before adding); no touch/long-press input scenario (hooks only); one monitor size and one theme in the session harness | M | agent |
| G.6 | Scenarios that need a real mouse drag (shelf drag and drop, real click on device row) call the handler directly | S | agent |
| G.7 | `toast with Open Folder button` (`dialog_toast::show_action`) has no scenario | S | agent |
| G.8 | Merge of formats (`book_formats::merge`) has no own unit test for undo; editor metadata tab provenance and bulk edit provenance have no UI scenario | S | agent |
| G.9 | No network fixtures for PDF reflow (real-world PDFs); `NET=0` is the only mode in CI so live source tests never run | M | agent |
| G.10 | Perf: budgets exist (`perf_budgets.rs` in 6 crates, 100k-book budgets); missing: scroll fps, memory ceiling beyond RSS, cold start at 100k, Select All at 100k, grouped editions and table model at 100k | M | agent |
| G.11 | Wave 9 gates: `.po` refresh needed for new msgids (i18n tests fail until `build-aux/i18n.sh` is rerun and de/es updated); `listening_time_goes_into_the_statistics…`, `library_nav::no_label_in_the_sources_ends_in_an_ellipsis`, clippy dead code in `annotation_sync*.rs` were other agents' unfinished gates in qa-wave5 (recheck with the full "before you finish" list) | S | agent |
| G.12 | `cargo-fuzz`/`evil` corpus gate for new parsers (F P1) | L | agent |
| G.13 | Real-device scenarios can not run in CI; keep `bookx-cli device-check` output as the attached evidence | - | owner |

## H. Release, packaging, docs and wiki gaps

| Id | Item | Size | Who |
|---|---|---|---|
| H.1 | GitHub: `ci.yml`, `release.yml`, `wiki.yml` never ran; image `flatpak-github-actions:gnome-50`, `cargo-deny-action@v2`, `action-gh-release@v2` inputs are from memory; Pages settings and DNS for `bookx.adhadse.com` | S | owner |
| H.2 | `.deb`/`.rpm`: `cargo deb` and `cargo generate-rpm` never run; hand-listed dependencies; no install test on Debian/Fedora; glibc floor | M | owner + agent |
| H.3 | Flatpak: Flathub submission, `x-checker-data`, GNOME 51, aarch64, AppImage (not provided), update notice, PNG icon sizes | M | owner decision |
| H.4 | Release 0.3.0: CHANGELOG `[Unreleased]` lists waves up to 5 only (nothing from waves 6-9: KOReader sync, Devices window, backup repository, comics, audiobooks, bookmarks, highlight repair) | S | agent |
| H.5 | Reproducible binaries (`--remap-path-prefix`), signing, SBOM (see F) | M | agent |
| H.6 | Translations: fr (787 lines) and hi (556) are starter sets (about 191 strings); es/de machine-assisted need native review; no Weblate project; translator-credits marker exists in `src/app.rs` but no credits provided; about 700 English literals | M | owner review + agent |
| H.7 | Wiki screenshots stale (listed in HIG §1): `library-main-menu-*` ("Share Library"; highest), `preferences-sync-*`, `library-device-*` name mismatch, `library-statistics/saved-searches/trackers/audiobooks/citations/catalogues/health/vocabulary-words`, `reader-menu-*`, `book-map-*`, `reader-goto-*`, `editor-book-menu-*`, `editor-checkpoints-*`, `editor-preferences-*`; no screenshots for Import/Export Calibre, About, Copy Diagnostics toast, Sync window, reader Properties, new windows from metadata-notes | M | agent (capture run) |
| H.8 | Mockups (`cargo run --example mockups`) not updated for new windows | S | agent |
| H.9 | Wiki statements to change after checks: `wiki/docs/accessibility.md:83` says vertical writing "is not supported yet" (it is built, see vertical-writing-plan) ; `devices-and-sharing.md` "Hardware tested so far" (only Kobo Libra Colour; keep until A is done); backup page "not yet against a real Nextcloud" | S | agent |
| H.10 | `wiki/docs/editing.md` duplicate sections (editor-preview-notes) | S | agent |
| H.11 | Wiki pages for waves 6-9 changes still to verify with `build-aux/wiki.sh` (Devices window rename, Library menu, Move to Trash, Preferences pages renamed, MCP page, selection mode) | M | agent |
| H.12 | `LIB-M14` wiki note on default sort Title; backup scope copy D8 on the wiki page | S | agent |

## I. Stale docs to clean

| File | Stale statement | Fix | Size |
|---|---|---|---|
| `human_interface_backlog.md` §5.6.1, 5.6.2, 5.6.4 | 30 rows `open` / `open (owner-decided)` for items done in waves 8-9 (MAIN_MENU 9 entries + Help; Library menu; PopoverMenu; "Main Menu" name) | mark done, keep mnemonics and WIN-R9 open | S |
| `human_interface_backlog.md` XC-1 | status says "reader part addressed", rest by area | collapse to done | S |
| `human_interface_backlog.md` WIN-L4, L15a, L16, L18, WIN-E4 | probably done by LIB-T*, LIB-L1, PRF-P5, LIB-M1, wave 9 editor | verify and close | S |
| `human_interface_backlog.md` §1 "nothing was built or run" | written before waves 7-9 | add a note | S |
| `roadmap-status.md` | audited 2026-11-13; counts, "Top 15", L5/L36/R44/P4c rows predate waves 6-9; R52 comics, audiobook, KOReader sync, backups, MCP changed | re-audit or add a banner | M |
| `enhancements.md` row 4 | "applying the plan to the device and the UI pending" but Devices window and auto-sync done in wave 7 | mark done | S |
| `enhancements.md` row 3 | "domain and Pages settings still to be done" | keep, owner item | S |
| `enhancements.md` row 7 | "partly done" while D9 incremental repository is done | update | S |
| `security.md` §4.2 | G1, G2, G3, G12 are done (SECURITY.md, deny CI, Dependabot, checklist) but listed as gaps | update table | S |
| `security.md` §4.1 | "20 unsafe uses" while grep finds 4 | recount | S |
| `bookx-feature-roadmap.md` Part 4 | "Not done: Calibre wireless", device writes, PDF OCR etc. now partly superseded | add a pointer to this file | S |
| `calibre-wireless-device-plan.md` | status "not implemented", D10 made wireless KOReader-only | note "deferred" | S |
| `device-ui-notes.md` caveat | notes `devices.rs` full_sync_plan merge as "one day" | keep in D | S |
| `wiki/docs/accessibility.md` | vertical writing "not supported yet" | fix | S |
| `pain-points-*.md` | counts audited 2026-11-20, before waves 7-9 | refresh | S |
| `README.md` / `_developer/docs.md` | fine; `_developer/docs.md` last paragraph (line 303) holds the highlight repair note, check it is also in "dev hooks" list | check | S |
| `docs/security-log.md` | referenced in security.md §6 but does not exist | create | S |
