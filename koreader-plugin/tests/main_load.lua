-- Loads main.lua (the KOReader glue) under host luajit with a mock KOReader around it and runs its
-- main paths: init, menu registration, settings dialog, sync now (annotation fields as KOReader
-- v2026.07 stores them), inbox, events. When BOOKX_KO_FRONTEND points at a copy of KOReader's
-- `frontend/` (+ `ffi/`) folder, the REAL luasettings, dump, gettext, logger, ffi/sha2 and menusorter are
-- used, so the menu entry is also placed by KOReader's own menu sorter. Otherwise small stand-ins are used.
local here = (arg and arg[0] or "main_load.lua"):match("^(.*)/[^/]*$") or "."
local plugin = here .. "/../bookx.koplugin"
local ko = os.getenv("BOOKX_KO_FRONTEND")
local tmp = os.tmpname(); os.remove(tmp); os.execute("mkdir -p '" .. tmp .. "'")
local failures, count = 0, 0
local function check(c, m) count = count + 1; if not c then failures = failures + 1; io.stderr:write("FAIL: " .. m .. "\n") end end
local function eq(a, b, m) check(a == b, m .. " (got " .. tostring(a) .. ", wanted " .. tostring(b) .. ")") end

local function stub_md5(s)
    local a, b = 7, 11
    for i = 1, #s do a = (a * 31 + s:byte(i)) % 4294967291; b = (b * 17 + s:byte(i) + i) % 4294967279 end
    return string.format("%016x%016x", a, b)
end
local shown = {}
local function widget(name) local W = { name = name }; W.__index = W
    function W:new(o) o = setmetatable(o or {}, self); if o.init then o:init() end; return o end
    function W:extend(o) o = o or {}; o.__index = o; return setmetatable(o, { __index = self }) end
    return W end
local P = package.preload
P["datastorage"] = function() return { getSettingsDir = function() return tmp end } end
P["dispatcher"] = function() return {} end
P["ui/widget/infomessage"] = function() return widget("info") end
P["ui/widget/textviewer"] = function() return widget("viewer") end
P["ui/widget/multiinputdialog"] = function()
    local W = widget("dialog"); function W:getFields() return self.values or { "", "", "", "" } end
    function W:onShowKeyboard() end; return W end
P["ui/widget/container/widgetcontainer"] = function() return widget("container") end
P["ui/uimanager"] = function() return { show = function(_, w) shown[#shown + 1] = w end, close = function() end } end
local online = true
P["ui/network/manager"] = function() return { isOnline = function() return online end,
    runWhenOnline = function(_, f) f() end } end
P["socketutil"] = function() return { set_timeout = function() end, reset_timeout = function() end } end
P["device"] = function()
    return setmetatable({ model = "Kobo" }, { __index = function() return function() return false end end }) end
P["util"] = function() return { partialMD5 = function() return string.rep("a", 32) end } end
if ko then
    package.path = ko .. "/frontend/?.lua;" .. ko .. "/?.lua;" .. package.path
    P["libs/libkoreader-lfs"] = function() return { attributes = function(f, what)
        local h = io.open(f, "r"); if not h then return nil end; h:close()
        return what == "mode" and "file" or 0 end } end
    P["util"] = function() return { partialMD5 = function() return string.rep("a", 32) end,
        splitToArray = function() return {} end, tableSize = function(t) local n = 0; for _ in pairs(t) do n = n + 1 end; return n end,
        makePath = function() end,
        -- KOReader's real writeToFile prepends `return` when `lua_dofile_ready` is set
        writeToFile = function(data, file, _, ready)
            local h = assert(io.open(file, "wb"))
            if ready then h:write("-- ", file, "\nreturn ", data, "\n") else h:write(data) end
            h:close(); return true end, tableEquals = function() return false end } end
    P["ffi/util"] = function() return { orderedPairs = function(t)
        local keys = {}; for k in pairs(t) do keys[#keys + 1] = k end
        table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
        local i = 0; return function() i = i + 1; if keys[i] ~= nil then return keys[i], t[keys[i]] end end end } end
else
    local files = {}
    P["luasettings"] = function()
        local S = {}; S.__index = S
        function S:open(f) files[f] = files[f] or {}; return setmetatable({ file = f, data = files[f] }, S) end
        function S:readSetting(k) return self.data[k] end
        function S:saveSetting(k, v) self.data[k] = v end
        function S:isTrue(k) return self.data[k] == true end
        function S:toggle(k) self.data[k] = not self.data[k] end
        function S:flush() end
        return S end
    P["gettext"] = function() return function(s) return s end end
    P["logger"] = function() return { info = function() end } end
    P["ffi/sha2"] = function() return { md5 = function(s) return stub_md5(s) end } end
end

-- a fake `socket.http` / `ltn12` so a sync can run without a network
local requests = {}
package.loaded["ltn12"] = { source = { string = function(s) return s end }, sink = { table = function(t) return t end } }
local responder
package.loaded["socket.http"] = { request = function(r)
    requests[#requests + 1] = r
    if responder then
        local code, body = responder(r)
        if code == nil or type(code) == "string" then return nil, body or code end
        r.sink[#r.sink + 1] = body or ""
        return 1, code
    end
    r.sink[#r.sink + 1] = '{"seq":3,"annotations":[{"id":"b1","origin":"bookx","text":"from Bookx","note":"hi","updated":5}]}'
    return 1, 200 end }

package.path = plugin .. "/?.lua;" .. package.path
local Sync = require("bookxsync")
local menu_items_registered
local ui = { menu = { registerToMainMenu = function(_, w) menu_items_registered = w end },
    document = { file = "/mnt/onboard/b.epub" },
    doc_settings = { readSetting = function(_, k)
        if k == "partial_md5_checksum" then return string.rep("c", 32) end
        if k == "doc_props" then return { title = "Real Book", authors = "A\nB" } end end },
    annotation = { annotations = {
        { datetime = "2026-11-14 07:34:25", drawer = "lighten", color = "yellow", chapter = "Ch", pageno = 233,
          page = "/body/DocFragment[11]/body/p[1]/text().0", pos0 = "/body/DocFragment[11]/body/p[1]/text().0",
          pos1 = "/body/DocFragment[11]/body/p[1]/text().20", text = "A quote from a book." },
        { datetime = "2026-08-21 21:43:52", chapter = "Ch", pageno = 73, page = "/body/DocFragment[7]/body/p[3]/text().159",
          text = "in Ch" },
    } } }
local Bookx = dofile(plugin .. "/main.lua")
local b = Bookx:new{ ui = ui }
check(menu_items_registered == b, "registers itself with the main menu")
check(b.settings:readSetting("device_id") ~= nil, "device id made")
local items = {}; b:addToMainMenu(items)
check(items.bookx and items.bookx.sorting_hint == "more_tools", "menu entry hangs under Tools > More tools")
eq(#items.bookx.sub_item_table, 7, "seven entries")
for _, e in ipairs(items.bookx.sub_item_table) do check(e.text or e.text_func, "entry has text") end
if ko then
    local Sorter = require("ui/menusorter")
    local order = dofile(ko .. "/frontend/ui/elements/reader_menu_order.lua")
    local item_table = { ["KOMenu:menu_buttons"] = {}, more_tools = { text = "More tools" }, bookx = items.bookx }
    for _, tab in ipairs{ "navi", "typeset", "setting", "tools", "search", "filemanager", "main" } do
        item_table[tab] = { icon = "appbar." .. tab }
    end
    local ok, res = pcall(function() return Sorter:sort(item_table, order) end)
    check(ok, "the real KOReader menu sorter accepts the entry: " .. tostring(res))
    if ok then
        local found
        for _, tab in ipairs(res) do
            if tab.icon == "appbar.tools" then
                for _, e in ipairs(tab) do
                    if e.id == "more_tools" and e.sub_item_table then
                        for _, x in ipairs(e.sub_item_table) do if x == items.bookx then found = true end end
                    end
                end
                if not found then for _, e in ipairs(tab) do if e == items.bookx then found = "tools-tab" end end end
            end
        end
        check(found == true, "the entry is in Tools > More tools of the sorted menu (found: " .. tostring(found) .. ")")
        check(items.bookx.new == true and items.bookx.id == "bookx", "sorter treated it as a plugin entry")
        check(not items.bookx.text:match("^NEW:"), "not marked as an orphan")
    end
end

-- not configured: says so
shown = {}
b:syncNow(false)
eq(#shown, 1, "message when not configured")
-- settings dialog: password -> md5 key
b:editSettings()
local dlg = shown[#shown]
dlg.values = { "http://192.168.1.20:8765/bookx", "ann", "secret", "My Kobo" }
for _, row in ipairs(dlg.buttons) do for _, btn in ipairs(row) do if btn.text == "Save" then btn.callback() end end end
eq(b.settings:readSetting("server"), "http://192.168.1.20:8765/bookx", "server saved")
eq(b.settings:readSetting("user"), "ann", "user saved")
eq(b.settings:readSetting("key"), "5ebe2294ecd0e0f08eab7690d2a6ee69", "key = md5 of the password")
check(b:configured(), "configured")
-- sync
shown = {}
b:syncNow(false)
eq(#requests, 1, "one request")
local r = requests[1]
eq(r.method, "PUT", "PUT")
check(r.url:match("/bookx/annotations/" .. string.rep("c", 32) .. "%?since=0$") ~= nil, "url: " .. r.url)
eq(r.headers["x-auth-user"], "ann", "auth user")
local json = require("bookxjson")
local sent = json.decode(r.source)
eq(sent.authors, "A; B", "several authors (KOReader separates them with a new line) are sent on one line")
eq(sent.title, "Real Book", "title"); eq(#sent.annotations, 2, "two annotations (highlight + page bookmark)")
local kinds = {}; for _, a in ipairs(sent.annotations) do kinds[a.kind] = a end
check(kinds.highlight and kinds.highlight.page == 233 and kinds.highlight.drawer == "lighten", "highlight fields")
check(kinds.bookmark and kinds.bookmark.text == "", "page bookmark has no text")
eq(shown[1].text, "Sent 2: 1 highlight, 1 bookmark, received 1 note from Bookx", "summary shown")
shown = {}; b:showInbox()
check(shown[1].text:find("from Bookx", 1, true) ~= nil, "inbox shows the note")
-- events
requests = {}
b.settings:saveSetting("auto", true)
b:onSuspend()
eq(#requests, 1, "auto sync on suspend sends the (empty) delta once")
eq(#json.decode(requests[1].source).annotations, 0, "nothing changed: no annotation is sent again")
b:onCloseDocument(); eq(#requests, 2, "auto sync on close")
online = false; b:onSuspend(); eq(#requests, 2, "offline: quiet, nothing sent")

-- ===== account UX: save feedback, test connection, register =====
online = true
local WIRE_KEY = "5ebe2294ecd0e0f08eab7690d2a6ee69"
local function press(dialog, label)
    for _, row in ipairs(dialog.buttons) do for _, btn in ipairs(row) do
        if btn.text == label then btn.callback(); return true end end end
    return false
end
local function settings_text(dir)
    local out = {}
    for _, name in ipairs{ "bookx_sync.lua", "bookx_sync_state.lua" } do
        local h = io.open(dir .. "/" .. name, "r"); if h then out[#out + 1] = h:read("*a"); h:close() end
    end
    return table.concat(out, "\n")
end
local function reopen()
    local nb = Bookx:new{ ui = ui }
    return nb
end
local function open_dialog(bk)
    shown = {}; bk:editSettings(); return shown[#shown]
end
local function last_text() return shown[#shown] and shown[#shown].text or "" end

-- Save: confirmation message with server and user, key persisted, plaintext never written
do
    local d = open_dialog(b)
    check(d.fields[3].hint:find("Saved", 1, true) ~= nil and d.fields[3].hint:find("leave empty to keep", 1, true) ~= nil,
        "reopened with a saved key: the password hint says it is saved (" .. tostring(d.fields[3].hint) .. ")")
    eq(d.fields[3].text, "", "the password field itself stays empty")
    d.values = { "http://192.168.1.20:8765/bookx", "ann", "", "My Kobo" }
    press(d, "Save")
    eq(b.settings:readSetting("key"), WIRE_KEY, "empty password on a later save keeps the old key")
    local m = last_text()
    check(m:find("Saved. Password is kept as a hash; the field will be empty next time.", 1, true) ~= nil, "save message: " .. m)
    check(m:find("192.168.1.20:8765/bookx", 1, true) ~= nil and m:find("ann", 1, true) ~= nil, "save message names server and user")
    check(m:find(WIRE_KEY, 1, true) == nil and m:find("secret", 1, true) == nil, "save message never shows key or password")
    local d2 = open_dialog(b)
    d2.values = { "http://192.168.1.20:8765/bookx", "ann", "newpass", "My Kobo" }
    press(d2, "Save")
    eq(b.settings:readSetting("key"), "e6053eb8d35e02ae40beeeacef203c1a", "a new password replaces the key")
    d2 = open_dialog(b)
    d2.values = { "http://192.168.1.20:8765/bookx", "ann", "secret", "My Kobo" }
    press(d2, "Save")
    eq(b.settings:readSetting("key"), WIRE_KEY, "key set again")
    -- reopen (new plugin instance, same files)
    if ko then
        local nb = reopen()
        eq(nb.settings:readSetting("key"), WIRE_KEY, "key persists across reopen (real LuaSettings file)")
        eq(open_dialog(nb).fields[3].text, "", "reopened field is empty")
        check(settings_text(tmp):find("secret", 1, true) == nil, "plaintext password is not in the settings files")
        check(settings_text(tmp):find(WIRE_KEY, 1, true) ~= nil, "the hash is in the settings file")
    else
        local nb = reopen()
        eq(nb.settings:readSetting("key"), WIRE_KEY, "key persists across reopen")
        check(open_dialog(nb).fields[3].hint:find("Saved", 1, true) ~= nil, "reopened hint says saved")
        local all = {}
        for k, v in pairs(nb.settings.data) do all[#all + 1] = tostring(k) .. "=" .. tostring(v) end
        for k, v in pairs(nb.state_file.data) do all[#all + 1] = tostring(k) .. "=" .. tostring(v) end
        check(table.concat(all, "\n"):find("secret", 1, true) == nil, "plaintext password is not in the stored settings")
    end
end

-- no key yet: the hint says so
do
    local saved = b.settings:readSetting("key")
    b.settings:saveSetting("key", nil)
    check(open_dialog(b).fields[3].hint:find("not saved yet", 1, true) ~= nil, "no key: hint says not saved yet")
    b.settings:saveSetting("key", saved)
end

-- Test connection
do
    local function run_test(responder_fn)
        responder = responder_fn; requests = {}; shown = {}
        b:testConnection()
        return last_text()
    end
    local m = run_test(function() return 200, '{"authorized":"OK"}' end)
    eq(m, "Connected as ann", "test: 200")
    eq(#requests, 2, "an auth request and the version check")
    eq(requests[1].method, "GET", "auth is a GET")
    check(requests[1].url == "http://192.168.1.20:8765/bookx/users/auth", "auth url: " .. requests[1].url)
    eq(requests[1].headers["x-auth-user"], "ann", "auth user header")
    eq(requests[1].headers["x-auth-key"], WIRE_KEY, "auth key header is the hash")
    check(b.state.test and b.state.test.ok == true, "test result recorded")
    check(m:find(WIRE_KEY, 1, true) == nil, "message has no key")
    eq(run_test(function() return 401, "{}" end), "Wrong user name or password", "test: 401")
    eq(b.state.test.ok, false, "failed test recorded")
    eq(run_test(function() return 404, "{}" end), "Bookx sync is not enabled at that address (404)", "test: 404")
    eq(run_test(function() return nil, "connection refused" end), "Cannot reach 192.168.1.20:8765 (connection refused)", "test: network error")
    check(run_test(function() return nil, "timeout" end):find("timeout", 1, true) ~= nil, "test: timeout")
    -- a saved URL without the path gets /bookx
    b.settings:saveSetting("server", "http://192.168.1.20:8765/")
    run_test(function() return 200, "{}" end)
    eq(requests[1].url, "http://192.168.1.20:8765/bookx/users/auth", "bare host: auth url gets /bookx")
    b.settings:saveSetting("server", "http://192.168.1.20:8765/bookx")
    -- not configured
    local saved = b.settings:readSetting("key"); b.settings:saveSetting("key", nil)
    requests = {}; shown = {}; b:testConnection()
    eq(#requests, 0, "not configured: no request"); check(last_text():find("first", 1, true) ~= nil, "not configured: message")
    b.settings:saveSetting("key", saved)
    -- the dialog button saves and tests
    responder = function() return 200, "{}" end
    local d = open_dialog(b)
    d.values = { "http://192.168.1.20:8765/bookx", "ann", "", "My Kobo" }
    requests = {}; press(d, "Test connection")
    check(#requests >= 1, "the button in the dialog tests")
    -- menu item
    local its = {}; b:addToMainMenu(its)
    local found
    for _, e in ipairs(its.bookx.sub_item_table) do if e.text == "Test connection" then found = e end end
    check(found ~= nil, "Test connection menu item")
    requests = {}; found.callback(); check(#requests >= 1, "menu item runs the test")
    b.state.last = nil
    check(its.bookx.sub_item_table[1].text_func():find("Last test OK", 1, true) ~= nil, "status shows the test result: " .. its.bookx.sub_item_table[1].text_func())
end

-- Register account
do
    local fresh = Bookx:new{ ui = ui }
    fresh.settings:saveSetting("key", nil); fresh.settings:saveSetting("user", nil)
    local d = open_dialog(fresh)
    local sent_bodies = {}
    responder = function(r)
        if r.method == "POST" then sent_bodies[#sent_bodies + 1] = r; return 201, '{"username":"zoe"}' end
        return 200, "{}"
    end
    d.values = { "http://10.0.0.5:8765", "zoe", "secret", "Kobo" }
    requests = {}; shown = {}
    press(d, "Register account")
    local post = requests[1]
    eq(post.method, "POST", "register is a POST")
    eq(post.url, "http://10.0.0.5:8765/bookx/users/create", "register url gets /bookx")
    local body = json.decode(post.source)
    eq(body.username, "zoe", "register user"); eq(body.password, WIRE_KEY, "register posts the md5")
    check(post.source:find("secret", 1, true) == nil, "register never posts the plaintext")
    eq(fresh.settings:readSetting("key"), WIRE_KEY, "credentials saved after register")
    eq(fresh.settings:readSetting("user"), "zoe", "user saved after register")
    eq(requests[2].method, "GET", "register then tests the connection")
    eq(last_text(), "Connected as zoe", "register ends with the test result")
    -- failures keep nothing
    local function fail_with(code, body_or_err)
        local f2 = Bookx:new{ ui = ui }
        f2.settings:saveSetting("key", nil)
        local dd = open_dialog(f2)
        responder = function() return code, body_or_err end
        dd.values = { "http://10.0.0.5:8765/bookx", "zoe", "secret", "" }
        shown = {}; press(dd, "Register account")
        check(f2.settings:readSetting("key") == nil, "failed register saves no key")
        return last_text()
    end
    check(fail_with(402, "{}"):find("taken", 1, true) ~= nil, "402: name taken")
    check(fail_with(403, "{}"):find("closed", 1, true) ~= nil, "403: registration closed")
    eq(fail_with(nil, "connection refused"), "Cannot reach 10.0.0.5:8765 (connection refused)", "register network error")
    check(fail_with(404, "{}"):find("not enabled", 1, true) ~= nil, "register 404")
    -- empty fields
    local f3 = Bookx:new{ ui = ui }; local dd = open_dialog(f3); dd.values = { "http://x/bookx", "u", "", "" }
    requests = {}; shown = {}; press(dd, "Register account")
    eq(#requests, 0, "register needs a password"); check(last_text():find("password", 1, true) ~= nil, "register hint")
end

-- ===== panic hardening: an error in a callback is reported, never thrown at KOReader =====
-- The owner's crash: saving the password used to require KOReader's FFI md5, which raised an
-- error on the device. The plugin now carries its own md5 and guards every entry point.
do
    local saved_ffi = package.loaded["ffi/sha2"]
    package.loaded["ffi/sha2"] = nil
    local fresh = Bookx:new{ ui = ui }
    local d = open_dialog(fresh)
    d.values = { "http://h:8765/bookx", "u", "secret", "" }
    shown = {}
    press(d, "Save")
    eq(fresh.settings:readSetting("key"), "5ebe2294ecd0e0f08eab7690d2a6ee69", "saving works with no FFI module")
    check(last_text():find("Saved", 1, true) ~= nil, "saving is confirmed without an FFI module")
    package.loaded["ffi/sha2"] = saved_ffi
end

do
    local bad = Bookx:new{ ui = ui }
    bad.settings = setmetatable({}, { __index = function() error("settings exploded") end })
    shown = {}
    local ok = pcall(function() return bad:syncNow(false) end)
    check(ok, "an error in a settings read does not escape a plugin method")
    local ok2 = pcall(function() bad:onCloseDocument() end)
    check(ok2, "an error in an event hook does not escape")
    check(#shown >= 1 and last_text():find("unexpected error", 1, true) ~= nil,
        "the error is shown as a message: " .. last_text())
end

do
    local its = {}
    b:addToMainMenu(its)
    local entries = its.bookx.sub_item_table
    local saved_status = Sync.status_line
    Sync.status_line = function() error("status exploded") end
    shown = {}
    local ok = pcall(function() return entries[1].text_func() end)
    check(ok, "an error in the menu status text is caught")
    eq(entries[1].text_func(), "", "the menu falls back to an empty status")
    Sync.status_line = saved_status
    local saved_ui, saved_settings = b.ui, b.settings
    b.ui = setmetatable({}, { __index = function() error("ui exploded") end })
    local ok2 = pcall(function() return entries[4].enabled_func() end)
    eq(entries[4].enabled_func(), false, "the menu falls back to disabled")
    b.ui = saved_ui
    b.settings = setmetatable({}, { __index = function() error("settings exploded") end })
    local ok3 = pcall(function() return entries[5].checked_func() end)
    check(ok2 and ok3, "errors in menu enabled/checked functions are caught")
    b.settings = saved_settings
end

do
    local d = open_dialog(b)
    local saved_fields = d.getFields
    d.getFields = function() error("dialog exploded") end
    shown = {}
    local ok = pcall(function() press(d, "Save") end)
    check(ok, "an error while reading the dialog fields is caught")
    d.getFields = saved_fields
end

responder = nil
os.execute("rm -rf '" .. tmp .. "'")
print(string.format("main.lua load tests: %d checks, %d failures%s", count, failures, ko and " (real KOReader modules)" or ""))
os.exit(failures == 0 and 0 or 1)
