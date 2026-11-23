# Diagnostic report: analysis, design and measurements

Status: living | Owner: @adhadse | Audience: maintainers (internal, not on the wiki; the user-facing text is
`wiki/docs/troubleshooting.md#what-is-in-the-report` and `wiki/docs/privacy.md#problem-reports`).

**Goal.** From the report alone, in the first ten minutes, a maintainer can tell *what went wrong*, *in which
feature*, *with which book, device or server*, and *in which state*, without asking for the replication steps again
(a re-run should be needed about one time in ten), and the report holds no personal data.

## 1. What the two real reports could not tell

| | #39 (Boox Leaf 2, "books do not sync") | #49 (Grimmory, KOSync) |
|---|---|---|
| What the maintainer needed | which device and plugin version connected; whether it ever synced; what the sync server does and does not move (positions and highlights, not books) | the server address *shape*, the matching method, the format and hash of the book, the HTTP status of login, pull and push |
| What the report showed | `Sync server: on`, 0 books, and 30 KB of one repeated desktop warning | the same portal warning, 60 KB of it, `(older lines left out to fit 60 KB)`, five start-up lines; the useful `kosync_pull`/`kosync_push` lines were on stderr only, so the user had to run a terminal command to find them |
| Could not see | the device model behind "Onyx Scubapilot-IDP" (a Boox Leaf 2), plugin and client version, last successful sync, the sync section at all | server kind and path (`/api/koreader`), `Document matching`, last login/pull/push with status, the book (pdf, size class, hash, in the library or not, local progress), Flatpak permissions, the library folder's file system |
| Noise | `zbus::proxy: Failed to populate properties cache` (one line per portal request), 40+ lines | the same, until the 60 KB cap pushed everything else out |

(#49 was fixed on main by #51: a 404 on push now reads "the server does not know this book (document id ...)" and the recorder classifies it as status 404.)

Lessons that shaped the design: (1) the *structured* facts must not live only on stderr; (2) the log tail must be
chosen by importance, not by age; (3) known harmless lines are counted, never listed; (4) a report must say what a
feature does *not* do, because a user's expectation can be the bug (#39); (5) the state of the sandbox, the disk and
the file system explains a third of all reports before any code is read.

## 2. Feature inventory and failure modes

Every user-facing area, the ways it breaks in the field (sources: `docs/bugs.md`, `docs/archive/`, the changelog
"Fixed" lines, issues #39 and #49, and incidents: a Fedora 45-only package on another distribution, the F-keys lost to
the beta notice's focus, highlights not painted after the notice resized the page, flaky local servers, database
lock errors, quota and full disks, Flatpak permission denials, GTK and libadwaita version differences), what the
maintainer needs to see, and where the report gets it. `●` captured now, `◐` partly, `○` not captured (see §8).

| Area (recorder `Area`) | Likely failures | What the maintainer needs | Captured by |
|---|---|---|---|
| Library, catalogue (`database`, `library`) | database locked by two writers; schema newer than supported; corrupt catalogue; library on NFS/SMB/FUSE or a synced folder; read-only folder; disk full; pre-update copy present | open result and class, schema, copies, integrity, file system kind, sync client, free space class, read-only | ● `database/open_library`, library folder section, log line |
| Import and add (`import`) | corrupt or unsupported file; DRM; huge file; Calibre `metadata.db` unreadable or locked; duplicates | format, size class, error class, source kind | ● `import/prepare`, `import/calibre`, `import/job` |
| Formats and conversion (`convert`) | KEPUB/MOBI/AZW3/FB2 conversion error; missing `unrar`, DjVuLibre, `ebook-convert`; DRM refusal; output too big | input format and size class, target, helper presence, class | ● `convert/epub_convert`, helper lines, jobs |
| Reader (`reader`) | layout slow or wrong; highlights not painted after a relayout; PDF without a text layer; comics and RTL/Hindi shaping; e-ink refresh | format, size class, chapters, layout time, highlights stored vs shown, language and script, renderer | ● `open_book`, `load_chapter` ms, `paint_highlights`, `pdf_relayout`, `comic_relayout`; ◐ visual defects |
| Editor (`editor`) | save fails; validation false positives; plugin action crashes | format, class, plugin id and version | ◐ plugin load; ○ editor operations (not hooked) |
| Search (`search`) | index empty or stale; query slow; PDF scan | indexed n of m, class | ● `search/fulltext`, `extract_text`, library section |
| Read aloud and audiobooks (`tts`, `audiobook`) | voice missing; model download fails or its hash does not match; no audio player; no espeak-ng; engine server unreachable | engine, voice language, helper presence, sockets, class | ● `tts/open_engine`, `download_packs`, helper lines; ◐ playback errors |
| Devices (`devices`) | MTP not mounted; USB path; Kobo database missing or new schema; copy to a full device; format not accepted | model (resolved name or code), backend, class, free space class | ● `devices/copy`, Devices section when opened from the window |
| Sync, client (`sync`) | 401, 404, TLS, DNS, timeout; wrong path; matching method differs; held back; queue | server address shape, matching, login state, last op, status, book format/size/hash/progress | ● `sync/login`, `pull`, `push`, `queue`, sync section, books involved |
| Built-in server (`server`) | port in use; HTTPS certificate; device never synced; annotations not linked; reverse proxy | port, switches, devices seen (model, client, plugin), last sync class, rejected statuses | ● `server/start`, `annotation_merge`, server section |
| OPDS and web import (`opds`) | 401, TLS, LAN timeout, bad feed | host kind, scheme, status, class | ● `opds/request` (all fetchers wrapped) |
| Backups (`backup`) | WebDAV 401/TLS/timeout; disk full; place not mounted; restore fails | place kind and address shape, class, duration, free space | ● `backup/job`, backups section |
| News, metadata, dictionaries (`news`, `metadata`) | rate limits; flaky downloads; offline mode | status, class, kind of job | ● `metadata/request`, `*/job` |
| AI assistants (`mcp`) | port in use; token; write access | transport, port, class | ● `mcp/start` |
| Plugins and patches (`plugins`) | load failure; ABI; trap | id, version, error class | ● `plugins/load`, plugin section |
| Settings, keyring (`settings`, `secrets`) | file cannot be saved; keyring locked or missing; master password | class, mode | ● `settings/save`, `secrets/unlock`, keyring line |
| Network (`network`) | offline mode; proxy; Flatpak without network; DNS | offline, proxy set (yes/no), sandbox permission | ● network section, sandbox section |
| Display and interface (`ui`) | main-thread stalls; banner takes focus; startup slow with a huge library; scale/renderer differences | stall length, area and last operation, start-up ms, backend, renderer, scale | ● `ui/stall`, `ui/notice_shown`, machine section |
| Packaging | wrong distro build (glibc, libcrypto); Flatpak permissions; version differences | install kind, runtime, permissions, GTK/libadwaita versions | ● environment, sandbox; ○ pre-start failures (§8) |
| Crash | panic | message, location, backtrace, last operations | ● crash section + trace tail persisted at exit and in the panic hook |
| Accessibility, translations | missing names, untranslated strings | locale, script, direction | ◐ language and script only |

## 3. The mechanism

`src/trace.rs` is the flight recorder: `trace::op(area, name)` returns a guard that records duration and outcome
(`ok`, or an error *class* plus a cleaned message), with bounded fields (a count, a size class, a format, a hash, a
fixed word: never free text, so a field cannot carry a title or a path). `trace::event(area, name)` records a moment.
`trace::traced(area, name, |o| ...)` and `trace::timed` wrap a whole function in one line. Entries are kept per
area (30 each, 400 in all; the same thing again right after itself is one entry with `[×N]`), failures, slow
operations and events are also written to the log as a breadcrumb (`trace sync push fail http=404 ...`, which
`Entry::from_line` reads back), and the newest of every area is written next to the log when the app closes or
panics (`trace-last-session.txt`, 0600), so a report made after a restart still has the last session's tail.

State comes from `diag_gather` (one function per feature returning a `Section` of `Fact`s) and from providers
registered with `trace::register` (`fn() -> Vec<(label, Fact)>`). `diag_system` reads the machine: Flatpak sandbox,
memory and CPU as classes, the library folder's file system, free space, sync client and catalogue.
`diag_problems` groups the warnings and errors, counts the noise listed in `NOISE`, and chooses the log lines that
fit the 60 KB by importance. `diag_stall` and `ui/stall_watch` measure main-loop stalls (a 100 ms timer, a stall is
250 ms late or more, a gap over a minute is a suspended computer). `diag_fetch` wraps the `Fetcher` of the metadata,
OPDS and web-import features so every request is recorded without touching the feature.

Hooks (each one to a few lines): `jobs::finish` (every job: import, export, send, fetch, backup, news, voice and
dictionary download), `import::prepare`, `export::convert_epub_bytes`, `devices::send_full`, `fulltext::search` and
`extract_text`, `metadata_fetch::fetch_with`, `tts::open_engine` and `download_packs`, `mcp_host::start`, the KOSync
login, pull, push and queue (`ui/kosync.rs`, through `kosync_sync::trace_op`), the built-in server start and stop and
the annotation merge, the library open, plugin loads, settings save, the keyring unlock, the reader's open, chapter
layout, PDF and comic relayout and highlight painting, the notice banner, and the three fetchers.

### Noise filters (`diag_problems::NOISE`)

| Filter | Matches | Why it is harmless |
|---|---|---|
| desktop portal property cache | `Failed to populate properties cache` | zbus warns when a short-lived portal request ends before its properties are read; seen in every report from GNOME |
| desktop portal request proxy | `org.freedesktop.portal.Request` and `proxy` | the same notice in another wording |
| portal property change streams | `Property change streams will not produce values` | the second half of the same warning |

Add a filter only for a line that is known to say nothing about a bug; it is counted in the report ("left out as
harmless"), never silently dropped.

## 4. The summary and the rule table

`build-aux/triage-rules.json` holds the rules (regular expressions over the text of the report, a cause, a hint,
the evidence lines to quote, five questions). `src/triage.rs` reads it for the report's *Summary and suspected
area*; `build-aux/triage-report.py` reads the same file for a report pasted in an issue (area, severity, cause,
key facts, five questions; `--json` for scripts). One table, two readers; a Rust test runs the Python tool on every
replay report and asserts the same headline. The rules read the *rendered* report (without the user's own words), so
a finding can never rest on something the maintainer cannot see. Severity is `crash`, `data risk` or `annoyance`.
Events (an operation failed) come before states (the library is on a network disk). The file is generated from the
rules written in plain data; edit it directly, keep the patterns to the common subset of Rust `regex` and Python `re`
(no look-around), and a test checks every rule has a cause, a hint, evidence and at least three questions.

## 5. Redaction

Rules (every one is a test in `diag_addr`, `diag_facts`, `diag_pii`):

| Data | Rule |
|---|---|
| Server address | `Fact::address`: scheme, class of host (`<ip:private>`, `<ip:public>`, `<ip6:...>`, `<hostname>`, `<hostname:local>`, `<domain>`, `localhost`), port, and path words from a known list (`api`, `koreader`, `kosync`, `opds`, `dav`, ...); every other path segment is `<segment>`; user name and password become `<credentials>@`; the query is `?<query>`; the host itself only when the user switched it on |
| Free text (errors, log) | addresses are reduced to their shape first, then the scrubber (library folder and its name, titles, authors, series, servers, accounts, secrets, home, user name, host name, e-mail, IP, MAC, tokens), then paths that are left (`<path>`, `<file>.ext`), file names with a book extension, and serial-looking ids (`<id>`) |
| Paths | home becomes `~`; other paths that are not the program's or the system's become `<path>`; names with a book or document extension become `<file>.ext` |
| Device names | `trace::device_label`: the product name when the model is in the table, the model code when it looks like one, else `<custom name>`; names the user gave a device are also scrubber accounts |
| Books | format, size class, progress, library id; title and authors only through `Fact::title` and only when allowed, only for books an operation named |
| Counts and sizes | classes for memory, free space, file sizes, request counts; exact counts only for the library's number of books |
| Hashes | allowed (hex, 8 to 64 digits): a document id identifies bytes, not a person |
| Sandbox | `/.flatpak-info` read as words: runtime, network, sockets, devices, features; file-system permissions as `home`, `xdg-download`, `<path>` |
| Never | secrets, tokens, folders, file names, search words, text of books and notes, Wi-Fi names (not read at all) |

**The single door.** A `Fact` has a private kind and no constructor from a `String`: a feature chooses `word`
(a literal), `count`, `flag`, `size`, `hash`, `version`, `lang`, `choice` (a value from outside, shown only when it is
in a list the caller gives), `text` (cleaned), `address` or `title`. The report builder renders a `Section` only with
`Redactor::section`. `diag_pii` checks `report.rs` does not read section rows directly and that `diag_gather` and
`diag_system` build no text by hand.

**The corpus** (`src/diag_pii.rs`): a fake user whose every personal value is a sentinel, put into every source: the
settings (sync server with credentials in the URL and a token in the query, share and Readwise passwords, OPDS and
backup places with names and paths), the library (titles, authors, file names, the library folder), the log (an error
that embeds a path, a panic message, a URL with credentials, an e-mail with IP and MAC addresses, a device copy with a
serial, a search with a query), the trace (messages with addresses, paths and device names), the devices the server
knows (a custom name and a name with the owner's), a plugin id that is an e-mail address, the Flatpak info (folders),
the environment (proxy with credentials, user, host name, Wi-Fi name), the crash text. Asserted: no sentinel in any
combination of switches; titles only with the switch, only for the books involved, only on the title lines; the host
only with the switch and only on address lines; user credentials, the query and the token never; realistic library
error strings (`cannot open /home/alice/Books/Foo/Foo.epub`, `talking to http://10.0.0.7:30260/api/koreader`,
`no such file: 'Secret Title.pdf'`) lose their paths, addresses and names; 3000 generated texts with the shapes of
personal data (paths, addresses, credentials, e-mail, volume names) are cleaned *without the values being known*.

## 6. Replay coverage (the measure)

Each case is a failure the app has had or plausibly will, written as the state, trace and log the app would hold,
built through the same functions as the app (`diag_gather::input`, `report::build`). The test asserts the summary
names the area and the report holds the facts the maintainer needs (patterns per case in `src/diag_replay.rs`).

| Case | Identified from the report alone | Area named | Why |
|---|---|---|---|
| `kosync-push-404-pdf-grimmory` | yes | Sync (KOSync client) | issue #49: the address shape, the matching method, the book's format and hash, and the HTTP status are all in the summary and the sync section |
| `boox-leaf2-expects-books-to-sync` | yes | Built-in server | issue #39: nothing failed; the report names the device as a Boox Leaf 2, shows the plugin and that it never completed a sync, and says what the server does and does not move |
| `package-built-for-another-release` | no | (no report possible) | the program does not start (glibc symbol error): the app cannot build a report; build-aux/triage-report.py recognises the pasted terminal output instead |
| `beta-notice-takes-focus` | partly | Display and interface | the report shows that a notice banner appeared and what the reader did right after; whether a key reached the page stays a question |
| `highlight-not-painted-after-relayout` | yes | Reader | the reader records how many highlights are stored and how many belong to the chapter shown, next to the relayout and the banner |
| `database-locked-two-writers` | yes | Library database | the open fails with class locked and the message names a second Bookx; the sync client and the file system are listed |
| `catalogue-schema-newer-than-supported` | yes | Library database | the error line, the supported schema and the copies from before an update are in the report |
| `webdav-backup-401` | yes | Backups | the backup job fails with class unauthorized and the place is told by kind, encryption and address shape |
| `webdav-backup-tls-error` | yes | Backups | class tls on the backup job, the scheme and the kind of host |
| `webdav-backup-timeout` | yes | Backups | class timeout with the duration of the job |
| `mtp-boox-not-mounted` | yes | Devices | the copy fails with class not_mounted, the device model, the backend and the sandbox device permissions |
| `kobo-database-missing` | yes | Devices | the device name is told as a model, the message says the Kobo has no library database |
| `kepub-conversion-fails` | yes | Conversion | the conversion step, its target, the input size class and the error class |
| `cbr-without-unrar` | yes | Conversion | the import fails for a cbr, and the helper line says neither unrar nor 7-Zip is there |
| `djvu-without-djvulibre` | yes | Conversion | the import fails for a djvu and the DjVuLibre line says it is missing |
| `pdf-without-text-layer-search` | partly | Search | search and read aloud on a scan fail with a generic error; the report shows the format and the failing step, but not that the PDF is a scan |
| `tts-voice-missing` | yes | Read aloud | open_engine fails with class missing, the engine and the helper lines |
| `tts-no-audio-player` | yes | Read aloud | the helper line says no audio player is installed and the sandbox sockets are listed |
| `tts-model-hash-mismatch` | yes | Read aloud | the download fails with class hash_mismatch and the job duration |
| `plugin-fails-to-load` | yes | Plugins and patches | the plugin id and version, the load failure and the warning in the log |
| `mcp-port-in-use` | yes | AI assistants (MCP) | the start fails with class port_in_use |
| `flatpak-without-network` | yes | Packaging | the sandbox section says the network is not shared, next to the failing sync call |
| `library-on-nfs` | yes | Library folder | the file system type and its kind are told, with the other folder facts |
| `library-on-fuse-rclone` | yes | Library folder | the FUSE file system and its kind |
| `disk-full-during-backup` | yes | Storage | the backup fails with class disk_full and the free space class of the library disk |
| `keyring-locked` | yes | Passwords and keyring | the settings line says passwords are in a locked store |
| `keyring-missing` | yes | Passwords and keyring | the settings line says passwords are not saved, with the unlock attempt |
| `hindi-layout-complaint` | partly | Reader | the script, the languages, the fonts and the renderer narrow it, but a wrong glyph or direction leaves no error to find |
| `huge-library-startup-stall` | yes | Display and interface | the start-up time, the stall events with the area and last operation, the library size and its disk |
| `dictionary-download-flaky` | yes | Metadata and lookup | the job fails with class timeout and says it was a dictionary download |
| `opds-server-401` | yes | OPDS and web import | the request fails with 401, the kind of host and the shape of the catalogue address |
| `calibre-import-unreadable-database` | yes | Import and add books | the import fails with its class and a message that says the file is not a database |
| `app-crashes-while-opening-a-pdf` | yes | Crash | the crash text with the message and the first frames, and the last operations before it (the book's format and size) |
| `server-port-busy` | yes | Built-in server | the start fails with class port_in_use and the port and sharing switches are listed |
| `news-feed-404` | yes | News and read later | the news job fails with the HTTP status of the feed |
| `device-copy-disk-full` | yes | Storage | the copy fails with class disk_full for the named device model and format |
| `full-text-index-empty` | yes | Search | the library section says how many books are indexed, with the disk and file system of the folder |
| `library-folder-read-only` | yes | Library folder | the folder section says it is read-only |
| `library-inside-syncthing` | yes | Library folder | the folder section names the sync client |
| `annotations-from-device-not-linked` | yes | Built-in server | the merge event counts the annotations that found no book, with the device model and the sync switches |
| `opds-lan-server-timeout` | yes | OPDS and web import | the request times out; the host is told as a private address and the scheme is plain http |
| `import-of-a-damaged-epub` | yes | Import and add books | the import fails with class corrupt, the format and the size class |
| `metadata-service-rate-limited` | yes | Metadata and lookup | the request fails with HTTP 429 and the kind of host |
| `settings-file-cannot-be-saved` | yes | Settings | the save fails with its class (read-only or full disk) and the folder facts |

40 of 44 cases are "yes" (90 percent).


How to read it. The cases were written by the people who wrote the recorder, from failures they knew, so this is a
**lower bound on how well the recorder fits its own failures, not a measurement of failures nobody has met**. The
honest gaps: a package that does not start (no report can be made; `triage-report.py` recognises the pasted
terminal output), and three that leave no error: the banner taking focus, a Hindi glyph or direction problem, and a
scanned PDF (the report narrows each; a question remains).

## 7. How to add a fact safely

1. For the state of a feature, add one function to `diag_gather` returning a `Section` of `Fact`s and call it from
   `sections` (or `trace::register` a provider). Pick the narrowest `Fact`: a literal, a count or a class before
   `choice`, `choice` before `text`.
2. For something that happens, add `trace::op(Area::X, "name")` (or wrap a function with `traced`), set the fields
   with the typed helpers (`fmt`, `size_class`, `word`, `hash`, `book`, `device`), and finish it with `ok()` or
   `fail(&message)`. Never put a title, a path or an address into a field.
3. If the new fact decides a diagnosis, add a rule to `build-aux/triage-rules.json` and a case to
   `src/diag_replay.rs`; update the coverage table here (a test fails until the row exists).
4. Add the new value to the sentinel fixture in `diag_pii` if it comes from a new source.

## 8. Decisions and what is not included

| Item | Decision | Why |
|---|---|---|
| Server address | include, as a shape | scheme, port and path decide most sync bugs; the host does not |
| Host of an address | opt-in, default off | can identify a person's server |
| Titles and authors | opt-in, default off, only the books involved | needed sometimes, never by default |
| Document hash | include | identifies bytes, not a person |
| Device model | include (resolved name or code), never the custom name | the model is what the maintainer needs |
| Device serial, MAC, volume label | never | personal |
| Wi-Fi name | never read | not needed |
| Exact RAM, disk size, free space | classes | reduce fingerprinting |
| Timezone | offset only | coarse |
| GPU name | not included | needs a GL query at the wrong time; the renderer name (ngl, vulkan, cairo) and backend are included |
| Portals available | not included | a portal call per report; the noise lines show portals answer |
| Environment variables | only "proxy set" and the renderer override | values can hold credentials |
| Catalogue integrity | include for files under 200 MB (read-only quick check); larger skipped | cost |
| Settings values | switches, counts and classes only | values are typed by the user |
| Editor operations, comic and audiobook playback errors | not hooked yet | no failure data from the field yet; add as they are reported |
| A name the user typed in free text | not caught, by design | the user's own words are shared as typed, and the dialog says so |

**What cannot be caught.** A personal word in a message with no recognisable shape that is not a value the settings
or the library know (for example an unknown Wi-Fi name inside an error text). No source the report reads puts such a
word in a message, the corpus asserts it for every source it knows, and the preview shows the whole report before it
is shared. A user-chosen device name that looks like a model code (`leaf2`) is shown as one.

## 9. GitHub labeller idea (not added; ci.yml is owned elsewhere)

A workflow on `issues: opened` could run `build-aux/triage-report.py --json` on the issue body and apply labels from
the headline: `area:<slug>` from `area`, `severity:crash|data-risk|annoyance`, and `needs-info` when `findings` is
empty or the body has no `### Summary and suspected area`; for `needs-info` it could post the five questions from
`generic_asks` (or the rule's `asks`) as a comment. It needs no secrets beyond the default token, and runs the script
from the base branch so a pull request cannot change what it does to issues.

## 10. Not verified

No real device, KOReader or sync server was used: the replay cases are simulated. The Fedora-45 package problem is
tested only through the recognition of its terminal output. The stall detector and the reader hooks ran in the
Broadway UI tests, not on a slow disk or a very large library. Translations (de, es, fr, hi) are written, not
reviewed by a native speaker.
