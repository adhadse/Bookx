# Bookx developer notes

## Toolchain and dependencies

Everything in the application is Rust. System libraries are only the GTK stack that Relm4 binds to.

| Crate | Why |
|---|---|
| `relm4` 0.11 (`libadwaita`, `gnome_49`) | UI architecture; re-exports `gtk4` 0.11 and `libadwaita` 0.9 |
| `pangocairo` | drawing Pango layouts in the reader canvas and editor gutter |
| `zip` | EPUB containers |
| `roxmltree` | read-only XML (OPF, NCX, nav); gives byte ranges so edits can be text splices |
| `scraper` (html5ever) | tolerant XHTML/HTML parsing for chapters |
| `pulldown-cmark` | Markdown → HTML on import |
| `regex` | search language helpers and editor find & replace |
| `serde`, `serde_json` | library catalogue and settings |
| `chrono`, `uuid`, `directories`, `anyhow`, `tracing` | housekeeping |
| `resvg` (dev) | renders the mockups |

Minimum system versions: GTK 4.20, libadwaita 1.8 (needed for `adw::ShortcutsDialog`, `adw::WrapBox`,
`adw::Dialog` / `adw::AlertDialog`).

## Architecture

### `core/` — no GTK

* **`Archive`** loads the whole EPUB in memory (`name → bytes`) and writes it back with `mimetype` first and
  uncompressed. All editing happens on this structure.
* **`opf::Package`** parses the package document. Edits (metadata, cover, manifest items, spine) are *text
  splices*: the original XML is kept and only the affected range is replaced, so unknown elements, comments and
  formatting survive. Every splice is re-parsed before it is stored.
* **`toc`** reads EPUB 3 `nav` and EPUB 2 NCX and rewrites both.
* **`html::parse_html`** turns a chapter into a `doc::Doc` (blocks of styled spans, anchors, images). The reader,
  the editor preview, search and the TXT/Markdown/HTML exporters all work on this model.
* **`library::Library`** is a Calibre-like folder tree plus a catalogue behind the `CatalogBackend` trait. `bookx-store` implements it with SQLite (`bookx-library.db`, WAL, incremental saves: only changed books are written; `bookx_store::open_library` also migrates an old `bookx-library.json`). `bookx-cli library dump` prints the catalogue as JSON. `prepare_import` is thread-safe
  (parse + convert), `commit_import` registers the book on the main thread.
* **`query`** implements the search language; **`check`** the book checker; **`convert`** the format conversions.

### `ui/`

* One `Rc<Shared { lib, cfg }>` is shared by all components (GTK is single threaded).
* `LibraryView`, `Reader`, `Editor` are Relm4 `SimpleComponent`s. They build their widgets imperatively in `init`
  and keep widget handles in the model; `update` mutates them directly.
* `App` owns an `adw::NavigationView`: the library page is permanent, the reader page is pushed on demand,
  editors are separate `adw::Window`s.
* **`canvas::BookCanvas`** is the text engine. It lays a `Doc` out into one *flow* of lines using Pango, then
  either shows the flow scrolled, or cuts it at line boundaries into pages drawn as one or two columns.
  Selection, highlights, search hits and links are addressed by *global character offsets* in the chapter, which
  is what the catalogue stores for highlights.
* `syntax.rs` applies `GtkTextTag`s for the code editor.

## Developer tools

Environment variables (all optional):

| Variable | Effect |
|---|---|
| `BOOKX_LIBRARY=/path` | use this library folder instead of the configured one |
| `BOOKX_SIZE=1440x900` | initial window size |
| `BOOKX_DEV_SCRIPT=…` | comma-separated script run after start-up |
| `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` | isolate settings / icon cache |

Script commands: `wait:<ms>`, `shot:<png>[@window]`, `read:<id>`, `edit:<id>`, `close:`, `prefs:`, `quit`,
`library:<table|grid|select1|selectall|editmeta|import=<dir>|search=<q>>`,
Library Health: `library:health=<open|dump|backup|check|drift-on|restore1|fix1|undo|trash-restore1|trash-empty|tidy|inbox-scan|inbox-add-all|inbox-add1|inbox-dismiss1|auto-off|keep=N|inbox-folder=DIR|calibre=DIR[|nocopy]|layout=NAME[|TEMPLATE]>` (buttons are pressed for real; a disabled button prints `health: button is disabled`; `dump` prints `health_*` lines); backup destinations: `library:health=<backup-add=NAME|PATH[|encrypt][|keep=N]|backup-add-dav=NAME|URL|USER|PASSWORD|FOLDER[|encrypt]|backup-passphrase=TEXT|backup-remove1|backup-reload|backup-pick=N|backup-restore-remote1|backup-cancel|backup-dump>` (`backup-dump` prints `backup_*` lines; `backup:` lines on stderr say what was copied, verified or failed; see `docs/backup-notes.md`); `BOOKX_SECRETS=memory|none` keeps passwords in memory only / pretends there is no keyring (the UI tests set `memory`); `BOOKX_READ_ONLY=1` opens the library read-only,
Metadata review and library order (`docs/metadata-notes.md`): `library:bulkfetch` (starts *Fetch Metadata…* for the selection), `library:bulkfetch=<dump|accept-all|accept-none|accept-safe|accept-field=F|toggle=N|check=N|apply|undo|auto-on|auto-off|close>` (`dump` prints `bulkfetch:`, `bulkfetch_health:` and one `bulkrow:` per row), `library:preflight` / `library:preflight=<dump|fix|continue|dest=kindle|kobo|email|any>`, `library:organize=<series|works|attention|series-dump|works-dump|attention-dump|idclean>`, `library:seriestext=<text>` / `library:seriesdump` (series number field of the open edit dialog), `library:undo=N` (take back N operation-log entries). `$BOOKX_SOURCES_MOCK/<openlibrary|google>.status` containing a number makes that source answer with it (outages, rate limits). The search term `weak:` finds books with poor metadata.

Formats, authors, trackers, audiobooks, DRM, import lookup (`docs/metadata-notes.md`, second round): `library:formats=<dump|rows=ID|drm|retype=ID:EXT|merge-preview=ID,ID|merge=ID,ID|add=ID:/file|remove=ID:FORMAT|offers=add|separate|skip>` (`retype` renames a book's file, the way another tool would have filed it as AZW3; `offers=` answers the "add as another format" question of the next import without a dialog), `library:authors=<dump|open|merge=A|B>>Canonical|alias=A>>Canonical|rename-preview=A|B>>Name>`, `library:trackers=<open|plan=/file.csv|import=/file.csv|export=goodreads|storygraph:/file.csv>`, `library:audiobooks=<open|open=/dir|find=/dir|link=/dir|dump>`, `library:bulkedit=tag:+x|tag:-y|author:A|series:S|publisher:P|language:l|rating:N` (the bulk editor on the selection, same code as the dialog), `editor:meta-apply=publisher:P|title:T|series:S` (the editor's Metadata tab apply path), `library:bulkfetch=cache` (cache hits and misses). `$BOOKX_SOURCES_CACHE=1` switches the response cache on while `$BOOKX_SOURCES_MOCK` is set (the tests normally skip it); `{"metadata_prefs":{"fetch_on_import":true}}` in the config file turns on the lookup after an import.
News: `library:newsadd=<title>|<url>|<manual|hourly|daily 07:00|weekly mon 07:00>|<keep>`, `newsopml=<title>|<file>`, `newsfeedoff=<key>|<url>`, `newsrun=all`, `newstick` (run what is due; move the clock with `stats:skew:<s>`), `newsshow`, `newsdialog`, `newsdump`.
`reader:<goto=N|next|chapter|toc|notes|search|select|hl|note|bookmark|light|sepia|dark|single|two|scroll>`,
Reader tests (`reader:` too): `dump` (prints `breaks:`, `wrap:`, `rtl:`, `fixed:`, `hyphens:` and the rest), `ui-dblclick` (a double click on the first word as the mouse makes it), `ui-dict-btn`, `ui-wiki-btn` (press the real buttons), `ui-optimal-on|off`, `ui-bench`, `ui-selectall`, `ui-vertical` (prints `vertical:` with the writing mode, pages, ruby and tate-chu-yoko counts, kinsoku violations, a hit-test check and the size of the page, then `vlines:` the text of every column of the page and `vprobe:` where characters 0, 5 and 12 are drawn), `ui-vsel:A:B` (a drag from the middle of character A to the middle of character B, prints the selection, its text and its screen rectangles), `ui-shot:FILE` (paints the page to a PNG: look at layouts without a screen), `ui-tweak:KEY=VALUE` (KEY: `scope=all|book`, `line_height`, `margin`, `spacing`, `font`, `justify=on|off`, `optimal=on|off`, `hyphen=off|de|…`, `css=…`, `reset=1`; drives the real Style Tweaks dialog), `ui-tweaks-state`, `tts-play|pause|stop|voices`, `tts-speed:X`, `tts-pitch:X`, `tts-voice:NAME`, `tts-skip:N` (sentences, negative = back), `tts-sleep:chapter|MINUTES`, `tts-dump`. `BOOKX_TTS_MOCK=<ms>` replaces the voice by a silent speaker that finishes a sentence after that many milliseconds (it queues two sentences ahead, like a real engine). `BOOKX_TTS_DIR` is where models are looked for (tests point it at an empty folder), `BOOKX_TTS_PLAYER` a shell command that receives the raw s16le samples on stdin instead of pacat/pw-cat/aplay (`cat >/dev/null` in tests; `{rate}` is replaced), `BOOKX_TTS_THREADS` the ONNX threads, `BOOKX_ESPEAK` the espeak-ng program, `BOOKX_TTS_API_KEY` the bearer key for voice servers. Settings page hooks (`tts:` commands, Preferences open): `tts:engine=ID`, `tts:server=URL`, `tts:test`, `tts:dump`. `build-aux/tts-fake-server.py` is an OpenAI-compatible fake server for scenario `tts_server`; `build-aux/tts-catalog.py` regenerates the pinned download list; `cargo run --release -p bookx-tts --example speak` makes a WAV with a downloaded model and prints the real-time factor; `BOOKX_TTS_LIVE=1 BOOKX_TTS_DIR=… [BOOKX_TTS_LIVE_PACK=kokoro-int8] cargo test --release -p bookx-tts live_download -- --nocapture` downloads a real pack (opt-in).
`editor:<open=<path>|find|check|toc|meta|gentoc|first-image|save>`, and for the editor scenarios `goto=N`, `mark`, `cursor=N`, `select=a-b`, `find-in-text=TEXT[|sel|end]`, `insert=TEXT`, `tag=ACTION[|ARG]`, `has=TEXT`, `cur`, `buf`, `cat=PATH`, `cps`, `tabs`, `tab-close`, `step=N`, `quickopen=Q`, `select-files=a|b`, `merge-selected`, `bulk=PREFIX|N`, `newfile=NAME|TEMPLATE[|link]`, `linkcss[=remove]`, `export=DIR`, `delete=a|b`, `move=up|down`, `addpaths=P`, `title=T`, `tocgen=SOURCE|LEVEL|MODE[|PAGE]`, `toc-go=N`, `toc-dump`, and for the GtkSourceView code view `view` (language, scheme, options), `scheme=dark|light|eink`, `view-opt=minimap|whitespace:on|off`, `live` (run the debounced pass now), `marks`, `tip=LINE`, `spots`, `spell-menu=WORD`, `spell-act=replace|WORD|WITH` / `ignore|WORD` / `add|WORD`, `pair`, `hidden`, `find-set=Q`, `find-opt=regex|noregex|case|dotall`, `find-status`, `complete=TEXT-BEFORE-CARET` (⏎ for a line break), `complete-show` + `complete-last` (open the real popup), `snippet=TRIGGER`, `snippet-add=NAME|TRIGGER|any/markup/css|BODY`, `snippets`, `snippets-dialog`, `close-results` (commas cannot appear in arguments; `\n` in `insert=` is a line break). The vertical-writing test books are made by `build-aux/make-vertical-epubs.py` (`synthetic`, `synthetic-long`, `aozora SRC.html OUT.epub`; see `docs/test-books.md`). `SHOW=scenario ONLY=scenario build-aux/ui-tests.sh` also prints everything the app said in that scenario. Scenarios named `editor_live_*` get a tiny Hunspell dictionary through `BOOKX_HUNSPELL`. `BROADWAY_BASE=N` moves the Broadway display numbers of `build-aux/ui-tests.sh` (use it when several test runs share a machine), `CARGO_TARGET_DIR` is honoured.
Editor preview sync, splitting, compare view, checkpoints dialog and Live CSS: `editor:pv-*`, `editor:cmp-*`, `editor:cp-*`, `editor:css-*` (full list in `docs/editor-preview-notes.md`).

UX gaps (`docs/ux-gaps-notes.md`): `keys:<open|dump=ACTION|press=ACTION|ACCEL|settle=swap|take|cancel|clear=ACTION|reset=ACTION|resetall|filter=TEXT|export=FILE|import=FILE|local|dialog=ACTION>` (the Keyboard Shortcuts page; `press` runs what the capture dialog does after a key press, a conflict waits for `settle`), `library:shelf=<create=NAME|rename=OLD|NEW|delete=NAME|add=NAME|ID+ID|addsel=NAME|remove=NAME|ID+ID|drop=NAME|ID+ID|up=NAME|down=NAME|bookup=NAME|ID|bookdown=NAME|ID|open=NAME|menu|dump>` (`drop` runs the drop handler of a shelf row), `library:libs=<add=NAME|PATH|folder=PATH|switch=NAME|rename=OLD|NEW|remove=NAME|new=NAME|PARENT|dump>`, `jobs:<start=KIND|TITLE|cancel|progress=ID|FRACTION|DETAIL|log=ID|TEXT|finish=ID|ok|REASON|cancel=ID|clear|open|close|rows|dump>`, and for the reader `reader:<goto-text=TEXT|back|forward|goto-dialog|jumps|pagemap|anchor=ID|key=ACCEL|image-click|image-viewer=<dump|in|out|fit|actual|toggle|pinch=F|pan=DX+DY|copy|save=FILE|close>|map|map-<dump|wheel=DY[|ctrl]|key=NAME|hover=PAGE|drag=A|B|zoom=in|out|click=F|close>|dwell=FRACTION|SECONDS|use-default|reset-look|theme-row|theme-edit=NAME|press=ACCEL|key=ACCEL|profile=<set=FIELD|VALUE|dump|theme-save=NAME|FG/BG|theme-apply=NAME|theme-delete=NAME|theme-export=FILE|theme-import=FILE>>`. A comma in an argument cannot be written, so id lists use `+`. `reader:profile=set=…` takes effect at once (the dump reads the settings it just changed).

Preferences, read-aloud bars and editor palette (`src/ui/preferences.rs`): `prefs:` opens Preferences, then `prefs:search=TEXT`, `prefs:pick=SECTION` and `prefs:dump` (prints `prefs: query=… sections=A|B selected=… hits=N collapsed=… empty=…`); `reader:tts-open` / `reader:tts-close` (the read-aloud popover), `until:<probe>[:<ms>]` (a script command: waits for a condition instead of a fixed sleep; probes `reader`, `page-ready`, `tts-card`, `tts-playing`, `tts-card-paused`, `cards-settled`; `!probe` waits for it to stop holding, e.g. `until:!reader` after `close:`; prints `until: <probe> ok|timeout`), `reader:cards-rects` (prints `cards: area=WxH sliding=0|1 shown=[id:x,y,w,h;…] target=[…] pops=[…]`: the card layout manager's final rectangles in overlay coordinates, the targets of the slides, and the popovers the cards avoid; checked by `build-aux/cards_geom.py` in the `cards_layout_*` scenarios; the `tts-player-drag` / `rsvp-drag` hooks raise the card like the end of a real drag), `reader:chrome` (prints `chrome: idle=… busy=… tts_popover=…`: the same decision the auto-hide uses), `reader:nav-layout` (`nav_layout: next_left_of_prev=… sidebar_end=… rtl=…`), `editor:cmdpalette` (opens the editor's command palette; drive it with `palette:type=`, `palette:dump`, `palette:pick`).

AI assistants page (`src/ui/mcp_settings.rs`, open it with `prefs:` then `prefs:pick=AI Assistants (MCP)`): `mcp:dump` (prints `mcp: running=… url=… enabled=… port=… write=… status=… token_shown=… clients=… leaked=… saved=…`; `leaked=1` means the full token was found on screen), `mcp:enable=on|off`, `mcp:port=N`, `mcp:write=on|off` (no confirmation dialog in scripts), `mcp:snippet=CLIENT[:http]` (`claude-desktop`, `claude-code`, `cursor`, `opencode`; copies like the buttons and prints `mcp_snippet:`), `mcp:copied` (`mcp_copied:` what is on the clipboard), `mcp:token=regen`, `mcp:ping` (a real `initialize`, an authorised and an unauthorised `search_books`, and an `add_tags` over HTTP to the running server: `mcp_ping: init=… search=… total=… no_token=… write=no-such-tool|done`). `@MCPPORT@` in a scenario script is replaced by a free port. The settings file is `mcp.json` next to `config.json`; `BOOKX_SECRETS=memory` keeps the token in memory. The server crate has its own tests: `cargo test -p bookx-mcp` (protocol over in-memory pipes, HTTP over loopback) and `cargo test -p bookx-cli --test mcp` (the real binary).

**Registering Preferences sections and rows.** Preferences is a sidebar of sections; each one is an `AdwPreferencesPage`. Built-in sections: General, Library, Reading, Devices and Sharing, E-mail, Sync, Extensions, Patches. A module adds its own with the registry in `src/ui/preferences.rs`, from `external_registrations()` (add one line there, it runs once before the first Preferences is built):

```rust
// a new section (Backup 70, Text to Speech 100, Advanced 110 are the suggested orders)
register_page("backup", "Backup", "document-save-symbolic", 70);
// rows: a group in a section, built when Preferences opens. `ctx.shared` is the app state,
// `ctx.parent` the window for dialogs, `(ctx.on_change)(PrefChange::…)` tells the application.
register_group("backup", 10, |ctx: &PrefCtx| {
    let g = adw::PreferencesGroup::builder().title("Destination").build();
    g.add(&adw::SwitchRow::builder().title("Back up automatically").build());
    g
});
```

`register_group("general", …)` etc. appends a group to a built-in section (after the built-in groups, ordered by the number). Sections without a group are not shown. Search, the command palette's settings index (`index()`), `prefs_focus` and the accessibility audit work on whatever the groups contain, so only use `adw::PreferencesRow` subclasses (`ActionRow`, `SwitchRow`, `ComboRow`, `EntryRow`…) with a title; name controls that have no text with `ui::audit::label`. The pure matching is `query_matches` (unit tests there).

Dictionary downloads (enhancement 30; scenarios `build-aux/ui-tests-dictionaries.sh`, real session `lookup_dictionary_download`): `BOOKX_DICT_CATALOG=FILE` replaces the catalogue (http://127.0.0.1 addresses allowed), `BOOKX_DICT_AUTO=on|off` overrides the setting (the harnesses set `off` everywhere so no scenario uses the network by itself; unset it to test the default), `BOOKX_OFFLINE=1` stops downloads, `BOOKX_DICTIONARIES` is the folder; `reader:ui-dict-prompt-btn` presses the main button of the offer; `reader:dump` prints `dict_prompt:` (state offer|working|failed|hidden, language, heading, button, progress) and `dict_service:` (active, failed, installed, pending, considered); `prefs:dict-auto=on|off` drives the switch; `build-aux/fake-dict-server.py PORTFILE DIR [DELAY_MS] [LANGS] [MODE] [WORDS]` is the fake server (writes `DIR/catalog.json`); `build-aux/dict-catalog.py generate|check|verify` makes and checks `data/dictionaries.json`.

Reader UX (sidebar, footer, handles, Wikipedia picture; scenarios in `build-aux/ui-tests-reader-ux.sh`, real session `build-aux/session/scenarios_reader_ux.py`, big test book `build-aux/make-big-epub.py OUT.epub [ENTRIES]`): `reader:bulk-highlights=N` (N generated highlights), `reader:demo-highlights` (five real passages with colours, styles and notes, for screenshots), `reader:ui-note-popover` (opens the selection popover of the first highlight that has a note, at the highlight's real place, for screenshots), `reader:side-open|side-close|side-tab=toc|bookmarks|notes|side-state` (prints `side:` with shown, collapsed, row counts, how often the notes list was rebuilt, its filter), `reader:side-time=open|close` and `reader:side-watch` (prints `side_time:` per toggle: `first_frame_ms`, `max_gap_ms`, `frames`, `work_ms` = the longest update+layout+paint of a frame; `side-watch` does it for every toggle the user makes), `reader:side-filter=TEXT|COLOUR|STYLE|note` (sets the notes search and filters, prints `side_filter: … -> N`), `reader:side-act=N:recolor:COLOUR|restyle:STYLE|note:TEXT|delete|go` (acts on the Nth highlight in book order, as the ⋮ menu does), `reader:footer` (prints `footer:` left | middle | right, bar, own, items, align, size, opacity), `reader:footer-preset=minimal|standard|everything`, `reader:footer-toggle=KEY`, `reader:footer-move=FROM:TO`, `reader:footer-own=on|off`, `reader:footer-tick`, `reader:footer-open`, `reader:handles` (prints `handles:` count, tip/ball of both, popover shown, selected text), `reader:handle-drag=start|end:DX:DY` (a drag of a handle through the code a finger uses), `reader:handle-clear`; `reader:dump` prints `wiki_image:` (frame shown, which page, size of the picture). `BOOKX_WEB_MOCK=DIR` answers Wikipedia from `DIR/wiki-search.json`, `wiki-summary.json` and `wiki-thumb.png` (the picture fails like a 404 when only the first two exist); `BOOKX_OFFLINE=1` fetches no pictures (cached ones show). `build-aux/wiki-screenshots-reader.sh` takes the wiki pictures of these features.

Accessibility and command palette (`docs/a11y-notes.md`): `a11y-audit:` (prints `a11y: controls=N problems=M`, one `a11y-problem:` line per control without an accessible name or an icon-only button without a tooltip, and `a11y-env:` with the desktop's animations / high-contrast / dark settings), `action:NAME` (activates an application action such as `share`, `devices`, `palette`), `palette:<open|dump|type=TEXT|down|up|pick|close|registry>` (`dump` prints the query, the selected row and the results; `registry` lists commands whose action does not exist), `reader:page-text` (the text a screen reader gets, and `book_rtl`), `reader:ksel=<char|word|sent><+|->` (Shift+arrows without a keyboard), `reader:selkey=<commit|dict|highlight|note|clear|copy>`, `reader:tap=X` and `reader:swipe-left|right` (what a tap at `X` 0..1 or a swipe does in this book). New controls should be named with `ui::audit::label(&widget, "…")` when they have no text of their own (a scale, a switch, an entry, a text view); icon-only buttons need `set_tooltip_text`. Real-session scenarios for the same live in `build-aux/session/scenarios_a11y.py`; `GDB=1` runs the app under gdb there (a backtrace of a crash lands in `app.log`), `G_DBUS_DEBUG=message` helps with the AT-SPI traffic.

Example — screenshots without touching your desktop, using GTK's headless Broadway backend
(the Wayland compositor stops rendering while the session is locked):

```bash
gtk4-broadwayd :5 &
GDK_BACKEND=broadway BROADWAY_DISPLAY=:5 BOOKX_LIBRARY=/tmp/lib \
BOOKX_DEV_SCRIPT="wait:1500,library:import=$HOME/Books,wait:8000,shot:/tmp/a.png,quit" cargo run
```

## Testing

`cargo test` covers the engine. `BOOKX_TEST_EPUBS=<dir>` additionally round-trips metadata, covers and TOCs of
real books and runs the checker on them.

## Tests

* `cargo test` — unit tests (run in parallel).
* How Relm4 tests UI: it has no UI-test framework. Its own tests are one integration-test binary per file (`tests/*.rs`),
  each starting a real `RelmApp`, because GTK must stay on one thread (`#[gtk::test]` even forces all GTK tests onto one
  main thread, serially). Bookx follows that: one *process* per scenario.
* `JOBS=16 build-aux/ui-tests.sh` sets how many scenarios run at once (default: all CPU cores, `0` = unlimited);
  `RUST_TEST_THREADS=16 cargo test` does the same for unit tests.
* `build-aux/ui-tests.sh` — end-to-end scenarios against the real widgets, **run in parallel**: each one gets its own
  Broadway display, D-Bus session, config and library. `NET=0` skips online look-ups, `ONLY='notes translate'` runs a subset.
* `build-aux/session-tests.sh` — **real-session tests**: Bookx runs as a Wayland client of a real GNOME Shell (Mutter,
  `--headless --virtual-monitor 1280x800`) and is driven with real pointer and keyboard events (Mutter's RemoteDesktop D-Bus
  API), checked through AT-SPI and screenshots (`<target>/session-results/*.png`, look at them). Use it for anything with
  popovers, focus, gestures or real input: Broadway cannot show those bugs (its dev hooks such as `reader:ui-dict-btn` call
  the button's handler directly and never go through GTK's gesture/popup machinery). `ONLY='lookup_dictionary header_menus'`
  runs a subset, `JOBS=2` sets the parallelism (capped by the free inotify instances, see `docs/test-notes.md`), `NET=0` skips
  the downloads and online look-ups, `SKIP_BUILD=1` reuses built binaries, `KEEP=1` keeps the work folders (shell, app and
  AT-SPI logs), `WAYLAND_DEBUG=client` traces the app's Wayland protocol (needed to see why Mutter killed it). Scenarios live in
  `build-aux/session/scenarios.py` (`@scenario def name(c)`, the helpers are in `harness.py`, `sandbox.py`, `atspi.py`).
  It needs `gnome-shell`, `python3` with PyGObject + Atspi + Pillow, `at-spi2-registryd`; `tesseract` is optional (menu and
  canvas text checks). Test books are downloaded to `~/.cache/bookx-testbooks` (`docs/test-books.md`).
* Scripts must end with `wait:…,quit`-style spacing: `quit` exits immediately, so a command sent right before it may never run
  (the harness adds the wait for you).
* Project rules (tests with every change, no shared state) are in `AGENTS.md`.

## The public wiki

`wiki/` is the user-facing site (https://bookx.adhadse.com, built with Zensical). `build-aux/wiki.sh` is its test:
it installs Zensical and Pillow into `~/.cache/bookx-wiki-venv` (uv if present, else `python -m venv`), builds `wiki/site`, checks the
`CNAME`, turns the screenshots into lazy-loaded WebP (`wiki-optimize.py`), runs `check-site-links.py` (a missing page, image,
stylesheet or `#anchor`, a screenshot without its light/dark partner, an unused screenshot) and `wiki-theme-check.py` (clicks the
palette toggle in headless Firefox and checks the right pictures show; `WIKI_BROWSER=0` skips it).
`build-aux/wiki.sh serve` previews it. `ZENSICAL_VERSION` (pinned in the script) and `VENV` can be overridden.
Screenshots (light and dark, with the window shadow): `build-aux/wiki-screenshots.sh`, see `docs/wiki-notes.md`.
`.github/workflows/wiki.yml` runs the same script on pull requests and deploys on pushes to `main`.

## Devices, sharing over HTTPS

* `BOOKX_DEVICE_ROOTS` (colon separated) replaces `/proc/mounts` as the place devices are looked for; `BOOKX_MTP_ROOT` is the folder
  holding gvfs-style `mtp:host=…/<storage>` directories (MTP is off when only `BOOKX_DEVICE_ROOTS` is set).
* Dev hooks: `library:ctxmenu` (the labels of the book menu), `library:send=<text in the device path>` (uses the saved
  format/template/shelf), `library:import-highlights=<path;path>` (read + plan like Import Highlights…, prints `import_summary:`/`import_plan:`), `library:import-confirm` (apply the review), `library:export-all=<fmt>:<folder>` (Export All Annotations), `library:annotate-more` (more highlights: a green one, a PDF-style one, one in the second book), `library:notes` (opens Notes & Highlights) then `library:notes:search=|group=book|month|color|tag|none|color=<name>|book=<id>|hasnote=yes|no|any|from=<YYYY-MM-DD>|select-all|select-first|clear|open-first|edit-first=<note>|delete-first|export=<id>:<path>|token=<readwise token>|readwise|dump|close` (prints `notes_state:`/`notes_edit:`/`notes_export_result:`/`readwise:`; wait ~400 ms after `search=`: the entry debounces), `reader:tts-save:chapter:<file.wav>|book:<folder>` (Save as audio without the chooser; prints `tts_save:`), `kosync:setup=<url>;<user>;<password>[;filename]|register|login|continue|stay|flush|queue-flush|queue-dump|dump` (reading position sync, offline queue `kosync_queue:`/`kosync_queue_flush:`; prints `kosync_pull:`/`kosync_prompt:`/`kosync_push:`; `build-aux/kosync-peer.py` is the second client of the `kosync_client*` scenarios), `devwin:show|dump|reload|device=<n or name part>|select=<i,j>|remove|sync|format=<id>|template=<text>|shelf=<name>|dav=<url>|view=grid|list|notes=on|off|autosync=on|off|syncask|syncconfirm|send|close`
  for the Device window (more hooks for the Device button, banner and panels: `docs/device-ui-notes.md`), `share:https`, `share:httpsown`, `share:httpsfiles:<cert>:<key>`, `share:gen`, `share:fetchtls:<path>[:untrusted]` for HTTPS sharing.
* Scenarios: `device_none`, `device_button`, `device_ui`, `device_sync_ui`, `device_auto_sync`, `details_panel`, `details_narrow`, `device_window`, `device_options`, `sharing_https`, `sharing_https_files`. `fake_devices` in
  `build-aux/ui-tests.sh` builds a Kobo with a SQLite database (python3's `sqlite3`), a Boox with a KOReader sidecar and an SD card.
* Real hardware (Kobo database writes, MTP, Kindle) has not been tested; the plan for the Calibre wireless protocol is in
  `docs/calibre-wireless-device-plan.md`.

## Library files, health and safety (crates `bookx-core`, `bookx-store`)

* **Layout** (`core/layout.rs`): `Tree` (default `Author/Title (id)/Title - Author.epub` + `cover.jpg`), `Flat`, `Template` (rendered by
  `bookx-sources::template::render_library_path`, registered once with `layout::register_template_renderer`) and `Linked`
  (book `file` is an absolute path, never moved or deleted). The choice lives in `<library>/.bookx/layout.json`, so the app and the CLI agree.
* **Moves** (`core/fsops.rs`, `core/files.rs`): a layout change or a metadata edit plans every move (`Library::plan_layout` is the dry run),
  validates it, applies it with rollback on the first failed rename (`apply_plan_with` takes the mover so tests inject failures) and only then
  updates the catalogue. Nothing ignores a rename error.
* **Trash and undo** (`core/history.rs`): deleting moves the files to `<library>/.bookx-trash/<stamp>-<id>/files/…` with an `entry.json`; the
  operation log `<library>/.bookx/oplog.json` (import, delete, edit, move; the last 200) feeds `Library::undo_last`.
* **Lock and read-only** (`store/guard.rs`): `<library>/.bookx-lock` names the process (pid, host); a lock of a dead process on this machine is
  taken over, one from another computer never. `open_library_guarded` returns a read-only library when somebody else has it. The same module
  detects network file systems (`/proc/self/mountinfo`) and sync-client folders.
* **Snapshots** (`store/health.rs`): `<library>/.bookx-backups/bookx-<UTC time>-n<books>-<auto|manual|prerestore>.db` (SQLite online backup).
  `retention` is a pure function: the newest N automatic ones plus the newest of each of the last 7 days; manual ones are never deleted.
* **Provenance**: table `provenance(book, field, source, at)` (schema v5) with `SqliteStore::set_provenance/provenance`. Recorded for
  catalogue-level facts only so far; see the roadmap for what is not wired.
* **Import folder** (`core/inbox.rs`): state in `<library>/.bookx/inbox.json`; files must be unchanged for 3 s before they are offered.
* UI: `src/ui/health.rs` (window, Preferences rows, start-up/exit hooks), glue and its tests in `src/library_health.rs`.

### Problems panel hooks (`editor:check:...`)

`run`, `dump` (prints `check:` and one `check_item:` per problem), `help`, `prefs`, `select=N`, `select-id=ID`, `open`, `fix`,
`fixkind`, `ignore`, `restore[=ID]`, `next`, `prev`, `a11y=on|off`, `edit=FILE`. Scenarios: `editor_problems`,
`a11y_audit_editor_problems` (ui-tests.sh) and `editor_problems_open` (session-tests, real double-click).

Spelling panel, reports window and special characters (`src/ui/editor/spelling*.rs`, `reports.rs`, `special_chars.rs`): `editor:spell:open|close|refresh|dump|sel`, `spell:select=WORD`, `spell:jump=WORD`, `spell:change=WITH` (Change All for the selected row), `spell:suggest-first`, `spell:ignore|ignore-book|add=DICT`, `spell:all=on|off`, `spell:bycount=on|off`, `spell:filter=T`, `spell:f8|shift-f8`, `spell:exclude|include=PATH|excluded-dialog`, `spell:manage`, `spell:new-dict=N|remove-dict=N|import-oxt=PATH|import-pair=PATH`; `editor:reports:open[=TAB]|tab=ID|filter=T|sort=COL|asc|desc|activate=N|select=N|N|next-place|delete-selected|delete-unused|csv=PATH|refresh|close|dump` (tab ids `files words images rules classes chars links`); `editor:chars:open|close|search=Q|category=NAME|insert=C[|keep]|fav=C|move=C|N|altx|semicolon|entities=on|off|focus-find|field|dump`. `$BOOKX_HUNSPELL_USER` moves the folder for imported dictionaries (the UI tests point it into the scenario's folder). Scenarios `editor_live_spellpanel*`, `editor_live_spelldicts`, `editor_live_a11y_spelling`, `editor_reports`, `editor_chars`; real-session `editor_reports_double_click` and `editor_alt_x`.

## Roadmap pass: saved searches, OPDS client, language, goals, footnotes, speed reading

Environment: `BOOKX_OPDS_MOCK=<dir>` serves `https://mock.example/opds/<file>` from that folder (OPDS client);
`BOOKX_LANG=en` pins the interface language (the UI tests set it); `BOOKX_LOCALE_DIR` adds a folder of compiled
`<lang>/LC_MESSAGES/bookx.mo` files.

Dev hooks (`library:` / `reader:` / `stats:` prefixes):

| Hook | What it does |
|---|---|
| `library:saved=add=NAME\|QUERY`, `delete=NAME`, `vl=NAME`, `vl-off`, `dump`, `open` | saved searches and the virtual library |
| `library:visible` | prints the ids the grid shows and the window subtitle |
| `library:opds=open`, `catalog=NAME`, `go=TITLE`, `more`, `back`, `search=TERMS`, `add=TITLE`, `add-catalog=NAME\|URL\|USER\|PW`, `dump` | online catalogues window |
| `library:language=CODE`, `library:menus-tr` | switch language, print the translated main menu |
| `ntr:N` | prints `ntr: <N books in the current language> | <1234567 formatted>` (plurals and number grouping); give `library:language=…` a `wait:400` first |
| `diag:` | prints the redacted diagnostics report on one line (`diag: …`); `action:copy-diagnostics` copies it |
| `open-uri:bookx://…`, `resolve-uri:bookx://…` | open a link as the desktop would / print `resolve_uri: book N\|no book\|invalid` |
| `stats:goal:MINUTES:BOOKS` | set the reading goals and print their text; `stats:dump` also prints the yearly review |
| `reader:link=ARCHIVE/PATH#frag` | follow a link as a click would (footnote popup or jump); `reader:dump` prints `footnote_popup:` |
| `reader:footnotes-off` | switch the footnote popups off |
| `reader:rsvp`, `rsvp-dump`, `rsvp-play`, `rsvp-pause`, `rsvp-faster`, `rsvp-slower`, `rsvp-step=N`, `rsvp-back`, `rsvp-close` | speed reading window |

Translations: mark strings with `tr("English")`, `trf("Sent {} to {}", &[&n, &name])` (placeholders), `ntr("{} book", "{} books", n)`
(plural; `{}` is the count), `tr_c("context", "English")` or `N_("English")` in constants (then call `tr(x)` where it is shown),
run `build-aux/i18n.sh` to rewrite `po/bookx.pot`, add the text to every complete `po/<lang>.po` (a unit test fails when es or de lack a
marked string, keep one that is gone, lose a `{}` or a plural form). A new language is a `po/<code>.po` plus a line in `LANGUAGES` in
`src/i18n.rs`; see the wiki page *Translating Bookx*. Never build a sentence from translated fragments or with
`if n == 1 { "" } else { "s" }`. Packaging, Flatpak and CI notes: `docs/packaging-notes.md`; `build-aux/flatpak.sh` builds and smoke-tests the
Flatpak, `make desktop-check` validates the desktop file, metainfo and MIME definitions.

PDF reader (`src/ui/reader_pdf.rs`, `reader:` commands, only while a PDF is open): `pdf-state` (prints `pdf_state:` with page, zoom, mode, scale, rendered/texts counts, scroll, selection, highlights, links, hits), `pdf-goto=N|LABEL`, `pdf-zoom=width|page|PERCENT`, `pdf-zoom-in|out`, `pdf-view=single|continuous`, `pdf-crop=on|off`, `pdf-invert=on|off`, `pdf-scroll-by=PX`, `pdf-turn[=back]`, `pdf-select=TEXT` (selects the first match from the current page and opens the popover), `pdf-highlight[=COLOR]`, `pdf-style=NAME`, `pdf-note=TEXT`, `pdf-copy`, `pdf-dict`, `pdf-clear`, `pdf-highlights` (prints what is stored), `pdf-search=Q` / `pdf-search-next|prev` / `pdf-search-state`, `pdf-bookmark`, `pdf-tab=pages|toc|bookmarks|notes`, `pdf-sidebar`, `toc-row=N`, `pdf-link=N` (follows the Nth link of the page; an address is printed as `pdf_link: uri …`, never opened), `pdf-back`, `pdf-password=PW`, `pdf-reflow`, `pdf-thumbs`, `pdf-lists`. Give a command a `wait:` before `close:` or `quit` or it may never run. Fixtures: `cargo run -p bookx-pdf --example make_test_pdfs DIR` (story, locked with password `sesame`, scan, margins, a 500 page `big.pdf`; `BIG=0` skips it); `bookx_pdf::testpdf` builds small PDFs in tests. Scenarios: `build-aux/ui-tests-pdf.sh` (Broadway) and `build-aux/session/scenarios_pdf.py` (real session). View settings live in `pdf.json` next to `config.json`.

## Editor tool dev hooks (`editor:x:<name>=<args>`)

Defined in `src/ui/editor/ext_dev.rs`; used by `build-aux/ui-tests-editor-tools.sh`. A comma in a text is written `§`. Examples: `x:smarten=all|smarten|de|qde`, `x:fixhtml=all|html5`, `x:styles=change|color|red|green;;scale|2`, `x:drop=<from>|<onto>|before`, `x:dialog=smarten` then `x:dialog-click=preview|apply`, `x:save-copy-edit=<path>`, `x:open-names=a.xhtml|text`. Environment: `BOOKX_FONT_DIRS`, `BOOKX_EDITOR_NET_MOCK`.

## Read aloud, speed reading and the cards (dev hooks)

* `reader:tts-player` dumps the read-aloud card (`pos=` fractions, `size=`, `collapsed=`, `text="N / M"`, `offered=`);
  `reader:tts-player-click:<play|stop|next|prev|next-chapter|prev-chapter|faster|slower|collapse|cancel|retry|copy>`,
  `reader:tts-player-seek:<0..1>`, `reader:tts-player-drag:<fx>:<fy>` (place as fractions).
* `reader:rsvp` opens speed reading; `reader:rsvp-<dump|play|pause|faster|slower|back|next|close|step=N|drag:<fx>:<fy>|key:<space|left|right|up|down|escape>>`.
* `reader:tts-download:yes|always` answers the first-time download dialog; `reader:work-watch` prints
  `work: t=<ms> draws=N layouts=M` of the page canvas every 100 ms (the drag test counts them).
* Environment: `BOOKX_TTS_MOCK=<ms>` silent engine; `BOOKX_TTS_MOCK_DOWNLOAD=<ms>|offline` fake voice download (marker `.mock-voice` in
  `$BOOKX_TTS_DIR`); `BOOKX_TTS_MOCK_ESPEAK=missing`; `BOOKX_NO_MPRIS=1`. `cargo run -p bookx-tts --example fake_install -- <dir> <pack>...`
  makes packs that look installed; `--example dl_bench -- <dir> [pack]` times the real download (`BENCH_ONLY=1`, `BENCH_SECS=n`).
* Real-session harness: `c.restart_app()` quits and relaunches the app in the same sandbox (state survives).
