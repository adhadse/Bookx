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
| `BOOKX_STALL_WATCH=MS` | print `stall: N ms after <last dev command>` for every stretch of MS or more in which the main loop did not run |
| `BOOKX_FILL_SYNC=1` | fill the library grid in one call instead of in pieces (the old behaviour; to measure against) |
| `BOOKX_LAYOUT_SYNC=1` | lay a chapter out in one call on the main loop instead of in slices (the old behaviour; to measure against) |
| `BOOKX_SLICE_FROM=N`, `BOOKX_WINDOW_FROM=N` | chapters of N blocks or more are laid out in slices / shown from a window around a saved position (defaults 120 and 240; `=1` runs every reader scenario through the sliced layout, `src/flow_window.rs`) |

Script commands: `wait:<ms>`, `shot:<png>[@window]`, `read:<id>`, `edit:<id>`, `close:`, `prefs:`, `quit`,
`library:<table|grid|select1|selectall|editmeta|import=<dir>|search=<q>>`,
Library Health: `library:health=<open|dump|backup|check|drift-on|restore1|fix1|undo|trash-restore1|trash-empty|tidy|inbox-scan|inbox-add-all|inbox-add1|inbox-dismiss1|auto-off|keep=N|inbox-folder=DIR|calibre=DIR[|nocopy]|layout=NAME[|TEMPLATE]>` (buttons are pressed for real; a disabled button prints `health: button is disabled`; `dump` prints `health_*` lines); backup places (the incremental repository, D9): `library:health=<backup-add=NAME|PATH[|encrypt][|keep=N][|auto][|nobooks][|nocovers][|maxmb=N]|backup-add-dav=NAME|URL|USER|PASSWORD|FOLDER[|encrypt]|backup-passphrase=TEXT|backup-remove1|backup-reload|backup-pick=N|backup-restore-remote1|backup-restore-mode=missing|replace|backup-restore-to=DIR|backup-restore-dialog|backup-dialog-mode=missing|replace|backup-dialog-preview|backup-dialog-restore|backup-dialog-dump|backup-place=N|backup-check=N|backup-check-deep=N|backup-clean=N|backup-journal|backup-cancel|backup-scope|backup-dump>` (`auto` allows scheduled copies without a manual first copy; `backup` presses Back Up and prints `backup: copied NAME … new_files=N reused_files=M up_to_date=B`; `backup-restore-remote1` takes the script's shortcut (a dry run, then the restore into the open library, replacing, or `backup-restore-to` / `…-mode=missing`), `backup-restore-dialog` opens the real window; `backup-scope` prints the `backup_scope:` line used by `backup_scope_explanation`; `backup-dump` prints `backup_place:`, `backup_banner:` and `backup_*` lines; `backup_plan:` and `backup_stats:` lines say what each place shows under its name; see `docs/backup-notes.md`); `BOOKX_SECRETS=memory|none` keeps passwords in memory only / pretends there is no keyring (the UI tests set `memory`); `BOOKX_READ_ONLY=1` opens the library read-only,
Library wave 9 (see the header of `build-aux/ui-tests-lib8.sh`): `library:undo-last` (presses the Undo of the last library toast), `library:tablecols`, `library:sortcol=Name[:desc]`, `library:sortstate`, `library:hover=N|unhover=N|checks|check=N|longpress=N|cardclick=N|carddesc=N`, `library:details-click|details-state`, `library:gridwidth`, `library:group=Authors|group-expand=Authors|group-showall=Authors`, `library:saved-state|tips-open|tips|tips-close|tips-save=NAME`, `library:shelf-empty|shelf-empty-click|shelf=delete-undo=NAME`, `library:paste=TEXT|webdialog|add-web-open|drag-enter|drag-leave|drophint`, `library:dialog|dialog-close|import-dialog|copy-citation`, `library:focus-order|focus-grid|focus-where`, `library:works=activate:first`, `library:notes:export-menu|anki-dialog=apkg|anki-id=FILE:cloze:vocab|edit-dialog|edit-type=T|edit-save|edit-cancel|edit-keys|toolbar`, `jobs:copy-error=ID|icon`. Table columns by width: `crate::library_nav::table_columns_for_width`; the BOOKX_SIZE suffix of a scenario name (`_900x600`) cannot exceed the 1024 px of the Broadway display.

Layout in slices (X18, `docs/performance.md`): `reader:goto-frac=SPINE:FRAC` opens a chapter at a fraction the way a saved position does, `reader:layout-state` prints `layout: complete=… shown=… window=… frac=… page=… pages=… items=… layout_ms=… total_h=… flow_h=… y_off=…` (`window=true`: the page is shown from a window while the whole chapter is laid out beside it; `pages`/`frac` are estimates until `complete=true`), `library:selcount` prints `selcount: model=N selected=M items=K` (Select All of 100,000 books without listing the ids). Scenarios `big_book_window` (a 10 MB book) and `scale_library_100k` (100,000 books through the Calibre importer).

Grid fill (`src/grid_fill.rs`, `src/grid_columns.rs`): `library:stalls` (sort, search, clear search, sort back; prints `stalls: sort=N search=N clear=N sortback=N max=N`, the longest main-loop block in ms of each), `library:fillstate` (`tiles=` alive in the grid, `adj=value/page/upper`, `grid=`/`table=` rows, `filling=`), `library:fillselect` (rebuild, then select all before the rest of the list arrived), `library:scroll-to=PX`. Scenarios `grid_stalls_2000` (Broadway) and `library_stalls_2000` (real session).
Metadata review and library order (`docs/metadata-notes.md`): `library:bulkfetch` (starts *Fetch Metadata…* for the selection), `library:bulkfetch=<dump|accept-all|accept-none|accept-safe|accept-field=F|toggle=N|check=N|apply|undo|auto-on|auto-off|close>` (`dump` prints `bulkfetch:`, `bulkfetch_health:` and one `bulkrow:` per row), `library:preflight` / `library:preflight=<dump|fix|continue|dest=kindle|kobo|email|any>`, `library:organize=<series|works|attention|series-dump|works-dump|attention-dump|idclean>`, `library:seriestext=<text>` / `library:seriesdump` (series number field of the open edit dialog), `library:undo=N` (take back N operation-log entries). `$BOOKX_SOURCES_MOCK/<openlibrary|google>.status` containing a number makes that source answer with it (outages, rate limits). The search term `weak:` finds books with poor metadata.

Editions of one work (PP-G-5, `src/work_groups.rs`, `src/ui/library_works.rs`): `library:works=<on|off|link:ID+ID|unlink:ID|prefer:ID|card:ID[:ctrl]|show:ID|cards|selected>` (`cards` prints `works: cards=N books=M groups=[rep=[members]] subtitle=…`, `selected` prints the cards and the books they reach, `show` opens the edition list of a card and prints its row count; ids are separated by `+` because the script splits on commas). Scenarios: `build-aux/ui-tests-works.sh` (seed in `ui-tests.sh`: books 1 and 2 are editions, 3 their German translation) and the real-session `works_editions_*` in `build-aux/session/scenarios_works.py` (`Ctx.start(seed=fn)` runs `fn(c)` in the sandbox before the app starts).

Formats, authors, trackers, audiobooks, DRM, import lookup (`docs/metadata-notes.md`, second round): `library:formats=<dump|rows=ID|drm|retype=ID:EXT|merge-preview=ID,ID|merge=ID,ID|add=ID:/file|remove=ID:FORMAT|offers=add|separate|skip>` (`retype` renames a book's file, the way another tool would have filed it as AZW3; `offers=` answers the "add as another format" question of the next import without a dialog), `library:authors=<dump|open|merge=A|B>>Canonical|alias=A>>Canonical|rename-preview=A|B>>Name>`, `library:trackers=<open|plan=/file.csv|import=/file.csv|export=goodreads|storygraph:/file.csv>`, `library:audiobooks=<open|open=/dir|find=/dir|link=/dir|dump>`, `library:bulkedit=tag:+x|tag:-y|author:A|series:S|publisher:P|language:l|rating:N` (the bulk editor on the selection, same code as the dialog), `editor:meta-apply=publisher:P|title:T|series:S` (the editor's Metadata tab apply path), `library:bulkfetch=cache` (cache hits and misses). `$BOOKX_SOURCES_CACHE=1` switches the response cache on while `$BOOKX_SOURCES_MOCK` is set (the tests normally skip it); `{"metadata_prefs":{"fetch_on_import":true}}` in the config file turns on the lookup after an import.
News: `library:newsadd=<title>|<url>|<manual|hourly|daily 07:00|weekly mon 07:00>|<keep>`, `newsopml=<title>|<file>`, `newsfeedoff=<key>|<url>`, `newsrun=all`, `newstick` (run what is due; move the clock with `stats:skew:<s>`), `newsshow`, `newsdialog`, `newsdump`.
`reader:<goto=N|next|chapter|toc|notes|search|select|hl|note|bookmark|light|sepia|dark|single|two|scroll>`,
Reader tests (`reader:` too): `dump` (prints `breaks:`, `wrap:`, `rtl:`, `fixed:`, `hyphens:` and the rest), `ui-dblclick` (a double click on the first word as the mouse makes it), `ui-dict-btn`, `ui-wiki-btn` (press the real buttons), `ui-optimal-on|off`, `ui-bench`, `ui-selectall`, `ui-vertical` (prints `vertical:` with the writing mode, pages, ruby and tate-chu-yoko counts, kinsoku violations, a hit-test check and the size of the page, then `vlines:` the text of every column of the page and `vprobe:` where characters 0, 5 and 12 are drawn), `ui-vsel:A:B` (a drag from the middle of character A to the middle of character B, prints the selection, its text and its screen rectangles), `ui-shot:FILE` (paints the page to a PNG: look at layouts without a screen), `ui-tweak:KEY=VALUE` (KEY: `scope=all|book`, `line_height`, `margin`, `spacing`, `font`, `justify=on|off`, `optimal=on|off`, `hyphen=off|de|…`, `css=…`, `reset=1`; drives the real Style Tweaks dialog), `ui-tweaks-state`, `tts-play|pause|stop|voices`, `tts-speed:X`, `tts-pitch:X`, `tts-voice:NAME`, `tts-skip:N` (sentences, negative = back), `tts-sleep:chapter|MINUTES`, `tts-dump`. `BOOKX_TTS_MOCK=<ms>` replaces the voice by a silent speaker that finishes a sentence after that many milliseconds (it queues two sentences ahead, like a real engine). `BOOKX_TTS_DIR` is where models are looked for (tests point it at an empty folder), `BOOKX_TTS_PLAYER` an absolute program path plus arguments (run directly, no shell) that receives the raw s16le samples on stdin instead of pacat/pw-cat/aplay (`/bin/cat` in tests; `{rate}` is replaced in each argument), `BOOKX_TTS_THREADS` the ONNX threads, `BOOKX_ESPEAK` the espeak-ng program, `BOOKX_TTS_API_KEY` the bearer key for voice servers. Settings page hooks (`tts:` commands, Preferences open): `tts:engine=ID`, `tts:server=URL`, `tts:test`, `tts:dump`. `build-aux/tts-fake-server.py` is an OpenAI-compatible fake server for scenario `tts_server`; `build-aux/tts-catalog.py` regenerates the pinned download list; `cargo run --release -p bookx-tts --example speak` makes a WAV with a downloaded model and prints the real-time factor; `BOOKX_TTS_LIVE=1 BOOKX_TTS_DIR=… [BOOKX_TTS_LIVE_PACK=kokoro-int8] cargo test --release -p bookx-tts live_download -- --nocapture` downloads a real pack (opt-in).
`editor:<open=<path>|find|check|toc|meta|gentoc|first-image|save>`, and for the editor scenarios `goto=N`, `mark`, `cursor=N`, `select=a-b`, `find-in-text=TEXT[|sel|end]`, `insert=TEXT`, `tag=ACTION[|ARG]`, `has=TEXT`, `cur`, `buf`, `cat=PATH`, `cps`, `tabs`, `tab-close`, `step=N`, `quickopen=Q`, `select-files=a|b`, `merge-selected`, `bulk=PREFIX|N`, `newfile=NAME|TEMPLATE[|link]`, `linkcss[=remove]`, `export=DIR`, `delete=a|b`, `move=up|down`, `addpaths=P`, `title=T`, `tocgen=SOURCE|LEVEL|MODE[|PAGE]`, `toc-go=N`, `toc-dump`, and for the GtkSourceView code view `view` (language, scheme, options), `scheme=dark|light|eink`, `view-opt=minimap|whitespace:on|off`, `live` (run the debounced pass now), `marks`, `tip=LINE`, `spots`, `spell-menu=WORD`, `spell-act=replace|WORD|WITH` / `ignore|WORD` / `add|WORD`, `pair`, `hidden`, `find-set=Q`, `find-opt=regex|noregex|case|dotall`, `find-status`, `complete=TEXT-BEFORE-CARET` (⏎ for a line break), `complete-show` + `complete-last` (open the real popup), `snippet=TRIGGER`, `snippet-add=NAME|TRIGGER|any/markup/css|BODY`, `snippets`, `snippets-dialog`, `close-results` (commas cannot appear in arguments; `\n` in `insert=` is a line break). The vertical-writing test books are made by `build-aux/make-vertical-epubs.py` (`synthetic`, `synthetic-long`, `aozora SRC.html OUT.epub`; see `docs/test-books.md`). `SHOW=scenario ONLY=scenario build-aux/ui-tests.sh` also prints everything the app said in that scenario. Scenarios named `editor_live_*` get a tiny Hunspell dictionary through `BOOKX_HUNSPELL`. `BROADWAY_BASE=N` moves the Broadway display numbers of `build-aux/ui-tests.sh` (use it when several test runs share a machine), `CARGO_TARGET_DIR` is honoured.
Code-editing helpers (`src/ui/editor/{navigate,format_actions,markup_hints,insert_dialogs}.rs`, `editor:code:<name>=<args>`; `|` separates arguments, a comma in a text is `§`): `ctrlclick=NEEDLE[|K]` (what Ctrl+click at K characters into the first NEEDLE does; prints `editor_code: jumped current=PATH at=CHAR toast=…` and the buffer with the selection as ⟦…⟧), `gotodef` (F12), `menu=NEEDLE[|K]` (labels of the code section of the context menu), `help=NEEDLE[|K]` (the MDN URL), `rename-preview=OLD|NEW` (`total= files= collides=`), `rename=OLD|NEW` (checkpoint + apply), `rename-dialog=OLD`, `format=bold|italic|underline|strike|sup|sub|code` (acts on the selection; prints the buffer and the toast), `split`, `sort-css`, `insert-image=HREF|ALT|TITLE`, `insert-link=HREF|TEXT|TITLE`, `dialog=image|link`, `autoclose=TEXT` (types TEXT at the caret and runs the `</` completion), `type=C` (a typed character; `swallowed=true` when it repeats what `</` inserted), `dump`, `hints` (`invalid= link_ok= link_missing= colors= bold= italic=`), `hints-tags` (the text under each decoration tag and each colour swatch), `hint-opt=invalid|links|colors|emphasis|auto-close:on|off`, `tooltip=NEEDLE[|K]`. Scenarios `editor_code_*` in `build-aux/ui-tests-editor-code.sh`; real session `editor_code_*` in `build-aux/session/scenarios_editor_code.py`.
Editor preview sync, splitting, compare view, checkpoints dialog and Live CSS: `editor:pv-*`, `editor:cmp-*`, `editor:cp-*`, `editor:css-*` (full list in `docs/editor-preview-notes.md`).

UX gaps (`docs/ux-gaps-notes.md`): `keys:<open|dump=ACTION|press=ACTION|ACCEL|settle=swap|take|cancel|clear=ACTION|reset=ACTION|resetall|filter=TEXT|export=FILE|import=FILE|local|dialog=ACTION>` (the Keyboard Shortcuts page; `press` runs what the capture dialog does after a key press, a conflict waits for `settle`), `library:shelf=<create=NAME|rename=OLD|NEW|delete=NAME|add=NAME|ID+ID|addsel=NAME|remove=NAME|ID+ID|drop=NAME|ID+ID|up=NAME|down=NAME|bookup=NAME|ID|bookdown=NAME|ID|open=NAME|menu|dump>` (`drop` runs the drop handler of a shelf row), `library:libs=<add=NAME|PATH|folder=PATH|switch=NAME|rename=OLD|NEW|remove=NAME|new=NAME|PARENT|dump>`, `jobs:<start=KIND|TITLE|cancel|progress=ID|FRACTION|DETAIL|log=ID|TEXT|finish=ID|ok|REASON|cancel=ID|clear|open|close|rows|dump>`, and for the reader `reader:<goto-text=TEXT|back|forward|goto-dialog|jumps|pagemap|anchor=ID|key=ACCEL|image-click|image-viewer=<dump|in|out|fit|actual|toggle|pinch=F|pan=DX+DY|copy|save=FILE|close>|map|map-<dump|wheel=DY[|ctrl]|key=NAME|hover=PAGE|drag=A|B|zoom=in|out|click=F|close>|dwell=FRACTION|SECONDS|use-default|reset-look|theme-row|theme-edit=NAME|press=ACCEL|key=ACCEL|profile=<set=FIELD|VALUE|dump|theme-save=NAME|FG/BG|theme-apply=NAME|theme-delete=NAME|theme-export=FILE|theme-import=FILE>>`. A comma in an argument cannot be written, so id lists use `+`. `reader:profile=set=…` takes effect at once (the dump reads the settings it just changed).

Preferences, read-aloud bars and editor palette (`src/ui/preferences.rs`): `prefs:` opens Preferences, then `prefs:search=TEXT`, `prefs:pick=SECTION` and `prefs:dump` (prints `prefs: query=… sections=A|B selected=… hits=N collapsed=… empty=…`); `reader:tts-open` / `reader:tts-close` (the read-aloud popover), `until:<probe>[:<ms>]` (a script command: waits for a condition instead of a fixed sleep; probes `reader`, `page-ready`, `tts-card`, `tts-playing`, `tts-card-paused`, `cards-settled`; `!probe` waits for it to stop holding, e.g. `until:!reader` after `close:`; prints `until: <probe> ok|timeout`), `reader:cards-rects` (prints `cards: area=WxH sliding=0|1 shown=[id:x,y,w,h;…] target=[…] pops=[…]`: the card layout manager's final rectangles in overlay coordinates, the targets of the slides, and the popovers the cards avoid; checked by `build-aux/cards_geom.py` in the `cards_layout_*` scenarios; the `tts-player-drag` / `rsvp-drag` hooks raise the card like the end of a real drag), `reader:chrome` (prints `chrome: idle=… busy=… tts_popover=…`: the same decision the auto-hide uses), `reader:nav-layout` (`nav_layout: next_left_of_prev=… sidebar_end=… rtl=…`), `editor:cmdpalette` (opens the editor's command palette; drive it with `palette:type=`, `palette:dump`, `palette:pick`).

Wave 9 hooks (scenarios in `build-aux/ui-tests-prefs8.sh`, `prefs9_*`): `prefs:titles` (`prefs_titles: rows=N problems=…`: row titles over 48 characters, saying "my" or ending in an ellipsis), `prefs:fit` (`prefs_fit: section= natural_w= alloc_w= dialog=WxH window=WxH fits=B`), `prefs:eink=on|off`, `prefs:email-dump|email-test` (`prefs_email: titles=… test_sensitive=… tip=… result=…`; `prefs9_email_test_sends` gets the fake SMTP server like `email_book`), `prefs:stats-dump|stats-daily=on|off|stats-daily-amount=N` (`stats_ui: chart_label=… chart_desc=… daily_on=… daily_amount_visible=… daily_amount=… yearly_on=…`, open the window with `stats:show`), `prefs:sync-requirements` (opens the requirements dialog, `prefs_sync_requirements: groups=N titles=…`), `prefs:sync-account-dump` (adds `register_sensitive`, `login_sensitive`, `login_tip`, `status_controls`), `prefs:comic-dump` (adds `grey_sensitive`, `animate_sensitive`), `library:health=dump` (adds `health_ui: problems_visible= calibre_visible= check_button= status_titles=`) and `library:health=trash-heading`, `keys:resetall` / `keys:undo` (`keys: resetall -> … customised=B`, `keys: undo pressed=B customised=B`), `audio:rmbookmark=N` then `audio:undo`.

AI assistants page (`src/ui/mcp_settings.rs`, open it with `prefs:` then `prefs:pick=MCP`): `mcp:dump` (prints `mcp: running=… url=… enabled=… port=… write=… status=… token_shown=… clients=… leaked=… saved=…`; `leaked=1` means the full token was found on screen), `mcp:enable=on|off`, `mcp:port=N`, `mcp:write=on|off` (no confirmation dialog in scripts), `mcp:snippet=CLIENT[:http]` (`claude-desktop`, `claude-code`, `cursor`, `opencode`; copies like the buttons and prints `mcp_snippet:`), `mcp:copied` (`mcp_copied:` what is on the clipboard), `mcp:token=regen`, `mcp:ping` (a real `initialize`, an authorised and an unauthorised `search_books`, and an `add_tags` over HTTP to the running server: `mcp_ping: init=… search=… total=… no_token=… write=no-such-tool|done`). `@MCPPORT@` in a scenario script is replaced by a free port. The settings file is `mcp.json` next to `config.json`; `BOOKX_SECRETS=memory` keeps the token in memory. The server crate has its own tests: `cargo test -p bookx-mcp` (protocol over in-memory pipes, HTTP over loopback) and `cargo test -p bookx-cli --test mcp` (the real binary).

**Registering Preferences sections and rows.** Preferences is a sidebar of sections; each one is an `AdwPreferencesPage`. Built-in sections: General, Library, Reading, Sync (the one page for everything that syncs: `src/ui/sync_prefs.rs`, groups listed in `src/sync_model.rs`; the old “Devices and Sharing” name is an alias for `prefs:pick=` and the search), E-mail, Extensions (Plugins and Patches); registered by modules: Dictionaries 35, Editor 40, Text to Speech 50, Privacy 90, Keyboard Shortcuts 100, AI Assistants 110 (`prefs:pick=Patches` and `AI Assistants (MCP)` are kept as aliases, `legacy_section_id`). Sync hooks: window `sync:dump|primary|card=wifi|usb|opds|advanced=open|close`, page `prefs:sync-dump|server=on|opds=on|port=N|req|adv|advanced=open|plugin|ko|account=<user>;<pw>|account-register|account-login|account-dump`, `share:port0`, `action:sync`; scenarios in `build-aux/ui-tests-sync.sh` and `build-aux/session/scenarios_sync.py`; design in `docs/sync-ux-notes.md`. A module adds its own with the registry in `src/ui/preferences.rs`, from `external_registrations()` (add one line there, it runs once before the first Preferences is built):

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

Own vocabulary (enhancement 32; scenarios `build-aux/ui-tests-vocab.sh`, real session `build-aux/session/scenarios_vocab.py`): `reader:select-text=T` selects the first place `T` stands in the chapter, `reader:ui-vocab-add` presses the + button, `reader:ui-vocab-dialog=term=X|def=X|note=X|scope=book|all|series|save` drives the open dialog through its widgets (prints `vocab_dialog:`), `reader:ui-vocab-click` clicks the first saved term on the page, `reader:ui-vocab-mark=edit|remove|open` presses the popover buttons (prints `vocab_mark:`), `reader:ui-vocab-def-btn` presses *Add / Edit my definition* of the look-up popover, `reader:ui-vocab-terms`, `reader:vocab-dump` (`vocab_marks:` total / on page / first rect / texts, `vocab_def:`), `vocab:own=term|definition[|book id]` saves a term without the dialog, `vocab:own-dump`, `vocab:show-words`, `vocab:list=filter=mine|search=Q|add=T|D|paste-open|paste-text=…|paste-add|import=FILE|export=csv|json|anki|stardict:PATH|edit=T|delete=T` (prints `vocab_list:` rows, `[mine]` is the badge), `prefs:vocab-marks=on|off`. Dev scripts split on commas: definitions in tests have none.

Reader UX (sidebar, footer, handles, Wikipedia picture; scenarios in `build-aux/ui-tests-reader-ux.sh`, real session `build-aux/session/scenarios_reader_ux.py`, big test book `build-aux/make-big-epub.py OUT.epub [ENTRIES]`): `reader:bulk-highlights=N` (N generated highlights), `reader:demo-highlights` (five real passages with colours, styles and notes, for screenshots), `reader:ui-note-popover` (opens the selection popover of the first highlight that has a note, at the highlight's real place, for screenshots), `reader:side-open|side-close|side-tab=toc|bookmarks|notes|side-state` (prints `side:` with shown, collapsed, row counts, how often the notes list was rebuilt, its filter), `reader:side-time=open|close` and `reader:side-watch` (prints `side_time:` per toggle: `first_frame_ms`, `max_gap_ms`, `frames`, `work_ms` = the longest update+layout+paint of a frame; `side-watch` does it for every toggle the user makes), `reader:side-filter=TEXT|COLOUR|STYLE|note` (sets the notes search and filters, prints `side_filter: … -> N`), `reader:side-act=N:recolor:COLOUR|restyle:STYLE|note:TEXT|delete|go` (acts on the Nth highlight in book order, as the ⋮ menu does), `reader:footer` (prints `footer:` left | middle | right, bar, own, items, align, size, opacity), `reader:footer-preset=minimal|standard|everything`, `reader:footer-toggle=KEY`, `reader:footer-move=FROM:TO`, `reader:footer-own=on|off`, `reader:footer-tick`, `reader:footer-open`, `reader:handles` (prints `handles:` count, tip/ball of both, popover shown, selected text), `reader:handle-drag=start|end:DX:DY` (a drag of a handle through the code a finger uses), `reader:handle-clear`; `reader:dump` prints `wiki_image:` (frame shown, which page, size of the picture). `BOOKX_WEB_MOCK=DIR` answers Wikipedia from `DIR/wiki-search.json`, `wiki-summary.json` and `wiki-thumb.png` (the picture fails like a 404 when only the first two exist); `BOOKX_OFFLINE=1` fetches no pictures (cached ones show). `build-aux/wiki-screenshots-reader.sh` takes the wiki pictures of these features.

Accessibility and command palette (`docs/a11y-notes.md`): `a11y-audit:` (prints `a11y: controls=N problems=M`, one `a11y-problem:` line per control without an accessible name or an icon-only button without a tooltip, and `a11y-env:` with the desktop's animations / high-contrast / dark settings), `action:NAME` (activates an application action such as `share`, `devices`, `palette`), `palette:<open|dump|type=TEXT|down|up|pick|close|registry>` (`dump` prints the query, the selected row and the results; `registry` lists commands whose action does not exist), `reader:page-text` (the text a screen reader gets, and `book_rtl`), `reader:ksel=<char|word|sent><+|->` (Shift+arrows without a keyboard), `reader:selkey=<commit|dict|highlight|note|clear|copy>`, `reader:tap=X` and `reader:swipe-left|right` (what a tap at `X` 0..1 or a swipe does in this book). New controls should be named with `ui::audit::label(&widget, "…")` when they have no text of their own (a scale, a switch, an entry, a text view); icon-only buttons need `set_tooltip_text`. Real-session scenarios for the same live in `build-aux/session/scenarios_a11y.py`; `GDB=1` runs the app under gdb there (a backtrace of a crash lands in `app.log`), `G_DBUS_DEBUG=message` helps with the AT-SPI traffic.

Windows and dialogs (backlog wave 9, scenarios in `build-aux/ui-tests-win8.sh`): `a11y-audit:` also prints `dialog_fit: title= dialog=WxH window=WxH fits= default_visible= clipped_buttons=` for the dialog that is open (X6, XC-7: the dialog is not bigger than the window and its default button is on screen), `dialog:ctrlw` (Ctrl+W of the open dialog), `library:trackers=window=FILE|state|press-import` (`trackers_window:` apply label, preview counts, not-found list), `library:audiobooks=open[=DIR]|state|unselect=N` (`audiobooks_window: page=loading|list|empty rows= link= sensitive=`; the search runs on a worker thread), `library:opds=cancel` (the loading page's Cancel; `opds_view:` also prints `search_open=` and `retry=`), `vocab:dialog=remove` (the Remove Definition button, an Undo toast follows), `library:import-confirm` prints `import_review: label=` before it confirms, `reader:map-focus-note` (keyboard cursor onto the first note: the card shows it, `note=` in `book_map:`). A raw string passed to `set_description`, `set_title`, `set_subtitle`, `set_placeholder_text`, `set_tooltip_text` or `set_heading` under `src/ui/` fails `cargo test lint` (`ui/i18n_lint.rs`), and so does `tr(CONST)` on a constant that is not declared with `N_("…")`.

Readers (backlog wave 9, scenarios in `build-aux/ui-tests-reader8.sh`): `app:toast-undo-test` shows an undo toast (the one used by every removal: `Msg::ToastUndo(text, UndoFn)` in `src/app.rs`, `Output::ToastUndo` of the readers), `app:toast-undo-press` presses the Undo of the toast shown last (`toast_undo: none` when there is none), `app:fail-open` makes the next book fail to open and `app:open-error` prints `open_error: shown= title= description= button=` of the error page, `app:win-size=WxH` / `app:win-dump` resize and report the window (the reader's narrow form starts at 600 sp), `reader:menu-dump` (also in PDF and comic readers: `reader_menu: entries= sections= submenus= narrow= | Label [action] | ...`), `reader:menu-popup|menu-popdown`, `reader:ruler-state` (the check item against the setting), `reader:header-dump` (`reader_header: width= title= clipped= fullscreen= read_aloud= back= narrow= min_header= min_page=`), `reader:text-dump` / `text-more=open|close` / `text-hyphen=on|off` (Text Appearance and More Text Options), `reader:footer-dialog=dump|toggle=KEY|up=KEY|down=KEY` (the real widgets of the Footer Statistics dialog, `footer_dialog:`), `reader:goto-hint=TEXT` (the live line of Go to), `reader:hl-count` (`marks: highlights= notes= bookmarks=`), `reader:remove-highlight` (presses the Remove Highlight button), `reader:bookmark-toggle`, `reader:esc` (prints the action Escape takes, then presses it: `esc: ClearSelection`), `reader:opening-state|opening-demo` (the Opening Book page), `reader:sel-dump` (size of the selection popover), `reader:ruler=place:Y|drag:Y0:DY` (a tap and a finger drag on the reading ruler), `vocab:list=undo` (Undo of the last removal; the dump has `max_buttons=`), comic `reader:view-dump`, `reader:comic-failed-text`, `reader:goto-dialog`. Give a command that is sent to the reader a `wait:` before an `app:` command that depends on it (`reader:` commands run on the reader's own queue, `app:` ones at once).

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

* `build-aux/coverage.sh` — the coverage gate: `cargo llvm-cov nextest` over the workspace (install once:
  `cargo install cargo-llvm-cov cargo-nextest --locked` and `rustup component add llvm-tools-preview`), then
  `build-aux/coverage-gate.py` turns the lcov file into a per-crate table and compares it with
  `build-aux/coverage-baseline.json`. `--quick` skips the GTK app crate (no GTK build, fastest), `--html` writes
  `target/llvm-cov/html/index.html`, `--update-baseline` raises the baseline after you improved coverage,
  `--base origin/main` adds the patch check (`pip install diff-cover`). `build-aux/coverage-ui.sh` measures `src/ui/**`
  through the Broadway scenarios. Policy, exclusions and branch protection: `docs/ci-gates.md`.
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
Screenshots (light and dark, with the window shadow): `build-aux/wiki-screenshots.sh`, see `docs/wiki-notes.md`. The README pictures and the AppStream screenshots are not copies: they reference the same generated `wiki/docs/assets/<name>-light|dark.png` files (README via `<picture>` for light/dark), so a screenshot run updates them. `build-aux/check-readme-images.py` (run by `release-check.sh`) fails if README.md references anything else, a missing or non-PNG file, a picture not in `build-aux/wiki-pictures.txt`, or a picture without its light/dark pair. To change which pictures the README shows, edit the `<table>` in README.md.
Screenshot markup: a captioned picture is `<figure markdown="1">` + the light and the dark image + `<figcaption markdown="1">` (never an italic
line right under the images: both would share one paragraph and the caption runs on after the image); `check-wiki-pictures.py` fails on the
old form. The CSS in `wiki/docs/assets/extra.css` centres the figure and its caption and shows one variant. `check-wiki-centred.py` (run by
`wiki.sh`) fails when the window of a picture is more than 2 px off-centre; the crops of `wshot` keep equal padding on both sides
(`scenarios_wiki.centred_box`, tested by `build-aux/session/selftest.py`). The comic pictures show a real public-domain comic when it can be
downloaded (`build-aux/make-real-comic.py`, `docs/test-books.md`).
`.github/workflows/ci.yml` runs the same script on pull requests and deploys on pushes to `main`.

## Devices, sharing over HTTPS

* `BOOKX_DEVICE_ROOTS` (colon separated) replaces `/proc/mounts` as the place devices are looked for; `BOOKX_MTP_ROOT` is the folder
  holding gvfs-style `mtp:host=…/<storage>` directories (MTP is off when only `BOOKX_DEVICE_ROOTS` is set).
* Dev hooks: `library:ctxmenu` (the labels of the book menu), `library:send=<text in the device path>` (uses the saved
  format/template/shelf), `library:import-highlights=<path;path>` (read + plan like Import Highlights…, prints `import_summary:`/`import_plan:`), `library:import-confirm` (apply the review), `library:export-all=<fmt>:<folder>` (Export All Annotations), `library:annotate-more` (more highlights: a green one, a PDF-style one, one in the second book), `library:annotate-real` (highlights whose text is really in the first book: one made in Bookx, three left by a KOReader sync with only an xpointer in the ledger, one not in the book; open with `library:notes:search=…` + `open-first`, then `reader:hl-where` prints `hl_where: spine= page= start= end= visible= selected=`; for a bookmark `reader:bm-where` prints `bm_where: spine= page= frac=`; `reader:side-state` lists `side_bookmark:` rows), `library:notes` (opens Notes & Highlights; it now lists the bookmarks too, `library:annotate` gives the first book one) then `library:notes:kind=all|highlights|notes|bookmarks|kinds` (the type buttons; `notes_kinds:`; `notes_state:` ends with `bookmarks=N kind=K`)`|search=|checkgeom|hover=<n>|group=book|month|color|tag|none|color=<name>|book=<id>|hasnote=yes|no|any|from=<YYYY-MM-DD>|select-all|select-first|clear|open-first|edit-first=<note>|delete-first|export=<id>:<path>|token=<readwise token>|readwise|dump|close` (prints `notes_state:`/`notes_edit:`/`notes_export_result:`/`readwise:`; wait ~400 ms after `search=`: the entry debounces), `reader:tts-save:chapter:<file.wav>|book:<folder>` (Save as audio without the chooser; prints `tts_save:`), `kosync:setup=<url>;<user>;<password>[;filename]|register|login|continue|stay|flush|queue-flush|queue-dump|dump` (reading position sync, offline queue `kosync_queue:`/`kosync_queue_flush:`; prints `kosync_pull:`/`kosync_prompt:`/`kosync_push:`; `build-aux/kosync-peer.py` is the second client of the `kosync_client*` scenarios), `koannot:start|device=<json file>|merge|dump (prints `koannot_hl:`, `koannot_bm:` for every bookmark with its origin and `approx=`)|pull=<book id>|addhl=<id>;<text>;<note>|editnote=<id>;<text>;<note>|delhl=<id>;<text>|plugin=<dir>|plugins-folder=<mount>|push=<book>\|<text>\|<colour>[\|<note>[\|<device>]]|pushdel=<book>\|<text>|progress=<book>\|<0..1>\|<device>|events` (live updates: `push`/`pushdel`/`progress` are real HTTP PUTs to the running server and nothing else, the app merges and the open readers repaint by themselves; `events` prints the `SyncEvent`s delivered; the readers answer `reader:live-state` (`live_state:` highlights/notes/bookmarks/colours, painted looks, events heard, repaints, banner, open note editor, sidebar summary), `reader:live-edit=<n>` (opens the sidebar note dialog of the nth highlight), `reader:live-type=<text>`, `reader:live-press=primary|secondary` (a button of the sync banner); scenarios `koreader_annotations_live_*`) (KOReader highlight/note sync with the built-in server and an in-process fake KOReader client; scenarios `koreader_annotations*` in `build-aux/ui-tests-koannot.sh`; the Lua plugin tests are `koreader-plugin/tests/run.sh`, see `docs/koreader-annotation-sync.md`), `devwin:show|dump|reload|device=<n or name part>|select=<i,j>|remove|sync|format=<id>|template=<text>|shelf=<name>|dav=<url>|view=grid|list|notes=on|off|autosync=on|off|syncask|syncconfirm|send|close`
  for the Devices window (more hooks for the Devices button, banner and panels: `docs/device-ui-notes.md`), `share:https`, `share:httpsown`, `share:httpsfiles:<cert>:<key>`, `share:gen`, `share:fetchtls:<path>[:untrusted]` for HTTPS sharing.
  Wave 7 hooks: `devwin:page=books|settings` switches the two pages; `devwin:layout` prints the dialog and parent heights
  and the current page for `device_fit`; `library:device-banner=dismiss` is the canonical banner hook (old `always|later`
  aliases only dismiss and never enable auto-sync). The library dump prints `device_banner: widget=AdwBanner … dismiss=true`
  and `auto_sync` in `details_auto`.
* Scenarios: `device_none`, `device_button`, `device_ui`, `device_sync_ui`, `device_auto_sync`, `details_panel`, `details_narrow`, `device_window`, `device_options`, `device_window_hig` (Books/Settings pages, labels and selection controls), `device_fit` (content under 560 px and at most 85 % of the parent at 900x600), `backup_scope_explanation` (`library:health=backup-scope`, what a backup contains), `sharing_https`, `sharing_https_files`. Real-session accessibility: `device_window_a11y` and `device_banner_a11y`. `fake_devices` in
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
| `report:<cmd>` | Report a Problem (`src/ui/report_dialog.rs`, `report_notices.rs`; list in `build-aux/ui-tests-report.sh`): `open`, `type=`, `log=on\|off`, `library=on\|off`, `preview`, `dump`, `copy`, `save=PATH`, `github` (prints `report_open: URL`, nothing is opened under a dev script), `log-line=`, `crash-sim=`, `notices`, `recheck`, `dismiss-beta`, `dismiss-crash`, `report-from-*`. Env: `BOOKX_TEST_VERSION` replaces the version (beta notice tests), `XDG_STATE_HOME` holds `bookx/bookx.log` and `crash-*.txt` |
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
Flatpak, `make desktop-check` validates the desktop file, metainfo and MIME definitions. Distribution channels (Flathub first, Homebrew for
macOS, winget/Scoop for Windows) are planned in `docs/flathub-submission.md`, `docs/macos-port.md` and `docs/windows-port.md`; the files are in
`packaging/` (`flathub`, `homebrew`, `winget`, `scoop`, `msys2`), the manual workflows are `.github/workflows/package-*.yml` and the bundle scripts
are `build-aux/macos-bundle.sh` and `build-aux/windows-bundle.sh` (none of the macOS/Windows parts has been run). `build-aux/pin-screenshots.sh vX.Y.Z`
points the metainfo screenshots at a tag before releasing.

PDF reader (`src/ui/reader_pdf.rs`, `reader:` commands, only while a PDF is open): `pdf-state` (prints `pdf_state:` with page, zoom, mode, scale, rendered/texts counts, scroll, selection, highlights, links, hits), `pdf-goto=N|LABEL`, `pdf-zoom=width|page|PERCENT`, `pdf-zoom-in|out`, `pdf-view=single|continuous`, `pdf-crop=on|off`, `pdf-invert=on|off`, `pdf-scroll-by=PX`, `pdf-turn[=back]`, `pdf-select=TEXT` (selects the first match from the current page and opens the popover), `pdf-highlight[=COLOR]`, `pdf-style=NAME`, `pdf-note=TEXT`, `pdf-copy`, `pdf-dict`, `pdf-clear`, `pdf-highlights` (prints what is stored), `pdf-search=Q` / `pdf-search-next|prev` / `pdf-search-state`, `pdf-bookmark`, `pdf-tab=pages|toc|bookmarks|notes`, `pdf-sidebar`, `toc-row=N`, `pdf-link=N` (follows the Nth link of the page; an address is printed as `pdf_link: uri …`, never opened), `pdf-back`, `pdf-password=PW`, `pdf-reflow`, `pdf-thumbs`, `pdf-lists`. Give a command a `wait:` before `close:` or `quit` or it may never run. Fixtures: `cargo run -p bookx-pdf --example make_test_pdfs DIR` (story, locked with password `sesame`, scan, margins, a 500 page `big.pdf`; `BIG=0` skips it); `bookx_pdf::testpdf` builds small PDFs in tests. Scenarios: `build-aux/ui-tests-pdf.sh` (Broadway) and `build-aux/session/scenarios_pdf.py` (real session). View settings live in `pdf.json` next to `config.json`.

### Editor preferences, shortcuts, snippets and marks (`src/ui/editor/prefs_page.rs`, `snippets.rs`, `keymap.rs`)

Settings of the code view live in `editor_prefs::EditorLook` (`editor-look.json`; own file so the other editor windows' saves never overwrite it); `EditorLook::update(|l| …)` saves, updates `editor_prefs::look()` and notifies `settings_bus::editor()`, which every editor window answers with `Input::Prefs(PrefsAct::Changed)`. The page registers itself with the Preferences registry (`prefs_page::register`, page id `editor`) and the same groups open in a dialog from the editor (palette command *Editor Preferences*). Every editor action is a row of `editor_keymap::COMMANDS` (registry group *Editor*; `ui/editor/keymap.rs` turns the id into an `Input` and rebuilds the shortcut controllers on every change). Hooks: `editor:prefs-set=KEY=VALUE` (fields of `EditorLook`), `prefs-dump` (prints `editor_prefs: tab=… spaces=… lines=… hl=… margin=… col=… scheme=… font_css=… colors=… wrap=… minimap=… ws=…`), `prefs-open`, `prefs-write-scheme=ID` (drops a user scheme into the config folder), `keys-set=ACTION=ACCEL` (`editor_keys: bound=true|false`), `keys-palette=TITLE` (what the editor palette shows as the key), `snippet-run=BEFORE|SELECTED` (puts the text in the open file, selects the second part, presses Tab; prints `editor_snippet: expanded=… text=…`), `snippets-export=PATH`, `snippets-import=PATH`, `template-save=LABEL|EXT|BODY`, `clear-marks`; search: `editor:search:mode=fuzzy`. Scenarios `editor_prefs_*`, `editor_keys_registry`, `editor_snippet_*`, `editor_fuzzy_mode`, `editor_fn_functions`, `editor_marks_append` (`build-aux/ui-tests-editor-prefs.sh`); real session `editor_rebind_real_keys`, `editor_snippet_tab`, `editor_prefs_page` (`build-aux/session/scenarios_editor_prefs.py`).

## Editor plugin and narration dev hooks

* `editor:tool=plugin:action:<plugin id>:<action id>` runs an editor-plugin action (`src/ui/editor/plugin_host.rs`); the example
  plugin is `examples/plugins/trim-whitespace` (scenarios `editor_plugin_trim`, `editor_plugin_off`; the harness copies it into
  `$d/plugins` for scenarios named `editor_plugin*`).
* `editor:tool=plugin:narration:add|start|remove` (`src/ui/editor/narration.rs`): `add` opens the confirmation, `start` runs the
  narration without it (with `BOOKX_TTS_MOCK` the voice is a silent tone), `remove` removes it. `editor:mo:files|check|total`
  print what the book has (`editor_mo: ...`). Scenarios `editor_narration`, `reader_narration`.
* Opt-in live test with a real voice: `BOOKX_TTS_LIVE=1 BOOKX_TTS_DIR=~/.cache/bookx-tts cargo test --release -p bookx narration::tests::live -- --nocapture`.

## Editor window hooks, wave 9 (`src/ui/editor/dev9.rs`, scenarios in `build-aux/ui-tests-editor8.sh`)

`editor:header-info` (`editor_header: buttons=N inside=yes|no win=WxH sidebar_btn=… preview_tip=…`), `editor:tools-menu` and `editor:book-menu` (`editor_menu: name= top=N labels=a|b dup=-|labels`), `editor:revert-state` (`editor_revert: undo= redo=`), `editor:layout` (`editor_layout: win= code_w= collapsed= minimap= preview_w= vertical= sidebar_btn=` and `editor_preview_head: buttons= preview_visible=`), `editor:resize=WxH`, `editor:find-open`, `editor:replace-open`, `editor:find-info` (`editor_find: height= open= replace=`), `editor:switcher-info`, `editor:file-labels`, `editor:banner-info|banner-action|save-failed=WHY|foreign-banner`, `editor:fonts-state`, `editor:dialog-info` (`editor_dialog: card_open= first_row= visible= ellipsized=`), `editor:dialog-buttons|dialog-close`, `editor:suff-toggle=SET:MODE|suff-add|suff-text`, `editor:links-ui|links-cancel`, `editor:narration-ui|narration-background`, `editor:check:panel` (`check_panel: problems= results= empty= controls=`), `editor:spell:info` (`editor_spell_ui: placeholder= detail=`), `editor:chars:size=WxH|layout`, `editor:snippets-form=NAME|TRIGGER|BODY` (`editor_snippet_form: trigger_error= save= problem=`).

## Editor tool dev hooks (`editor:x:<name>=<args>`)

Defined in `src/ui/editor/ext_dev.rs`; used by `build-aux/ui-tests-editor-tools.sh`. A comma in a text is written `§`. Examples: `x:smarten=all|smarten|de|qde`, `x:fixhtml=all|html5`, `x:styles=change|color|red|green;;scale|2`, `x:drop=<from>|<onto>|before`, `x:dialog=smarten` then `x:dialog-click=preview|apply`, `x:save-copy-edit=<path>`, `x:open-names=a.xhtml|text`, `x:open-with-export` then `until:editor-external-pending` and `x:external-pending` / `x:external-accept`. Environment: `BOOKX_FONT_DIRS`, `BOOKX_EDITOR_NET_MOCK`, `BOOKX_OPEN_WITH_PROGRAM` (absolute path of a stand-in for the other program; it gets the temporary file as its only argument).

## Editor structure tools (dev hooks)

`src/ui/editor/struct_hub.rs` (scenarios in `build-aux/ui-tests-editor-struct.sh`, real input in
`build-aux/session/scenarios_editor_struct.py`): `x:toc-drop=<from>|<target>|before|after|into`, `x:toc-op=up|down|in|out|delete|add`,
`x:toc-problems`, `x:drop-merge=<from>|<onto>`, `x:drop-merge-accept`, `x:cover=existing:<path>|file:<path>|gen[;crop=0..2][;size=0..3][;page=0|1][;old=0|1]`
(or `remove`), `x:cover-state`, `x:fonts-manage=list|rename=<from>|<to>|remove=<path>|sample=<path>[|<text>]`,
`x:html-rules=<kind>|<selector>|<fields>;;<next rule>[~~current|preview|<path>]`, `x:html-ruleset=save|<name>|<rules>`/`list`/`load|<name>`/`delete|<name>`,
`x:jacket=add[|cover=0…]|remove|refresh|set-title|<text>|state`, `x:a11y-form=read|suggest|validate|preset|<id>|set|summary=…|modes=a:b|…`,
`x:foreign-open=<path>`, `x:foreign-save=epub|<dest>` / `back` / `prompt`, `x:foreign-status`, `x:win-size=<w>x<h>`,
`x:animations=off`, `x:dialog-close`. `x:dialog=cover|fonts|transform-html|jacket|a11y-form` opens the dialogs.

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

Pain-point work (`build-aux/ui-tests-pain.sh`, `docs/pain-points-*.md`): `library:citations=open` or `write=STYLE|QUERY|FILE` (writes the bibliography as the window would, prints `citations: wrote N STYLE`); `library:health=verify|scan-covers|lookup-covers` (File integrity and Covers groups of Library Health; print `health_fixity:` and `health_covers: poor=N total=M`); `library:dupsapart` / `library:dupsvisible` (declare every duplicate group "not a duplicate", print how many groups are still shown); `prefs:privacy-offline=on|off` and `prefs:ruler-on|off` (the switches of Preferences); `reader:ruler=toggle|up|down|state` (prints `ruler: enabled= visible= lines= dim= band=TOP..BOTTOM view_h= …`). The scenario `pain_health` prepares a library whose book file was damaged without its date or size changing and whose cover is 150×220. CLI: `bookx-cli library cite|fixity|covers`. Real session: `reading_ruler_keys` (Alt+Down / Alt+Up with real keys).

Audiobooks (`build-aux/ui-tests-audiobook.sh`, `docs/audiobook-notes.md`): `audio:add=PATH` imports an audio file, folder or zip (prints `audio: added id=N`), `audio:open=ID` opens the player, `audio:close` goes back, `audio:stats` prints the listening numbers (`audio_stats:`), and with the player open `audio:play|pause|toggle|skip=SECONDS|seek=MS|chapter=N|next|prev|speed=X|volume=X|sleep=MINUTES|sleep=chapter|sleep-secs=N|bookmark=NOTE|gobookmark=N|rmbookmark=N|tick` and `audio:dump` (one `audio: state=… pos=… chapter=i/n speed=… sleep=… bookmarks=… total=… mpris=… progress="…" error=…` line); `library:audiobadges` prints the headphones badges on the covers and whether the sidebar has its Audiobooks row. Environment: `BOOKX_AUDIO_SINK=null[:N]` (plays nothing, takes `1/N` of the time; `null:0` is instant; the scenarios use `null:20`), `BOOKX_AUDIO_PLAYER` (an absolute program path plus arguments, run directly without a shell, fed with raw s16le samples, `{rate}` and `{channels}` replaced), `BOOKX_NO_MPRIS=1`. Real session: `build-aux/session/scenarios_audiobook.py` (`audiobook_keys`, `audiobook_mpris`, `audiobook_dark_small`). Test audio: the crate's `testgen` (silent or sine WAV/FLAC, MP4 atoms) and `crates/bookx-audio/tests/fixtures/` (tiny FFmpeg-made files).

Comic reader (`build-aux/ui-tests-comics.sh`, `docs/comic-notes.md`): `reader:comic-state` prints `comic_state:` (pages, page, screen, mode, fit, rtl, guided, grey, zoom, fitted, centre `cx cy` in picture pixels, `shows=x0:y0:x1:y1` = the part of the first page of the screen that is in the window, `stop` = current guided stop, `panels` of the page, cache size, `animating`, `draws`, sidebar and thumbnails decoded); `reader:comic-goto=N` (1-based page), `comic-mode=single|spread|webtoon`, `comic-fit=page|width|height|original`, `comic-rtl=on|off`, `comic-guided=on|off`, `comic-grey=on|off`, `comic-cover-alone=on|off`, `comic-wide-alone=on|off`, `comic-reset` (the defaults for this book), `comic-key=[shift+|ctrl+]NAME` (a key through the same mapping as the keyboard: `Right`, `space`, `Page_Down`, `plus`, `g` …), `comic-click=FX:FY` and `comic-dblclick=FX:FY` (fractions of the window), `comic-dblclick-page=FX:FY` (fractions of the first page on screen), `comic-wheel=DY` / `comic-wheel=ctrl:DY`, `comic-drag=DX:DY` (pixels, press, move, release), `comic-thumb=N` (activates the row of a page in the strip), `comic-sidebar=on|off`, `comic-animate=on|off` (this session only; Broadway's frame clock is slow, so switch animations off), `comic-fullscreen`. The probe `until:comic` waits for the first pages. `prefs:comic-layout|fit|cover|wide|guided|grey|animate=…` and `prefs:comic-dump` drive Preferences → Reading → Comics. `cargo run -p bookx-comic --example make_test_comics DIR` writes the generated comics.

Highlight colours and styles through sync (`docs/highlight-sync-rca.md`): the one table is `bookx_core::devstyle` (KOReader `color`/`drawer`, Kobo `Color`, Bookx words, unknown words kept verbatim); the repair of highlights stored with the default look is `bookx_serve::look_repair` (library, no GTK), run once on library open by `highlight_repair::look_pass` (marker `.bookx/highlight-looks-v1`) and by hand with `bookx-cli highlights repair [--from PATH]… [--config-dir DIR] [--apply] [--json]` (dry run by default; `library undo` takes it back). Tests: `crates/bookx-core/src/devstyle.rs`, `crates/bookx-serve/tests/{highlight_looks,look_repair}.rs` (fixtures `tests/fixtures/koreader_all_looks.lua`, `koreader_legacy_highlights.lua`), `koreader_annotations.rs::a_real_shaped_sidecar_keeps_every_look_over_the_wire` (plugin replay `koreader-plugin/tests/replay_sidecar.lua`), `crates/bookx-cli/tests/highlights.rs`, and the end-to-end script `build-aux/ui-tests-hlsync.sh` (CLI only, no Broadway; standalone, add it to CI next to `ui-tests.sh`).

KOReader plugin tests (`koreader-plugin/`): `koreader-plugin/tests/run.sh` (Lua tests on `luajit` and `lua`; `scenarios.lua` on the fake KOReader `fake_koreader.lua`, `run.lua`, `main_load.lua`), `build-aux/plugin-tests.sh` (also the Rust live test `koreader_annotations::the_real_plugin_registers_and_syncs_against_a_real_server`, skipped with `NET=0` or without lua/curl), `build-aux/package-plugin.sh` (`dist/bookx.koplugin.zip`). How to add a scenario, how to read the device log and how to reinstall the plugin: `docs/koreader-plugin-testing.md`. Server side: `GET /bookx/info` (version check, no login).

Wave 11 hooks (formats, comics, audiobooks, conversion): `reader:comic-volume=next|prev` opens the neighbouring volume, `reader:comic-volumes` prints `comic_volumes: prev=TITLE|- next=TITLE|-`, `comic_state:` has `book=ID`, `prefs:comic-library-rtl|ignore-info=on|off` (and `comic_prefs:` prints them); `audio:link=BOOK:PATH`, `audio:continue-listening=BOOK`, `audio:continue-reading=BOOK` (one place for an e-book and its audiobook); `convert:open|format=epub|kepub|mobi|azw3|fb2|smarten|straighten|unused-css|blank|margins|split|inline-toc=on|off|version=auto|2|3|parallel=N|reset|close` drive Conversion Options and print `convert_options:` (scenario `convert_options` in `ui-tests-lib8.sh`). `cargo run -p bookx-comic --example make_test_comics DIR` also writes `DIR/saga/Saga v01.cbz` and `v02.cbz` (a series by folder name). RAR and DjVu tests skip themselves when `unrar`/`7z`/`ddjvu` are missing; the fuzz target `helpers` covers what comes back from them.

## Dev hooks of the library track (T3)

Used by `build-aux/ui-tests-lib8.sh` (`lib12_*`), see also `docs/library-t3-notes.md`.

| Command | Does |
|---|---|
| `library:customcol=add\|Name\|type\|arg` | adds a custom column (`arg`: enumeration values separated by `/`, or a composite template) |
| `library:customcol=set\|ID\|label\|value`, `undo`, `remove\|label` | sets a value, undoes the last edit, removes a column; prints `customcol: …` |
| `library:customtable=label:on\|off`, `library:customsort=label:asc\|desc`, `library:customcols` | ticks a table column, clicks its header, prints `customcols: defined=N table=[label:visible] sort=…` |
| `library:record=empty\|Title\|Authors`, `record=isbn\|ISBN` | adds an empty record / an ISBN record |
| `library:replace=field\|find\|with[\|regex]` | search and replace over the whole library (`replace: changed=N skipped=N`) |
| `library:marked=1,2`, `library:mark`, `library:clearmarks`, `library:quickview`, `library:replace` | marks, the Mark action, clear marks, open the related books window, open the replace dialog |
| `library:note=set\|kind\|name\|text`, `note=get\|kind\|name` | notes on authors, series, tags, publishers |
| `library:tagtree=on\|off\|rows` | the tag tree (`tagtree: on=B rows=[…] widgets=N`) |
| `library:inboxwatch=on\|off\|check\|FOLDER` | the import folder watcher |
| `library:indexstatus[=rebuild]` | the text index row |
| `library:shelf=send=NAME` | Send to Device of a shelf (prints the ids) |

## Wave 11 hooks: sync, secrets, settings, plugins, safe mode (checked 2026-11-22)

| Command | Does |
|---|---|
| `koannot:start` | starts the in-process sync server on a random port (`koannot_started: URL`); the session scenarios read the URL from `app.log` and talk to the real `/bookx` endpoints with their own HTTP client (`build-aux/session/scenarios_wave11.py`, class `Device`) |
| `koannot:push=BOOK\|TEXT\|COLOUR[\|NOTE[\|DEVICE]]`, `koannot:pushdel=BOOK\|TEXT`, `koannot:progress=BOOK\|0..1\|DEVICE` | a device sends, deletes a highlight or stores a position, through the real HTTP server (`koannot_push:`, `koannot_progress:`); `koannot:events` prints the events that reached the readers |
| `history:open[=BOOK]`, `history:toast`, `history:toast-undo`, `history:count`, `history:state`, `history:filter=all\|deleted\|edited`, `history:search=TEXT`, `history:restore-all`, `history:select`, `history:restore-selected`, `history:clear`, `history:close` | the annotation History dialog (`ui/annotation_history.rs`): `history_toast:` (the sync toast text and entry count), `history_state:` (groups, entries, restorable, select, page, filter), `history_first:` (newest entry shown: action, kind, source, text), `history_count:`; scenario `annotation_history_restore_after_device_delete` in `build-aux/ui-tests-koannot.sh` |
| `reader:live-state`, `reader:live-edit=N`, `reader:live-type=TEXT`, `reader:live-press=primary\|secondary` | what the open reader holds and paints (`live_state:`), edits a note, presses a banner button |
| `secrets:set=KEY:VALUE`, `create=PW`, `unlock=PW`, `relock`, `decline`, `reset`, `state`, `caption`, `has=KEY:VALUE`, `show-create`, `show-unlock` | the master-password store; `show-*` open the real dialogs (their toast goes to the log as `secrets_toast:`). Dialogs are quiet while a dev script runs unless `BOOKX_SECRETS_PROMPT=1`; with that set **and** a script, the watcher and `show-unlock` open two dialogs |
| `settings:export=PATH`, `settings:import=PATH`, `settings:import-apply`, `settings:get=KEY` | Settings Backup without the (portal) file chooser; `import` opens the real preview dialog (`settings_io:`) |
| `plugins:sheet=ID`, `plugins:sheet-allow`, `plugins:state` | the plugin capability sheet (`plugin_sheet:`, `plugin_trust:`) |
| `prefs:accounts-open\|close\|new\|edit=NAME\|remove=NAME\|undo\|fill=NAME;PW;REPEAT;tag+tag\|save\|guests=tag+tag\|verify=NAME;PW\|dump` | the Library server accounts window of Preferences → Sync → Advanced (`prefs_accounts:`, `prefs_accounts_verify:`); never prints a password or hash |
| `prefs:hardcover-set=TOKEN\|test\|import\|overwrite=on\|apply\|cancel\|undo\|dump` | Preferences → Library → Hardcover (`prefs_hardcover:`); the service answers from `hardcover.json` of `BOOKX_WEB_MOCK`; never prints the token |
| `news:recipes=open;KEY\|save\|feed;N;TITLE\|URL\|MODE\|MAX\|ON\|filters;…\|issue;…\|dump` and `sites\|site-new\|site-fill;HOST;USER;PW\|site-save\|site-remove;HOST\|site-undo\|sites-dump` | the Edit Feeds view and the site accounts of the News Subscriptions window (`news_recipes:`, `news_sites:`); never prints a password |
| `lookup:settings=up;TITLE\|down;TITLE\|web-new\|web-fill;NAME;URL\|web-save\|web-remove;NAME\|web-undo\|term;WORD\|web-open;NAME\|history-show\|history-clear\|history-undo\|keep;on\|off\|dump` | Preferences → Dictionaries: order, web sites, history (`lookup_settings:`, `lookup_web:`) |
| `safe:dump` | `safe_mode: active=B plugins_dir=P plugins_found=N offline=B`; `BOOKX_SAFE_MODE=1` or `--safe-mode` starts in safe mode |
| `launch:write=PATH\|TEXT`, `launch:spawn=ARGS` | writes a file / starts a second `bookx` the way a launcher does (`launch_spawn:`) |

`app:text-scale=1.5` (scales the text like the accessibility setting) exists in `src/app.rs` and was missing from this file; `sync:prefs` named in `docs/remaining-work.md` is not a command in the tree (the Sync page hooks are `prefs:sync-req` and `prefs:pick=Sync`).
Session notes: the file chooser of a GTK app in the session sandbox is the portal's (`xdg-desktop-portal-gtk` appears as an AT-SPI
application without a window), so scenarios use the hooks above instead of driving a chooser.
