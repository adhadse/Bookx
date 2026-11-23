# Testing and installing the Bookx Sync plugin for KOReader

Internal notes. User text: `wiki/docs/koreader-sync.md`. The plugin is `koreader-plugin/bookx.koplugin/` (version in
`bookxsync.lua`, `M.VERSION`; protocol number `M.PROTOCOL`, answered by the server's `GET /bookx/info`).

Status: living | Last reviewed: 2026-11-23 | Owner: area agent, reviewed by @adhadse

## Run the tests

```bash
build-aux/plugin-tests.sh             # Lua tests on every interpreter found + the endpoint text check + the Rust live test
koreader-plugin/tests/run.sh          # only the Lua tests (LUA=luajit to pick one; skips with a message if none installed)
NET=0 build-aux/plugin-tests.sh       # without the localhost server test
BOOKX_KO_FRONTEND=<copy of KOReader's frontend/ and ffi/ folders> koreader-plugin/tests/run.sh
                                      # also runs main_load.lua with KOReader's real luasettings, dump, logger, gettext, menusorter
cargo test -p bookx-serve --test koreader_annotations   # real server on localhost + the real main.lua (live.lua, needs curl)
```

KOReader runs LuaJIT: `luajit` is the interpreter that counts; plain `lua` (5.4/5.5) is run too so nothing works by luck.
CI: job `koreader-plugin` in `.github/workflows/ci.yml`. The three test files are independent and run in parallel.

## What is where (`koreader-plugin/tests/`)

| File | What |
|---|---|
| `fake_koreader.lua` | the fake KOReader: `K.world{online=, device_model=, koreader_version=}` gives stand-ins for datastorage, luasettings, logger, gettext, infomessage, textviewer, multiinputdialog, widgetcontainer, uimanager, network manager, socketutil, device, util, version, ltn12 (real semantics) and `socket.http` / `ssl.https` (drains the request source, fills the sink, answers from `w.http.script(req)`). Widgets copy their constructor arguments onto the instance like KOReader's `Widget:new`, and `w:plugin()` builds the plugin as `ReaderUI` does (`dialog`, `view`, `ui`, `document`) and wraps `on*` handlers like the plugin loader's sandbox. Also `K.runner` (`T.test/check/eq/has/finish`) |
| `scenarios.lua` | the stories (register, test connection incl. version check, sync push/pull, every colour/drawer, legacy layout, a real Kobo sidecar excerpt, hostile text, huge payload, error paths, offline, first run, menu, diagnostics, the **regression of bug 25**) |
| `run.lua` | the pure logic (`bookxsync.lua`, JSON, MD5); with `BOOKX_URL`... it also talks to a real server through curl |
| `main_load.lua` | the older `main.lua` walk-through, optionally with KOReader's real modules |
| `live.lua` | the real `main.lua` against a real server (started by the Rust test) |
| `replay_sidecar.lua` | replays real `.sdr` files through the sync logic (Rust test `a_real_shaped_sidecar_keeps_every_look_over_the_wire`) |
| `fixtures/kobo_sidecar_excerpt.lua` | one entry of every colour/drawer combination of the owner's real sidecar (texts replaced) |

## Add a test

1. Open `scenarios.lua` and add a story:

```lua
T.test("sync survives a server that answers 500 twice", function()
    local w, p = setup({ hl(1) })                      -- world + plugin with one highlight, account saved
    local n = 0
    w.http.script = function(req)                      -- req.method, req.url, req.headers, req.body
        n = n + 1
        if n <= 2 then return 500, "oops" end          -- an HTTP answer; `return nil, "timeout"` is a socket error; error("x") throws
        return 200, '{"seq":1,"annotations":[]}'
    end
    p:syncNow(false); p:syncNow(false); p:syncNow(false)
    T.has(w:last_text(), "Sent 1", "the third try goes through")
    T.eq(w:log_of("err"), "", "no Lua error logged")   -- a plugin bug would show here with its traceback
end)
```

2. Run `koreader-plugin/tests/run.sh`. Useful helpers: `w:last_text()`, `w:texts()`, `w:log_of("err")`, `w.requests`, `w:menu_entry(p, "Test connection")`,
   `K.world().press(dialog, "Save")`, `w.online = false`, `w.files` (every settings file in memory).
3. A bug fix needs a story that fails without the fix; check with `git stash push -- koreader-plugin/bookx.koplugin` (only that folder), run, `git stash pop`.
4. Anything KOReader-specific the fake does not model yet (a new widget, a new module) goes into `fake_koreader.lua`: add it to `P[...]` and keep it as faithful as the real module's behaviour the plugin relies on.

Rule of thumb that came out of bug 25: a mock must hand the plugin the **same constructor arguments** KOReader does, and tests
must run the code path a user presses (Sync now), not only the pure functions.

## Reading the log on the device

KOReader writes `crash.log` in its own folder (Kobo: `.adds/koreader/crash.log`, i.e. `/mnt/onboard/.adds/koreader/crash.log`; also shown by
KOReader's *Settings → Developer options → Enable debug logging* with more detail). Plugin lines contain `Bookx Sync`:

```
ERROR Bookx Sync error in Bookx:syncNow: plugins/bookx.koplugin/main.lua:120: attempt to call method 'document' (a table value)
```

Since 0.3.0 that line is followed by the full Lua traceback. The log is overwritten at every KOReader start. Automatic runs log `Bookx Sync: waiting: Wi-Fi is off` when they could not send, and `Bookx Sync: true|false <message>` after every sync.
In the fake KOReader a scheduled job (the 30 s debounce) waits until a test calls `w:run_scheduled()`.

## Update the plugin on the Kobo (owner)

1. On the computer: `build-aux/package-plugin.sh` (runs the tests, writes `dist/bookx.koplugin.zip`), or in Bookx *Preferences → Sync → Copy to the device* with the Kobo mounted.
2. Mounted Kobo: the plugin is the folder `.adds/koreader/plugins/bookx.koplugin` (the drive is mounted at e.g. `/run/media/<user>/KOBOeReader`).
   Delete that old folder, unzip the zip so that `bookx.koplugin/` (with `main.lua`, `bookxsync.lua`, `bookxjson.lua`, `bookxmd5.lua`, `_meta.lua`, `LICENSE`) is in `.adds/koreader/plugins/`.
   Keep `.adds/koreader/settings/bookx_sync.lua` (server, user, hash) as it is.
3. Eject the Kobo safely. KOReader restarts when the Kobo leaves USB mode; if it was running, use *top menu → Exit → Restart* (or *Exit*, then start KOReader again from Nickel).
4. Open a book → top menu → *More tools* → *Bookx Sync* → **About / Diagnostics** must say `Bookx Sync 0.3.1`; then *Test connection* and *Sync now*.
5. If something fails: *About / Diagnostics → Copy diagnostics* writes `.adds/koreader/settings/bookx_diagnostics.txt`; send that file and `.adds/koreader/crash.log`.
