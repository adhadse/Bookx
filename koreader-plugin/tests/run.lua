-- Tests of the Bookx Sync plugin logic under a mock KOReader environment.
-- Run with: koreader-plugin/tests/run.sh   (lua or luajit)
-- With BOOKX_URL, BOOKX_USER and BOOKX_KEY set (the Rust test does this) the live test also talks
-- to a real Bookx server through curl.
local here = (arg and arg[0] or "run.lua"):match("^(.*)/[^/]*$") or "."
package.path = here .. "/../bookx.koplugin/?.lua;" .. package.path
local json = require("bookxjson")
local Sync = require("bookxsync")

local failures, count = 0, 0
local function check(cond, msg)
    count = count + 1
    if not cond then
        failures = failures + 1
        io.stderr:write("FAIL: " .. msg .. "\n")
    end
end
local function eq(a, b, msg) check(a == b, msg .. " (got " .. tostring(a) .. ", wanted " .. tostring(b) .. ")") end

-- json
do
    local t = { a = 1, b = { 1, 2, 3 }, c = "x\n\"y\"\\", d = true, e = 1.5, f = "é ✓ 😀" }
    local back = assert(json.decode(json.encode(t)))
    eq(back.a, 1, "int"); eq(back.b[3], 3, "array"); eq(back.c, "x\n\"y\"\\", "escapes")
    eq(back.d, true, "bool"); eq(back.e, 1.5, "float"); eq(back.f, "é ✓ 😀", "utf8")
    eq(json.decode('"\\u00e9\\ud83d\\ude00"'), "é😀", "unicode escapes")
    eq(json.encode({}), "[]", "empty table is an array")
    eq(json.encode(json.object()), "{}", "empty object")
    eq(json.encode("\1"), '"\\u0001"', "control char")
    check(json.decode("{bad") == nil, "bad json is nil")
    check(json.decode("[1,2] x") == nil, "extra text")
    check(json.decode('{"a":null}').a == json.null, "null")
    local deep = string.rep("[", 100) .. string.rep("]", 100)
    check(json.decode(deep) == nil, "deep nesting refused")
end

-- md5 of the password: dependency-free, and identical to KOReader's when that module exists
do
    local md5 = require("bookxmd5").md5
    eq(md5(""), "d41d8cd98f00b204e9800998ecf8427e", "md5 of empty")
    eq(md5("abc"), "900150983cd24fb0d6963f7d28e17f72", "md5 of abc")
    eq(md5("secret"), "5ebe2294ecd0e0f08eab7690d2a6ee69", "md5 of a password")
    eq(md5(string.rep("x", 1000)), "398533d48111e9f664b1f64cb10c4b63", "md5 across blocks")
    local ko = os.getenv("BOOKX_KO_FRONTEND")
    if ko then
        package.path = ko .. "/?.lua;" .. package.path
        local ok, sha2 = pcall(require, "ffi/sha2")
        check(ok, "KOReader's sha2 can be loaded for comparison")
        if ok then
            for _, s in ipairs{ "", "a", "secret", string.rep("x", 55), string.rep("x", 56),
                    string.rep("x", 57), string.rep("x", 1000) } do
                eq(md5(s), sha2.md5(s), "md5 matches KOReader's sha2 for " .. #s .. " bytes")
            end
        end
    end
end

-- wire format
local NOW = os.time()
local function item(dt, text, note, p0)
    return { datetime = dt, text = text, note = note, pos0 = p0, pos1 = p0 .. ".9", drawer = "lighten",
        color = "yellow", chapter = "Part 1", page = p0, pageno = 12 }
end
do
    local a = Sync.to_wire(item("2024-05-17 20:15:03", "Fear is the mind-killer.", "", "/body/p[1]/text().0"), NOW)
    eq(a.kind, "highlight", "kind"); eq(a.drawer, "lighten", "drawer"); eq(a.page, 12, "page")
    check(a.id:match("^ko%-%x+$") ~= nil, "id format")
    eq(a.id, Sync.to_wire(item("2024-05-17 20:15:03", "other text", "n", "/body/p[1]/text().0"), NOW).id, "id is stable when the note changes")
    local b = Sync.to_wire({ datetime = "2024-05-17 20:15:03", page = 4, text = "Page 4" }, NOW)
    eq(b.kind, "bookmark", "no position = bookmark"); eq(b.text, "", "bookmark text dropped")
    local future = Sync.to_wire(item("2999-01-01 00:00:00", "t", "", "x"), NOW)
    check(future.updated <= NOW, "times are not in the future")
end

-- the look goes to the server exactly as KOReader wrote it: every colour and drawer, an unknown word
-- verbatim (it used to be dropped to ""), nothing that could make the server refuse the request
do
    for _, c in ipairs({ "red", "orange", "yellow", "green", "olive", "cyan", "blue", "purple", "gray" }) do
        for _, d in ipairs({ "lighten", "underscore", "strikeout", "invert" }) do
            local it = item("2024-05-17 20:15:03", "t", "", "/p")
            it.color = c; it.drawer = d
            local a = Sync.to_wire(it, NOW)
            eq(a.color, c, "colour " .. c); eq(a.drawer, d, "drawer " .. d)
        end
    end
    local it = item("2024-05-17 20:15:03", "t", "", "/p")
    it.color = "teal"; it.drawer = "wavy"
    local a = Sync.to_wire(it, NOW)
    eq(a.color, "teal", "an unknown colour is passed on"); eq(a.drawer, "wavy", "an unknown drawer is passed on")
    it.color = string.rep("x", 100) .. "\n"; it.drawer = nil
    a = Sync.to_wire(it, NOW)
    check(#a.color <= 32 and not a.color:find("%c"), "a long or control-character colour cannot refuse the request")
    eq(a.drawer, "", "no drawer says nothing")
    local changed = item("2024-05-17 20:15:03", "t", "", "/p")
    local other = item("2024-05-17 20:15:03", "t", "", "/p")
    other.color = "purple"
    check(Sync.fingerprint(Sync.to_wire(changed, NOW)) ~= Sync.fingerprint(Sync.to_wire(other, NOW)), "a colour change is a change")
end

-- PDF positions are tables: the id must not depend on a table address
do
    local pdf = function(x) return { datetime = "2024-05-17 20:15:03", page = 5, pageno = 5, text = "pdf quote text",
        pos0 = { page = 5, x = x, y = 2 }, pos1 = { page = 5, x = x + 1, y = 3 }, drawer = "lighten" } end
    eq(Sync.to_wire(pdf(1), NOW).id, Sync.to_wire(pdf(1), NOW).id, "id of a PDF highlight is stable")
    check(Sync.to_wire(pdf(1), NOW).id ~= Sync.to_wire(pdf(7), NOW).id, "PDF positions tell highlights apart")
end

-- names: no control characters (the server refuses them; seen with a book of two authors)
eq(Sync.clean_name("Noah Gift\nAlfredo Deza"), "Noah Gift; Alfredo Deza", "authors on one line")
eq(Sync.clean_name("a\tb\r\nc"), "a b; c", "other control characters")
eq(Sync.clean_name(nil), "", "nil")

-- diff: new, unchanged, changed, vanished
do
    local one = item("2024-05-17 20:15:03", "First quote here.", "", "/a")
    local two = item("2024-05-18 10:00:00", "Second quote here.", "", "/b")
    local send, synced = Sync.diff({ one, two }, {}, NOW)
    eq(#send, 2, "two new")
    local send2 = Sync.diff({ one, two }, synced, NOW)
    eq(#send2, 0, "nothing new on the second run")
    two.note = "my thought"
    local send3, synced3 = Sync.diff({ one, two }, synced, NOW)
    eq(#send3, 1, "a changed note is sent"); eq(send3[1].updated, NOW, "and dated now")
    local send4 = Sync.diff({ one }, synced3, NOW)
    eq(#send4, 1, "a vanished one is sent"); eq(send4[1].deleted, true, "as a tombstone")
end

-- sync_book with a scripted server
do
    local calls = {}
    local reply_seq = 7
    local function request(method, url, headers, body)
        calls[#calls + 1] = { method = method, url = url, headers = headers, body = json.decode(body) }
        return 200, json.encode({ document = "d", seq = reply_seq, annotations = {
            { id = "bx-1", kind = "highlight", text = "From the computer", note = "hi", origin = "bookx", updated = 5, deleted = false },
            { id = "ko-other", kind = "highlight", text = "from koreader", origin = "koreader", updated = 5 },
        } })
    end
    local state = {}
    local doc = string.rep("a", 32)
    local ctx = { server = "http://h:1/bookx/", user = "u", key = "k", device = "Libra", device_id = "id",
        document = doc, title = "Dune", authors = "FH", now = NOW, state = state, request = request,
        items = { item("2024-05-17 20:15:03", "First quote here.", "", "/a") } }
    local ok, r = Sync.sync_book(ctx)
    check(ok, "sync ok"); eq(r.sent, 1, "sent"); eq(r.received, 1, "received only Bookx's notes")
    eq(calls[1].url, "http://h:1/bookx/annotations/" .. doc .. "?since=0", "url")
    eq(calls[1].headers["x-auth-user"], "u", "auth user"); eq(calls[1].headers["x-auth-key"], "k", "auth key")
    eq(calls[1].body.title, "Dune", "title sent"); eq(calls[1].body.origin, "koreader", "origin")
    eq(#Sync.inbox_list(state, doc), 1, "inbox has the Bookx note")
    local ok2, r2 = Sync.sync_book(ctx)
    check(ok2, "second sync ok"); eq(r2.sent, 0, "nothing sent twice"); eq(r2.received, 0, "inbox note not counted twice")
    eq(calls[2].url, "http://h:1/bookx/annotations/" .. doc .. "?since=7", "since cursor used")

    -- the /bookx address, a bare host and a path behind a reverse proxy
    local d = string.rep("b", 32)
    eq(Sync.build_url("http://h:1/bookx", d, 3), "http://h:1/bookx/annotations/" .. d .. "?since=3", "bookx url")
    eq(Sync.build_url("http://h:1/bookx/", d, nil), "http://h:1/bookx/annotations/" .. d .. "?since=0", "trailing slash, no cursor")
    check(not Sync.build_url("http://h:1/bookx", d, 0):find("kosync"), "no legacy endpoint is ever built")
    eq(Sync.build_url("http://h:1", d, 0), "http://h:1/bookx/annotations/" .. d .. "?since=0", "bare host gets /bookx")
    eq(Sync.build_url("https://proxy.example/apps/bookx", d, 0), "https://proxy.example/apps/bookx/annotations/" .. d .. "?since=0", "custom path kept")

    -- a failing server leaves the state alone, so the next run repeats the send
    local failing = { server = "http://h:1/bookx", user = "u", key = "k", device = "L", device_id = "i",
        document = doc, title = "", authors = "", now = NOW, state = {},
        items = ctx.items, request = function() return 500, "{}" end }
    local okf, msg = Sync.sync_book(failing)
    check(not okf, "failure reported"); check(next(failing.state.docs[doc].synced) == nil, "state unchanged on failure")
    failing.request = function() return 401, "{}" end
    local _, m401 = Sync.sync_book(failing)
    check(m401:find("password") ~= nil, "401 says what is wrong")
    failing.request = function() return nil, "timeout" end
    local ok3, m3 = Sync.sync_book(failing)
    check(not ok3, "network error is a failure"); check(type(m3) == "string", "with a message")
    failing.request = function() return 200, "not json" end
    check(not Sync.sync_book(failing), "garbage answer is a failure")
    check(Sync.status_line({}, NOW):find("Not synced") ~= nil, "status before first sync")
    eq(Sync.status_line({}, NOW, false), "Not configured", "status: not configured")
    eq(Sync.status_line({}, NOW, true), "Not tested yet", "status: not tested yet")
    check(Sync.status_line({ test = { time = NOW - 300, ok = true } }, NOW, true):find("Last test OK 5 min ago", 1, true) ~= nil, "status: test ok")
    check(Sync.status_line({ test = { time = NOW, ok = false, message = "Wrong user name or password" } }, NOW, true):find("Last test failed", 1, true) ~= nil, "status: test failed")
    check(Sync.status_line({ test = { time = NOW - 600, ok = true }, last = { time = NOW - 60, ok = true, message = "m" } }, NOW, true):find("Last sync", 1, true) ~= nil, "status: newer sync wins")
    check(Sync.status_line({ test = { time = NOW - 60, ok = true }, last = { time = NOW - 600, ok = true, message = "m" } }, NOW, true):find("Last test OK", 1, true) ~= nil, "status: newer test wins")
    check(#Sync.status_line({ last = { time = NOW, ok = false, message = "x" } }, NOW, true) < 80, "status line is short")
    eq(Sync.auth_url("http://h:1"), "http://h:1/bookx/users/auth", "auth url, bare host")
    eq(Sync.auth_url("http://h:1/bookx/"), "http://h:1/bookx/users/auth", "auth url, with path")
    eq(Sync.auth_url("https://h/proxy/bookx"), "https://h/proxy/bookx/users/auth", "auth url, proxy path")
    eq(Sync.create_url("http://h:1"), "http://h:1/bookx/users/create", "create url")
    check(Sync.status_line({ last = { time = NOW - 300, ok = true, message = "fine" } }, NOW):find("5 min ago") ~= nil, "status text")
end

-- live test through curl (only when the Rust test provides a server)
local url, user, key = os.getenv("BOOKX_URL"), os.getenv("BOOKX_USER"), os.getenv("BOOKX_KEY")
if url then
    local function curl(method, u, headers, body)
        local f = os.tmpname()
        local h = io.open(f, "w"); h:write(body); h:close()
        local args = { "curl", "-s", "-S", "--max-time", "10", "-X", method, "-o", f .. ".out", "-w", "%{http_code}",
            "--data-binary", "@" .. f }
        for k, v in pairs(headers) do args[#args + 1] = "-H"; args[#args + 1] = "'" .. k .. ": " .. v .. "'" end
        args[#args + 1] = "'" .. u .. "'"
        local p = io.popen(table.concat(args, " "))
        local code = tonumber(p:read("*a")); p:close()
        local o = io.open(f .. ".out", "r"); local text = o and o:read("*a") or ""; if o then o:close() end
        os.remove(f); os.remove(f .. ".out")
        return code, text
    end
    local doc = os.getenv("BOOKX_DOC") or string.rep("b", 32)
    local state = {}
    local its = {
        item("2024-05-17 20:15:03", os.getenv("BOOKX_QUOTE1") or "Live quote number one.", "a note", "/a"),
        item("2024-05-18 10:00:00", os.getenv("BOOKX_QUOTE2") or "Live quote number two.", "", "/b"),
    }
    local ctx = { server = url, user = user, key = key, device = "Mock Libra", device_id = "mock",
        document = doc, title = os.getenv("BOOKX_TITLE") or "Live Book", authors = os.getenv("BOOKX_AUTHORS") or "Tester",
        now = os.time(), state = state, request = curl, items = its }
    local ok, r = Sync.sync_book(ctx)
    check(ok, "live sync ok: " .. tostring(ok and "" or r))
    if ok then eq(r.sent, 2, "live: two sent") end
    local ok2, r2 = Sync.sync_book(ctx)
    check(ok2 and r2.sent == 0, "live: repeated sync sends nothing")
    ctx.items = { its[1] }
    local ok3, r3 = Sync.sync_book(ctx)
    check(ok3 and r3.deleted == 1, "live: a removed highlight becomes a tombstone")
    ctx.key = "wrong"
    local ok4, r4 = Sync.sync_book(ctx)
    check(not ok4 and tostring(r4):find("password"), "live: wrong key refused")
    if os.getenv("BOOKX_EXPECT_INBOX") then
        ctx.key = key
        local ok5 = Sync.sync_book(ctx)
        check(ok5 and #Sync.inbox_list(state, doc) >= 1, "live: a note made in Bookx arrives in the inbox")
    end
end

if failures > 0 then
    io.stderr:write(string.format("%d of %d checks failed\n", failures, count))
    os.exit(1)
end
print(string.format("ok: %d checks", count))
