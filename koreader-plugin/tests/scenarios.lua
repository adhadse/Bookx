-- Scenario tests of Bookx Sync: the plugin's real code paths (main.lua + bookxsync.lua) against a fake KOReader
-- (fake_koreader.lua), with a scripted server. One `T.test` is one story; each builds its own world.
-- Run: koreader-plugin/tests/run.sh   (see docs/koreader-plugin-testing.md to add one)
local here = (arg and arg[0] or "scenarios.lua"):match("^(.*)/[^/]*$") or "."
local K = dofile(here .. "/fake_koreader.lua")
local T = K.runner("scenarios")
local json_path = K.plugin_dir .. "/?.lua;"
package.path = json_path .. package.path
local json = require("bookxjson")
local Sync = require("bookxsync")

local KEY = "5ebe2294ecd0e0f08eab7690d2a6ee69"
local OK_PUT = '{"document":"d","seq":4,"annotations":[]}'

local function hl(n, extra)
    local it = {
        chapter = "Chapter " .. n, color = "yellow", datetime = string.format("2026-08-17 11:00:%02d", n % 60),
        drawer = "lighten", page = "/body/p[" .. n .. "]/text().0", pageno = n,
        pos0 = "/body/p[" .. n .. "]/text().0", pos1 = "/body/p[" .. n .. "]/text().40",
        text = "Quote number " .. n,
    }
    for k, v in pairs(extra or {}) do it[k] = v end
    return it
end

local function with_annotations(list, o)
    o = o or {}
    o.annotations = list
    return o
end

local function setup(list, o)
    local w = K.world(o)
    local p = w:plugin(with_annotations(list or { hl(1) }, o))
    w:configure(p)
    return w, p
end

-- ===== the owner's crash =====
T.test("regression: KOReader passes `document` to the plugin and it shadows a method named document", function()
    local w, p = setup({ hl(1) })
    -- real KOReader: Bookx:new{ dialog=..., view=..., ui=..., document=<table> }
    T.check(type(p.document) == "table", "the instance holds the document table, as in ReaderUI")
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(false)
    T.eq(#w.requests, 1, "Sync now sends one request")
    T.check(not w:last_text():find("unexpected error", 1, true), "no 'unexpected error': " .. w:last_text())
    T.eq(w:log_of("err"), "", "nothing in the error log")
    T.has(w:last_text(), "Sent 1", "the summary is shown")
end)

T.test("no method of the plugin can be shadowed by a field KOReader (or a widget) puts on the instance", function()
    local w = K.world()
    local p = w:plugin()
    local fields = { "dialog", "view", "ui", "document", "name", "path", "fullname", "is_doc_only", "settings",
        "title", "width", "height", "dimen", "left_text", "item_table", "buttons" }
    for key in pairs(w.Bookx) do
        if type(w.Bookx[key]) == "function" and key:sub(1, 2) ~= "on" and key ~= "init" and key ~= "extend" and key ~= "new" then
            for _, f in ipairs({ "dialog", "view", "ui", "document" }) do
                T.check(key ~= f, "method '" .. key .. "' has the name of the constructor argument '" .. f .. "'")
            end
        end
    end
    -- and extra constructor arguments never break syncing either
    local w2 = K.world()
    local p2 = w2:plugin(with_annotations({ hl(1) }, { extra_args = { settings = nil, name = "bookx", path = "/x" } }))
    w2:configure(p2)
    w2.http.script = function() return 200, OK_PUT end
    p2:syncNow(true)
    T.eq(#w2.requests, 1, "synced")
    local _ = fields
end)

T.test("an unexpected Lua error: short message on screen, full traceback in the log and the diagnostics", function()
    local w, p = setup({ hl(1) })
    p.settings = setmetatable({}, { __index = function() error("boom from settings") end })
    p:syncNow(false)
    local text = w:last_text()
    T.has(text, "unexpected error", "short message says so")
    T.has(text, "boom from settings", "it carries the actual Lua message, not only generic text")
    T.has(text, "Diagnostics", "it points to the Diagnostics entry")
    T.check(#text < 300, "and stays short (" .. #text .. ")")
    local log = w:log_of("err")
    T.has(log, "boom from settings", "the log has the Lua error")
    T.has(log, "stack traceback", "the log has the traceback")
    T.has(log, "Bookx:syncNow", "the log says where")
    T.check(p.state.last_error ~= nil and p.state.last_error.traceback:find("stack traceback", 1, true), "the error is remembered")
end)

T.test("an error in an automatic hook is logged but never shown while reading", function()
    local w, p = setup({ hl(1) })
    p.settings:saveSetting("auto", true)
    w.http.script = function() error("socket library exploded") end
    p:onSuspend()
    T.eq(#w.shown, 0, "nothing is shown for an automatic sync")
    p.settings = setmetatable({}, { __index = function() error("settings exploded") end })
    p:onCloseDocument()
    T.eq(#w.shown, 0, "nothing is shown when the hook itself fails")
    T.has(w:log_of("err"), "settings exploded", "but it is logged")
end)

-- ===== connection =====
T.test("test connection: success, wrong password, 404, server down, timeout, bad host", function()
    local w, p = setup()
    w.http.script = function(r)
        if r.url:match("/info$") then return 200, '{"name":"bookx","protocol":1,"min_plugin_protocol":1,"annotations":true}' end
        return 200, '{"authorized":"OK"}'
    end
    p:testConnection()
    T.eq(w:last_text(), "Connected as ann", "connected")
    T.eq(w.requests[1].headers["x-auth-key"], KEY, "the hash is sent")
    T.eq(w.requests[1].url, "http://192.168.1.5:8080/bookx/users/auth", "auth url")
    local cases = {
        { function() return 401, "{}" end, "Wrong user name or password" },
        { function() return 404, "{}" end, "not enabled at that address" },
        { function() return 429, "{}" end, "Too many requests" },
        { function() return 500, "oops" end, "server failed (500)" },
        { function() return nil, "connection refused" end, "Cannot reach 192.168.1.5:8080 (connection refused)" },
        { function() return nil, "timeout" end, "did not answer in time" },
        { function() return nil, "host or service not provided, or not known" end, "(unknown host)" },
        { function() return nil, "closed" end, "(connection closed)" },
        { function() error("attempt to index a nil value") end, "Cannot reach" },
    }
    for i, c in ipairs(cases) do
        w.http.script = c[1]
        p:testConnection()
        T.has(w:last_text(), c[2], "case " .. i)
        T.check(not w:last_text():find("unexpected error", 1, true), "case " .. i .. " is not 'unexpected error'")
    end
    T.eq(p.state.test.ok, false, "failed test recorded")
end)

T.test("test connection: version check says server too old / plugin too old, but older servers still work", function()
    local w, p = setup()
    local info
    w.http.script = function(r)
        if r.url:match("/info$") then return info() end
        return 200, "{}"
    end
    info = function() return 404, "{}" end
    p:testConnection(); T.eq(w:last_text(), "Connected as ann", "a Bookx without /info is fine")
    info = function() return 200, '{"protocol":0,"annotations":true}' end
    p:testConnection(); T.has(w:last_text(), "too old", "server too old")
    info = function() return 200, '{"protocol":9,"min_plugin_protocol":9,"annotations":true}' end
    p:testConnection(); T.has(w:last_text(), "newer Bookx Sync plugin", "plugin too old")
    info = function() return 200, '{"protocol":1,"min_plugin_protocol":1,"annotations":false}' end
    p:testConnection(); T.has(w:last_text(), "switched off", "annotation sync off on the server")
    info = function() return 200, "not json" end
    p:testConnection(); T.eq(w:last_text(), "Connected as ann", "garbage /info is ignored")
    info = function() return nil, "timeout" end
    p:testConnection(); T.eq(w:last_text(), "Connected as ann", "a failing /info does not fail the login")
end)

T.test("test connection without a server address or password", function()
    local w = K.world()
    local p = w:plugin(with_annotations({}))
    p:testConnection()
    T.has(w:last_text(), "first", "asks to set the account first")
    T.eq(#w.requests, 0, "no request")
end)

T.test("offline: Sync now waits for the network, automatic sync stays quiet", function()
    local w, p = setup({ hl(1) }, { online = false })
    p:syncWithNetwork(false)
    T.check(w.queued ~= nil, "queued for when the network is back")
    T.eq(#w.requests, 0, "nothing sent yet")
    p.settings:saveSetting("auto", true)
    p:onSuspend()
    T.eq(#w.requests, 0, "automatic sync offline sends nothing")
    T.eq(#w.shown, 0, "and says nothing")
    w.http.script = function() return 200, OK_PUT end
    w.queued()
    T.eq(#w.requests, 1, "queued sync runs when online")
end)

T.test("register account: ok, taken, closed, down", function()
    local w = K.world()
    local p = w:plugin(with_annotations({}))
    local function try(script)
        w.http.script = script
        w.shown = {}; w.requests = {}
        local d; p:editSettings(); d = w.shown[#w.shown]
        d.values = { "http://10.0.0.5:8080", "zoe", "secret", "Libra" }
        K.world().press(d, "Register account")
        return w:last_text()
    end
    local m = try(function(r) if r.method == "POST" then return 201, "{}" end return 200, "{}" end)
    T.eq(m, "Connected as zoe", "registered and tested")
    T.eq(w.requests[1].url, "http://10.0.0.5:8080/bookx/users/create", "create url gets /bookx")
    T.has(w.requests[1].body, KEY, "the md5 is posted")
    T.check(not w.requests[1].body:find("secret", 1, true), "never the password")
    T.has(try(function() return 402, "{}" end), "taken", "402")
    T.has(try(function() return 403, "{}" end), "closed", "403")
    T.has(try(function() return nil, "connection refused" end), "Cannot reach 10.0.0.5:8080", "refused")
    T.has(try(function() error("x") end), "Cannot reach", "throwing socket library")
end)

-- ===== progress / annotations: push =====
T.test("push: every colour and drawer of the fixtures goes to the server verbatim", function()
    local list = dofile(here .. "/../../crates/bookx-serve/tests/fixtures/koreader_all_looks.lua").annotations
    T.check(#list >= 36, "the fixture has the full colour x drawer table (" .. #list .. ")")
    local w, p = setup(list)
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(true)
    local sent = {}
    for _, r in ipairs(w.requests) do
        for _, a in ipairs(json.decode(r.body).annotations) do sent[#sent + 1] = a end
    end
    T.eq(#sent, #list, "every annotation sent")
    local looks = {}
    for _, a in ipairs(sent) do looks[a.color .. "/" .. a.drawer] = true end
    local want = {}
    for _, it in ipairs(list) do want[it.color .. "/" .. it.drawer] = true end
    for look in pairs(want) do T.check(looks[look], "look " .. look .. " arrived unchanged") end
    for _, a in ipairs(sent) do
        T.eq(a.kind, "highlight", "kind of " .. a.id)
        T.check(a.updated >= a.created, "times are ordered")
    end
end)

T.test("push: the legacy layout (pre-2024: look in `highlight`, text in `bookmarks`) merged by hand still syncs", function()
    local t = dofile(here .. "/../../crates/bookx-serve/tests/fixtures/koreader_legacy_highlights.lua")
    local items = {}
    for _, b in ipairs(t.bookmarks) do
        local look = t.highlight[b.page] and t.highlight[b.page][1] or {}
        items[#items + 1] = { text = b.notes, pos0 = b.pos0, pos1 = b.pos1, page = b.page, datetime = b.datetime,
            chapter = b.chapter, drawer = look.drawer }
    end
    local w, p = setup(items)
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(true)
    local sent = json.decode(w.requests[1].body)
    T.eq(#sent.annotations, 2, "both entries")
    local by_text = {}
    for _, a in ipairs(sent.annotations) do by_text[a.text ~= "" and a.text or "bookmark"] = a end
    T.eq(by_text["Quote about underscores"].drawer, "underscore", "the drawer survives")
    T.eq(by_text["Quote about underscores"].color, "", "no colour in old data: empty, not invented")
    T.eq(by_text["bookmark"].kind, "bookmark", "an entry without positions is a bookmark")
end)

T.test("push: real device data (sidecar of the owner's Kobo, copied as a fixture) syncs without errors", function()
    local path = here .. "/fixtures/kobo_sidecar_excerpt.lua"
    local f = io.open(path, "r")
    if not f then return end
    f:close()
    local t = dofile(path)
    local w, p = setup(t.annotations)
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(false)
    T.eq(w:log_of("err"), "", "no error logged")
    T.has(w:last_text(), "Sent " .. #t.annotations, "all sent")
end)

T.test("push: nil fields, odd types, non-UTF-8 text, control characters and huge text never produce a request the server refuses", function()
    local items = {
        { datetime = "2026-01-01 00:00:00", text = "ok", pos0 = "a", pos1 = "b" },
        { text = "no datetime", pos0 = "c", pos1 = "d" },
        { datetime = "garbage", text = "bad date", pos0 = "e", pos1 = "f", pageno = "12", page = {} },
        { datetime = "2026-01-02 00:00:00", text = "caf\233 latin-1 \255\254 bytes", pos0 = "g", pos1 = "h" },
        { datetime = "2026-01-03 00:00:00", text = "tab\tnew\nline\r\1ctrl\194\133c1", pos0 = "i", pos1 = "j",
            chapter = "Ch\nwith\tcontrol", note = "n\0ul" },
        { datetime = "2026-01-04 00:00:00", text = string.rep("x", 50000), note = string.rep("\195\169", 30000), pos0 = "k", pos1 = "l" },
        { datetime = "2026-01-05 00:00:00", text = "\1\2\3", pos0 = "m", pos1 = "n" },
        { datetime = "2026-01-06 00:00:00", text = 12345, note = true, pos0 = 1, pos1 = 2, color = 5, drawer = {} },
        "not a table", false,
        { datetime = "2999-01-01 00:00:00", text = "from the future", pos0 = "o", pos1 = "p" },
    }
    local w, p = setup(items)
    local seen
    w.http.script = function(r) seen = r.body; return 200, OK_PUT end
    p:syncNow(false)
    T.eq(w:log_of("err"), "", "no Lua error: " .. w:log_of("err"))
    T.check(seen ~= nil, "a request was made")
    local body = json.decode(seen)
    T.check(body ~= nil, "the body is valid JSON")
    for _, a in ipairs(body.annotations) do
        T.check(#a.text <= 20000 * 3, "text capped")
        local _, n = a.text:gsub("[%z\1-\8\11-\31\127]", "")
        T.eq(n, 0, "no control characters in the text of " .. a.id)
        T.check(not a.chapter:find("%c"), "no control characters in the chapter")
        T.check(a.kind ~= "highlight" or a.text:find("%S") ~= nil, "a highlight has visible text (server refuses otherwise)")
        T.check(a.updated <= os.time() and a.created <= os.time(), "no time in the future")
        T.check(select(2, a.text:gsub("\239\191\189", "")) >= 0, "replacement characters are fine")
    end
    -- everything the server checks: valid UTF-8 overall
    T.check(not seen:find("\255", 1, true) and not seen:find("\233 ", 1, true), "no raw invalid bytes on the wire")
end)

T.test("push: more annotations than one request holds are sent in several requests, under the size limit", function()
    local list = {}
    for i = 1, 1200 do list[i] = hl(i, { text = "q" .. i .. string.rep("w", 300), datetime = "2026-01-01 00:00:00", pos0 = "/p" .. i, pos1 = "/q" .. i }) end
    local w, p = setup(list)
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(true)
    T.check(#w.requests >= 3, "split (" .. #w.requests .. " requests)")
    local total = 0
    for _, r in ipairs(w.requests) do
        T.check(#r.body < 1000000, "a body stays under 1 MB")
        total = total + #json.decode(r.body).annotations
    end
    T.eq(total, 1200, "all arrive once")
    local big = {}
    for i = 1, 100 do big[i] = hl(i, { text = string.rep("a", 19000), pos0 = "/b" .. i, pos1 = "/c" .. i }) end
    local w2, p2 = setup(big)
    w2.http.script = function() return 200, OK_PUT end
    p2:syncNow(true)
    for _, r in ipairs(w2.requests) do T.check(#r.body < 1048576, "big quotes: a body stays under the server limit") end
    T.check(#w2.requests >= 2, "big quotes are split by size")
end)

T.test("push: unchanged annotations are not sent twice, a deleted one becomes a tombstone", function()
    local list = { hl(1), hl(2) }
    local w, p = setup(list)
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(true)
    T.eq(#json.decode(w.requests[1].body).annotations, 2, "first sync sends both")
    p:syncNow(true)
    T.eq(#json.decode(w.requests[2].body).annotations, 0, "second sync sends none")
    T.has(w.requests[2].url, "since=4", "and asks only for what is new")
    table.remove(w.ui.annotation.annotations, 2)
    p:syncNow(true)
    local last = json.decode(w.requests[3].body).annotations
    T.eq(#last, 1, "one tombstone"); T.eq(last[1].deleted, true, "marked deleted")
end)

T.test("push: a failing server leaves the state alone so the next run repeats the send", function()
    local w, p = setup({ hl(1) })
    local codes = { 500, 401, 413, 429, 404, 422 }
    for _, code in ipairs(codes) do
        w.http.script = function() return code, '{"message":"nope"}' end
        w.shown = {}
        p:syncNow(false)
        T.has(w:last_text(), "Bookx sync failed", "failure shown for " .. code)
        T.check(not w:last_text():find("unexpected error", 1, true), "not a Lua crash for " .. code)
    end
    T.eq(next(p.state.docs[string.rep("c", 32)].synced), nil, "nothing marked as synced")
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(false)
    T.has(w:last_text(), "Sent 1", "the repeat goes through")
end)

T.test("push: the answers of a broken server are failures with a message, never Lua errors", function()
    local bodies = { "", "not json", "[]", "{}", '{"annotations":"x"}', "\255\254", "null", "<html>proxy</html>" }
    for _, b in ipairs(bodies) do
        local w, p = setup({ hl(1) })
        w.http.script = function() return 200, b end
        p:syncNow(false)
        T.has(w:last_text(), "not understood", "answer " .. string.format("%q", b))
        T.eq(w:log_of("err"), "", "no Lua error for " .. string.format("%q", b))
    end
    local w, p = setup({ hl(1) })
    w.http.script = function() return nil, "connection refused" end
    p:syncNow(false)
    T.has(w:last_text(), "cannot reach 192.168.1.5:8080 (connection refused)", "refused is named")
    w.http.script = function() return nil, "timeout" end
    p:syncNow(false)
    T.has(w:last_text(), "timed out", "timeout is named")
    w.http.script = function() error("ssl handshake failed") end
    p:syncNow(false)
    T.has(w:last_text(), "cannot reach", "a throwing library is a network failure")
    T.eq(w:log_of("err"), "", "and not a plugin error")
end)

-- ===== pull =====
T.test("pull: notes made in Bookx arrive in the inbox, are not counted twice, and deletions remove them", function()
    local w, p = setup({ hl(1) })
    local answer = { document = "d", seq = 7, annotations = {
        { id = "bx-1", kind = "highlight", text = "From the computer", note = "hi", origin = "bookx", updated = 5 },
        { id = "ko-x", kind = "highlight", text = "own", origin = "koreader", updated = 5 },
        { id = "bx-2", kind = "highlight", text = "gone", origin = "bookx", updated = 6, deleted = true },
        { id = "bx-3", kind = "highlight", text = "caf\195\169 \226\128\156quoted\226\128\157", origin = "bookx", updated = 8 },
    } }
    w.http.script = function() return 200, json.encode(answer) end
    p:syncNow(false)
    T.has(w:last_text(), "received 2", "two notes received")
    w.shown = {}
    p:showInbox()
    T.has(w:last_text(), "From the computer", "inbox shows the note")
    T.has(w:last_text(), "hi", "with its comment")
    p:syncNow(false)
    T.has(w:last_text(), "received 0", "not counted twice")
    answer.annotations = { { id = "bx-1", origin = "bookx", deleted = true, updated = 9 } }
    p:syncNow(false)
    w.shown = {}; p:showInbox()
    T.check(not w:last_text():find("From the computer", 1, true), "a deletion removes it")
    T.has(w.requests[#w.requests].url, "since=7", "the cursor moved to the server's seq")
end)

T.test("pull: nil and odd fields in the server's answer do not crash the inbox", function()
    local w, p = setup({})
    w.http.script = function()
        return 200, '{"seq":"x","annotations":[null,1,"s",{"id":5},{"id":"bx-9","origin":"bookx"},{"id":"bx-8","origin":"bookx","text":null,"note":null,"updated":null}]}'
    end
    p:syncNow(false)
    T.eq(w:log_of("err"), "", "no Lua error: " .. w:log_of("err"))
    w.shown = {}
    p:showInbox()
    T.check(#w.shown == 1, "the inbox opens")
end)

-- ===== settings and first run =====
T.test("first run: no settings at all, then Save keeps only the hash", function()
    local w = K.world()
    local p = w:plugin(with_annotations({ hl(1) }))
    T.check(p.settings:readSetting("device_id") ~= nil, "device id made")
    p:syncNow(false)
    T.has(w:last_text(), "Set the Bookx server", "not configured says so")
    local entry = w:menu_entry(p, "Server and account")
    entry.callback()
    local d = w.shown[#w.shown]
    d.values = { "http://192.168.1.5:8080/bookx", "ann", "secret", "" }
    K.world().press(d, "Save")
    T.eq(p.settings:readSetting("key"), KEY, "key is the md5")
    for k, v in pairs(w.files) do
        for kk, vv in pairs(v) do T.check(tostring(vv) ~= "secret", "no plaintext password in " .. tostring(kk)) end
    end
    T.check(p:configured(), "configured now")
end)

T.test("menu: all entries work and none can throw into KOReader", function()
    local w, p = setup({ hl(1) })
    local items = {}
    p:addToMainMenu(items)
    T.eq(items.bookx.sorting_hint, "more_tools", "hangs under More tools")
    w.http.script = function(r) if r.url:match("/info$") then return 404, "{}" end return 200, OK_PUT end
    for _, e in ipairs(items.bookx.sub_item_table) do
        if e.text_func then T.check(type(e.text_func()) == "string", "status line is text") end
        if e.enabled_func then T.eq(e.enabled_func(), true, "enabled with a book open") end
        if e.callback then e.callback() end
        if e.checked_func then e.checked_func() end
    end
    T.eq(w:log_of("err"), "", "no error logged: " .. w:log_of("err"))
end)

T.test("About / Diagnostics: shows version, server and last error; Copy diagnostics writes a file", function()
    local w, p = setup({ hl(1) })
    p.settings = setmetatable({}, { __index = function() error("first failure") end })
    p:syncNow(false)
    p.settings = nil
    local w2, p2 = setup({ hl(1) })
    p2.state.last_error = { time = os.time(), where = "Bookx:syncNow", message = "main.lua:1: boom", traceback = "stack traceback:\n\tmain.lua:1" }
    local entry = w2:menu_entry(p2, "About / Diagnostics")
    T.check(entry ~= nil, "the menu has the entry")
    entry.callback()
    local viewer = w2.shown[#w2.shown]
    T.has(viewer.text, "Bookx Sync " .. Sync.VERSION, "version")
    T.has(viewer.text, "http://192.168.1.5:8080/bookx", "server url")
    T.has(viewer.text, "main.lua:1: boom", "last error")
    T.has(viewer.text, "stack traceback", "with traceback")
    T.has(viewer.text, "v2026.07.2-test", "KOReader version")
    T.check(not viewer.text:find(KEY, 1, true), "never the key")
    T.check(viewer.buttons_table ~= nil, "has a Copy diagnostics button")
    viewer.buttons_table[1][1].callback()
    local f = io.open(w2.dir .. "/bookx_diagnostics.txt", "r")
    T.check(f ~= nil, "the file is in the settings dir")
    if f then T.has(f:read("*a"), "Bookx Sync " .. Sync.VERSION, "file content"); f:close() end
    T.has(w2:last_text(), "bookx_diagnostics.txt", "tells where it is")
    local _ = w
end)

T.test("pure helpers: errors, status text, utf-8 cleaning", function()
    T.eq(Sync.network_reason("connection refused"), "connection refused", "refused")
    T.eq(Sync.network_reason(nil), "no answer", "nil error")
    T.eq(Sync.unreachable("http://a:1/bookx", "timeout"), "cannot reach a:1 (timed out)", "unreachable")
    T.eq(Sync.short_error("one\ntwo"), "one", "first line")
    T.check(#Sync.short_error(string.rep("x", 500)) <= 140, "short")
    T.eq(json.clean_utf8("a\255b"), "a\239\191\189b", "invalid byte replaced")
    T.eq(json.clean_utf8("\195"), "\239\191\189", "truncated sequence replaced")
    T.eq(json.clean_utf8("\237\160\128"), "\239\191\189\239\191\189\239\191\189", "surrogates replaced")
    T.eq(json.clean_utf8("\195\169 \240\159\152\128"), "\195\169 \240\159\152\128", "valid text untouched")
    T.eq(Sync.clean_text("a\1b", 10), "ab", "control dropped")
    T.eq(Sync.clean_field("a\nb", 10), "a b", "newline to space")
    T.eq(Sync.clean_text("\195\169\195\169\195\169", 2), "\195\169\195\169", "cut on a character")
end)

-- ===== when does "sync automatically" sync? (the owner turned it on and nothing arrived) =====
local function header(req, name)
    for k, v in pairs(req.headers or {}) do if k:lower() == name:lower() then return v end end
end

T.test("every request tells the server who is asking: device, id, KOReader and plugin version, protocol", function()
    local w, p = setup({ hl(1) }, { device_model = "Kobo Libra" })
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(false)
    local req = w.requests[#w.requests]
    T.eq(header(req, "X-Bookx-Device"), "Kobo Libra", "device name")
    T.check((header(req, "X-Bookx-Device-Id") or "") ~= "", "a stable device id")
    T.eq(header(req, "X-Bookx-Client"), "KOReader/2026.07.2-test bookx-plugin/" .. Sync.VERSION, "client")
    T.eq(header(req, "X-Bookx-Protocol"), tostring(Sync.PROTOCOL), "protocol")
    local id = header(req, "X-Bookx-Device-Id")
    p:syncNow(false)
    T.eq(header(w.requests[#w.requests], "X-Bookx-Device-Id"), id, "the id stays the same")
    -- a non-ASCII name travels as plain ASCII in the header (the body carries the real name)
    p.settings:saveSetting("device_name", "Kobo \195\169")
    p:syncNow(false)
    T.eq(header(w.requests[#w.requests], "X-Bookx-Device"), "Kobo ??", "ASCII only in a header")
    -- the connection test says who it is as well
    w.http.script = function() return 200, '{"authorized":"OK"}' end
    p:testConnection()
    T.check(header(w.requests[#w.requests], "X-Bookx-Client") ~= nil, "the test names the client too")
end)

T.test("a highlight is sent about 30 seconds after the last change, not only when the book closes", function()
    local w, p = setup({ hl(1) })
    p.settings:saveSetting("auto", true)
    w.http.script = function() return 200, OK_PUT end
    p:onAnnotationsModified({ hl(1) })
    T.eq(#w.requests, 0, "nothing is sent at once")
    T.eq(#w.jobs, 1, "one job is waiting")
    T.eq(w.jobs[1].delay, 30, "30 seconds")
    p:onAnnotationsModified({ hl(1) })
    p:onAnnotationsModified({ hl(1) })
    T.eq(#w.jobs, 1, "a burst of changes is one job")
    w:run_scheduled()
    T.eq(#w.requests, 1, "then it is sent")
    T.has(p.state.last.message, "Sent 1", "and remembered")
    T.eq(w:log_of("err"), "", "no error")
    T.eq(#w.shown, 0, "an automatic run shows nothing on the screen")
end)

T.test("with automatic sync off a highlight schedules nothing", function()
    local w, p = setup({ hl(1) })
    w.http.script = function() return 200, OK_PUT end
    p:onAnnotationsModified({ hl(1) })
    T.eq(#w.jobs, 0, "no job")
    p:onCloseDocument()
    p:onSuspend()
    p:onReaderReady()
    T.eq(#w.requests, 0, "no request")
end)

T.test("closing the book cancels the pending job and syncs once", function()
    local w, p = setup({ hl(1) })
    p.settings:saveSetting("auto", true)
    w.http.script = function() return 200, OK_PUT end
    p:onAnnotationsModified({ hl(1) })
    p:onCloseDocument()
    T.eq(#w.jobs, 0, "the job is gone")
    T.eq(#w.requests, 1, "one sync")
end)

T.test("Wi-Fi off: the automatic sync is not silently lost, it waits and is shown", function()
    local w, p = setup({ hl(1) }, { online = false })
    p.settings:saveSetting("auto", true)
    w.http.script = function() return 200, OK_PUT end
    p:onCloseDocument()
    T.eq(#w.requests, 0, "nothing can be sent")
    T.check(p.state.waiting ~= nil, "it is remembered as waiting")
    T.has(Sync.status_line(p.state, os.time(), true), "Waiting for Wi-Fi", "the menu says so")
    T.has(p:diagnosticsText(), "Waiting for Wi-Fi", "so does About / Diagnostics")
    -- Wi-Fi comes up while a book is open
    w.online = true
    p:onNetworkConnected()
    T.eq(#w.requests, 1, "it is sent when the network is back")
    T.check(p.state.waiting == nil, "no longer waiting")
    T.has(p.state.last.message, "Sent 1", "result remembered")
    -- and a book opened with Wi-Fi on syncs too
    p:onReaderReady()
    T.eq(#w.requests, 2, "opening a book with Wi-Fi on syncs")
    -- Wi-Fi off at open: nothing, and Wi-Fi is never switched on by the plugin
    w.online = false
    p:onReaderReady()
    T.eq(#w.requests, 2, "no request offline")
    T.check(w.queued == nil, "the plugin never asks for Wi-Fi in an automatic run")
end)

T.test("Sync now shows what was sent and received, and a book Bookx does not have", function()
    local items = { hl(1), hl(2), hl(3) }
    local w, p = setup(items)
    w.http.script = function()
        return 200, '{"document":"d","seq":5,"in_library":false,"annotations":[{"id":"b1","origin":"bookx","kind":"highlight","text":"x","note":"hello","updated":5}]}'
    end
    p:syncNow(false)
    T.has(w:last_text(), "Sent 3 highlights, received 1 note from Bookx", "summary: " .. w:last_text())
    T.has(w:last_text(), "no matching book", "the book is not in the library")
    local d = p:diagnosticsText()
    T.has(d, "Books Bookx could not match in its library (1)", "listed in diagnostics")
    T.has(d, "Sent so far: 3 highlights", "counts")
    T.has(d, "received 1 note", "counts received")
    T.has(d, "Last sync: ok", "last sync time")
    -- once Bookx has the book, it is no longer listed
    w.http.script = function() return 200, '{"document":"d","seq":6,"in_library":true,"annotations":[]}' end
    p:syncNow(false)
    T.has(p:diagnosticsText(), "could not match: none", "cleared")
    T.check(not w:last_text():find("no matching book", 1, true), "no warning any more")
    -- an older server says nothing about it: no warning
    w.http.script = function() return 200, OK_PUT end
    p:syncNow(false)
    T.check(not w:last_text():find("no matching book", 1, true), "an older Bookx: no warning")
end)

T.finish()
