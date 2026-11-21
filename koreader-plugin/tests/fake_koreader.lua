-- A fake KOReader for the Bookx Sync tests: stand-ins for the modules main.lua requires, built to behave
-- like the real ones where the plugin depends on it (widgets copy their constructor arguments onto the
-- instance, the plugin loader wraps `on*` handlers, ltn12 sources and sinks are real functions, socket.http
-- drains the request source and fills the sink). Every test builds its own world with `K.world()`, so tests
-- share nothing and can run in any order.
--
--   local K = dofile(here .. "/fake_koreader.lua")
--   local w = K.world{ online = true }
--   local plugin = w:plugin()          -- main.lua loaded and instantiated like ReaderUI does
--   w.http.script = function(req) return 200, '{"seq":1,"annotations":[]}' end
--   plugin:syncNow(false)
--   w:last_text()                      -- text of the last InfoMessage
--
-- `w.http.script(req)` returns `code, body` (an HTTP answer), or `nil, "error text"` (a socket error); it can
-- also `error()` to simulate a library that throws. `req` has method, url, headers, body (the source drained).
local K = {}

local here = (debug.getinfo(1, "S").source or ""):match("^@(.*)/[^/]*$") or "."
K.plugin_dir = here .. "/../bookx.koplugin"

local function new_class(name)
    local C = { name = name }
    C.__index = C
    -- KOReader's Widget:new / :extend (frontend/ui/widget/widget.lua, event.lua): the constructor table
    -- becomes the instance, so a constructor argument named like a method SHADOWS that method.
    function C:new(o)
        o = o or {}
        setmetatable(o, self)
        self.__index = self
        if o.init then o:init() end
        return o
    end
    function C:extend(sub)
        sub = sub or {}
        sub.__index = sub
        setmetatable(sub, { __index = self })
        return sub
    end
    return C
end

-- ltn12 as in LuaSocket (only what the plugin and socket.http need)
local ltn12 = {
    source = {
        string = function(s)
            local done = false
            return function()
                if done or s == "" then return nil end
                done = true
                return s
            end
        end,
    },
    sink = {
        table = function(t)
            t = t or {}
            return function(chunk)
                if chunk then t[#t + 1] = chunk end
                return 1
            end, t
        end,
    },
}

--- Lua table -> text the way KOReader's `dump`/LuaSettings write it, used by the in-memory LuaSettings.
local function deep_copy(v)
    if type(v) ~= "table" then return v end
    local c = {}
    for k, x in pairs(v) do c[k] = deep_copy(x) end
    return c
end

function K.world(opts)
    opts = opts or {}
    local W = {
        shown = {}, closed = {}, logs = {}, files = {}, requests = {},
        online = opts.online ~= false, device_model = opts.device_model or "Kobo",
        http = { script = nil },
        timeouts = {}, jobs = {},
    }
    W.dir = opts.dir or os.tmpname()
    if not opts.dir then os.remove(W.dir); os.execute("mkdir -p '" .. W.dir .. "'") end

    function W:last_text() local w = self.shown[#self.shown]; return w and w.text or "" end
    function W:texts()
        local t = {}
        for _, w in ipairs(self.shown) do t[#t + 1] = w.text or w.title or "" end
        return t
    end
    function W:log_of(level)
        local out = {}
        for _, l in ipairs(self.logs) do if l.level == level then out[#out + 1] = l.text end end
        return table.concat(out, "\n")
    end
    --- Runs the jobs the plugin scheduled (as KOReader does when their time has come) and returns how many.
    function W:run_scheduled()
        local jobs = self.jobs
        self.jobs = {}
        for _, j in ipairs(jobs) do j.fn() end
        return #jobs
    end
    function W:cleanup() os.execute("rm -rf '" .. self.dir .. "'") end

    local P = {}
    W.preload = P
    P["datastorage"] = function() return { getSettingsDir = function() return W.dir end } end
    P["gettext"] = function() return function(s) return s end end
    P["logger"] = function()
        local L = {}
        for _, level in ipairs({ "dbg", "info", "warn", "err" }) do
            L[level] = function(...)
                local parts = {}
                for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
                W.logs[#W.logs + 1] = { level = level, text = table.concat(parts, " ") }
            end
        end
        return L
    end
    P["luasettings"] = function()
        local S = new_class("luasettings")
        function S:open(f)
            W.files[f] = W.files[f] or {}
            return setmetatable({ file = f, data = W.files[f] }, S)
        end
        function S:readSetting(k) return self.data[k] end
        function S:saveSetting(k, v) self.data[k] = v; return self end
        function S:isTrue(k) return self.data[k] == true end
        function S:toggle(k) self.data[k] = not self.data[k]; return self end
        function S:flush() W.flushed = (W.flushed or 0) + 1 end
        return S
    end
    local function widget(name)
        return function() return new_class(name) end
    end
    P["ui/widget/infomessage"] = widget("infomessage")
    P["ui/widget/textviewer"] = widget("textviewer")
    P["ui/widget/container/widgetcontainer"] = widget("widgetcontainer")
    P["ui/widget/multiinputdialog"] = function()
        local D = new_class("multiinputdialog")
        function D:getFields() return self.values or { "", "", "", "" } end
        function D:onShowKeyboard() end
        return D
    end
    P["ui/uimanager"] = function()
        return {
            show = function(_, w) W.shown[#W.shown + 1] = w end,
            close = function(_, w) W.closed[#W.closed + 1] = w end,
            -- jobs wait until a test runs them (`w:run_scheduled()`), like KOReader's timer
            scheduleIn = function(_, delay, f) W.jobs[#W.jobs + 1] = { delay = delay, fn = f } end,
            unschedule = function(_, f)
                for i = #W.jobs, 1, -1 do
                    if W.jobs[i].fn == f then table.remove(W.jobs, i) end
                end
            end,
        }
    end
    P["ui/network/manager"] = function()
        return {
            isOnline = function() return W.online end,
            runWhenOnline = function(_, f) W.queued = f end,
        }
    end
    P["socketutil"] = function()
        return {
            set_timeout = function(_, a, b) W.timeouts[#W.timeouts + 1] = { a, b } end,
            reset_timeout = function() W.timeouts[#W.timeouts + 1] = "reset" end,
        }
    end
    P["device"] = function() return { model = W.device_model } end
    P["util"] = function()
        return { partialMD5 = function() return string.rep("a", 32) end }
    end
    P["ltn12"] = function() return ltn12 end
    local function socket_http()
        return {
            request = function(r)
                local chunks = {}
                if r.source then
                    while true do
                        local c = r.source()
                        if not c then break end
                        chunks[#chunks + 1] = c
                    end
                end
                local req = { method = r.method, url = r.url, headers = r.headers, body = table.concat(chunks) }
                W.requests[#W.requests + 1] = req
                if not W.http.script then
                    error("no http script set: unexpected request " .. tostring(r.method) .. " " .. tostring(r.url))
                end
                local code, body = W.http.script(req)
                if code == nil or type(code) == "string" then return nil, body or code end
                if type(body) == "string" and body ~= "" then r.sink(body) end
                return 1, code, {}, "HTTP/1.1 " .. code
            end,
        }
    end
    P["socket.http"] = socket_http
    P["ssl.https"] = socket_http
    P["version"] = function()
        return { getCurrentRevision = function() return opts.koreader_version or "v2026.07.2-test" end }
    end

    --- Make the preloads visible to `require` and forget the plugin's own modules, so each world starts clean.
    function W:install()
        for name in pairs(package.loaded) do
            if name:match("^bookx") then package.loaded[name] = nil end
        end
        for name, loader in pairs(self.preload) do
            package.loaded[name] = nil
            package.preload[name] = loader
        end
        package.path = K.plugin_dir .. "/?.lua;" .. package.path
        return self
    end

    --- The ReaderUI of a book: what the plugin reads from `self.ui`.
    function W:reader_ui(o)
        o = o or {}
        local ui = {
            menu = { registerToMainMenu = function(_, p) W.registered = p end },
            document = o.document or { file = "/mnt/onboard/Book.epub" },
            doc_settings = {
                readSetting = function(_, k)
                    if k == "partial_md5_checksum" then return o.md5 or string.rep("c", 32) end
                    if k == "doc_props" then return o.props or { title = "Real Book", authors = "A\nB" } end
                end,
            },
            annotation = { annotations = o.annotations or {} },
        }
        if o.no_annotation then ui.annotation = nil end
        return ui
    end

    --- main.lua loaded and instantiated exactly like ReaderUI + PluginLoader do: the constructor table holds
    --- dialog, view, ui and document, and `on*` handlers of the class are wrapped by the handler sandbox.
    function W:plugin(o)
        o = o or {}
        self:install()
        local Bookx = dofile(K.plugin_dir .. "/main.lua")
        for key, value in pairs(Bookx) do
            if type(key) == "string" and key:sub(1, 2) == "on" and type(value) == "function" then
                Bookx[key] = setmetatable({ f = value, name = key }, {
                    __call = function(t, module, ...)
                        local ok, re = pcall(t.f, module, ...)
                        if ok then return re end
                        W.logs[#W.logs + 1] = { level = "err", text = "An error occurred while executing handler " .. t.name .. ": " .. tostring(re) }
                        return false
                    end,
                })
            end
        end
        local ui = o.ui or self:reader_ui(o)
        self.ui = ui
        local args = { dialog = {}, view = {}, ui = ui, document = ui.document }
        for k, v in pairs(o.extra_args or {}) do args[k] = v end
        self.Bookx = Bookx
        local p = Bookx:new(args)
        self.p = p
        return p
    end

    --- Saves a configured account (what Server and account + Save leave behind).
    function W:configure(p, server)
        p.settings:saveSetting("server", server or "http://192.168.1.5:8080/bookx")
        p.settings:saveSetting("user", "ann")
        p.settings:saveSetting("key", "5ebe2294ecd0e0f08eab7690d2a6ee69")
        return p
    end

    --- Presses a dialog button by label.
    function W.press(dialog, label)
        for _, row in ipairs(dialog.buttons) do
            for _, btn in ipairs(row) do
                if btn.text == label then btn.callback(); return true end
            end
        end
        return false
    end

    --- The menu entry labelled `text` (flat), from addToMainMenu.
    function W:menu_entry(p, text)
        local items = {}
        p:addToMainMenu(items)
        for _, e in ipairs(items.bookx.sub_item_table) do
            if e.text == text then return e end
        end
    end

    return W
end

K.deep_copy = deep_copy

--- A tiny test runner: `T.check`, `T.eq`, `T.test(name, fn)`; failures are collected and the process exits non-zero.
function K.runner(title)
    local T = { failures = 0, count = 0, tests = 0, current = "" }
    function T.check(cond, msg)
        T.count = T.count + 1
        if not cond then
            T.failures = T.failures + 1
            io.stderr:write(string.format("FAIL [%s]: %s\n", T.current, msg))
        end
        return cond
    end
    function T.eq(a, b, msg)
        return T.check(a == b, string.format("%s (got %s, wanted %s)", msg, tostring(a), tostring(b)))
    end
    function T.has(text, needle, msg)
        return T.check(type(text) == "string" and text:find(needle, 1, true) ~= nil,
            string.format("%s (%q does not contain %q)", msg, tostring(text), needle))
    end
    function T.test(name, fn)
        T.current = name
        T.tests = T.tests + 1
        local ok, err = xpcall(fn, debug.traceback)
        if not ok then
            T.failures = T.failures + 1
            io.stderr:write(string.format("ERROR [%s]: %s\n", name, tostring(err)))
        end
    end
    function T.finish()
        if T.failures > 0 then
            io.stderr:write(string.format("%s: %d of %d checks failed (%d tests)\n", title, T.failures, T.count, T.tests))
            os.exit(1)
        end
        print(string.format("%s: ok, %d checks in %d tests", title, T.count, T.tests))
    end
    return T
end

return K
