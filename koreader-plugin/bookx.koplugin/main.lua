-- Bookx Sync: KOReader glue (menu, settings, network). Own code, MIT licence.
-- The sync rules are in bookxsync.lua and are tested outside KOReader; this file only connects them
-- to KOReader's widgets. Its KOReader calls were checked by reading the frontend of KOReader v2026.07.2
-- (see docs/archive/kobo-koreader-sync-runbook.md).
--
-- NOTE on names: KOReader builds a plugin with `Bookx:new{ dialog, view, ui, document }`, and the constructor
-- table becomes the instance, so `self.document`, `self.ui`, `self.view` and `self.dialog` are DATA. A method
-- with one of those names is shadowed and `self:bookId()` fails with "attempt to call method 'document' (a
-- table value)" (the first device crash). Methods here therefore never use those names (tests/scenarios.lua
-- checks it against every field KOReader passes).
local DataStorage = require("datastorage")
local InfoMessage = require("ui/widget/infomessage")
local LuaSettings = require("luasettings")
local MultiInputDialog = require("ui/widget/multiinputdialog")
local NetworkMgr = require("ui/network/manager")
local TextViewer = require("ui/widget/textviewer")
local UIManager = require("ui/uimanager")
local WidgetContainer = require("ui/widget/container/widgetcontainer")
local logger = require("logger")
local _ = require("gettext")

local plugin_dir = (debug.getinfo(1, "S").source or ""):match("^@(.*)/[^/]*$") or "."
package.path = plugin_dir .. "/?.lua;" .. package.path
local Sync = require("bookxsync")
local json = require("bookxjson")
local md5 = require("bookxmd5").md5

local Bookx = WidgetContainer:extend{
    name = "bookx",
    is_doc_only = false,
}

local STATE_FILE = DataStorage:getSettingsDir() .. "/bookx_sync_state.lua"

local unpack = table.unpack or unpack

-- A Lua error in a plugin callback is reported by KOReader as a crash. Keep every entry point inside
-- xpcall: the full traceback goes to KOReader's log (and into the diagnostics), the screen gets one short
-- message. `quiet` (automatic syncs) logs only; nobody is interrupted while reading.
local remember_error -- set in Bookx:init: keeps the last error for About / Diagnostics

local function report_error(where, err, traceback, quiet)
    local text = tostring(err)
    -- even the error reporting must not throw: an error here was the panic on the device
    pcall(function()
        logger.err("Bookx Sync error in " .. where .. ": " .. text .. "\n" .. tostring(traceback or ""))
    end)
    pcall(function()
        if remember_error then
            remember_error({ time = os.time(), where = where, message = text, traceback = traceback })
        end
    end)
    if quiet then return end
    pcall(function()
        UIManager:show(InfoMessage:new{
            text = _("Bookx sync: unexpected error: ") .. Sync.short_error(text)
                .. "\n" .. _("Details: Bookx Sync > About / Diagnostics, and the KOReader log."),
        })
    end)
end

-- `quiet_arg`: the position of an argument that is `true` for a silent automatic run (see syncNow)
local function guarded(where, fn, fallback, quiet_arg, always_quiet)
    return function(...)
        local args, n = { ... }, select("#", ...)
        local results
        local ok, err = xpcall(function() results = { fn(unpack(args, 1, n)) } end, function(e)
            return { message = e, traceback = debug.traceback(tostring(e), 2) }
        end)
        if ok then return unpack(results, 1, 3) end
        local info = type(err) == "table" and err or { message = err }
        pcall(report_error, where, info.message, info.traceback,
            always_quiet or (quiet_arg and args[quiet_arg] == true))
        return fallback
    end
end

-- Runs a piece of the sync logic. A Lua error inside it is logged with its traceback and remembered, and
-- comes back as `false, "unexpected error: ..."` like any other failure, so the caller shows one message.
local function logic(where, fn, ctx)
    local results
    local ok, err = xpcall(function() results = { fn(ctx) } end, function(e)
        return { message = e, traceback = debug.traceback(tostring(e), 2) }
    end)
    if ok then return true, results[1], results[2] end
    pcall(report_error, where, err.message, err.traceback, true)
    return false, "unexpected error: " .. Sync.short_error(err.message, 100)
end

function Bookx:init()
    local ok, err = pcall(function()
        self.settings = LuaSettings:open(DataStorage:getSettingsDir() .. "/bookx_sync.lua")
        self.state_file = LuaSettings:open(STATE_FILE)
        self.state = self.state_file:readSetting("state") or {}
        remember_error = function(entry)
            self.state.last_error = entry
            self:save_state()
        end
        if not self.settings:readSetting("device_id") then
            self.settings:saveSetting("device_id", string.format("%08x%08x", os.time() % 4294967296, math.random(0, 2 ^ 31)))
            self.settings:flush()
        end
        self.ui.menu:registerToMainMenu(self)
    end)
    if not ok then report_error("Bookx:init", err) end
end

-- one HTTP request through KOReader's socket libraries; returns status, body
local function request(method, url, headers, body)
    local ltn12 = require("ltn12")
    local socketutil = require("socketutil")
    local http = url:match("^https:") and require("ssl.https") or require("socket.http")
    local sink = {}
    socketutil:set_timeout(10, 30)
    local h = {}
    for k, v in pairs(headers) do h[k] = v end
    h["Content-Length"] = tostring(#body)
    local ok, res, code = pcall(http.request, {
        method = method, url = url, headers = h,
        source = ltn12.source.string(body), sink = ltn12.sink.table(sink),
    })
    socketutil:reset_timeout()
    -- LuaSocket answers `1, code, headers, status` on success and `nil, "reason"` on a socket error
    if not ok then
        logger.warn("Bookx Sync: request failed: " .. tostring(res))
        return nil, tostring(res)
    end
    if type(code) ~= "number" then
        return nil, tostring(code or res)
    end
    return code, table.concat(sink)
end

function Bookx:bookId()
    local d = self.ui.doc_settings and self.ui.doc_settings:readSetting("partial_md5_checksum")
    if d then return d end
    local ok, util = pcall(require, "util")
    if ok and util.partialMD5 and self.ui.document then
        return util.partialMD5(self.ui.document.file)
    end
end

function Bookx:bookAnnotations()
    local a = self.ui.annotation and self.ui.annotation.annotations
    if type(a) ~= "table" then return nil end
    return a
end

function Bookx:save_state()
    self.state_file:saveSetting("state", self.state)
    self.state_file:flush()
end

function Bookx:configured()
    return (self.settings:readSetting("server") or "") ~= ""
        and (self.settings:readSetting("user") or "") ~= ""
        and (self.settings:readSetting("key") or "") ~= ""
end

local function koreader_version()
    local v = "unknown"
    pcall(function()
        local Version = require("version")
        local n = Version:getCurrentRevision()
        v = (n or "unknown"):gsub("^v", "")
    end)
    return v
end

-- who we are, for every request (the server shows it in Bookx under Sync, Connected devices)
function Bookx:identity()
    return {
        server = self.settings:readSetting("server"),
        user = self.settings:readSetting("user"),
        key = self.settings:readSetting("key"),
        device = self.settings:readSetting("device_name") or require("device").model or "KOReader",
        device_id = self.settings:readSetting("device_id"),
        koreader = koreader_version(),
    }
end

function Bookx:syncNow(quiet)
    if not self.ui.document then return end
    if not self:configured() then
        if not quiet then UIManager:show(InfoMessage:new{ text = _("Set the Bookx server and your user name first.") }) end
        return
    end
    local doc, items = self:bookId(), self:bookAnnotations()
    if not doc or not items then
        if not quiet then
            UIManager:show(InfoMessage:new{ text = _("This KOReader version does not offer its annotations to plugins.") })
        end
        return
    end
    local props = self.ui.doc_settings and self.ui.doc_settings:readSetting("doc_props") or {}
    local now = os.time()
    local ctx = self:identity()
    ctx.document, ctx.title, ctx.authors = doc, props.title or "", props.authors or ""
    ctx.items, ctx.now, ctx.state, ctx.request = items, now, self.state, request
    local ran, success, detail = logic("Sync.sync_book", Sync.sync_book, ctx)
    local ok, message
    if not ran then
        ok, message = false, tostring(success)
    elseif success then
        ok, message = true, Sync.summary_text(detail)
    else
        ok, message = false, tostring(detail)
    end
    self.state.last = { time = now, ok = ok, message = message }
    self:save_state()
    logger.info("Bookx Sync:", ok, message)
    if not quiet then
        UIManager:show(InfoMessage:new{ text = ok and message or (_("Bookx sync failed: ") .. message) })
    end
end

-- Network first when needed, then sync. A silent (automatic) run never switches Wi-Fi on; when Wi-Fi is off it
-- remembers that something is waiting (shown in the menu and in About / Diagnostics) and syncs when a book is
-- next open with Wi-Fi on.
function Bookx:syncWithNetwork(quiet)
    if NetworkMgr:isOnline() then
        self:syncNow(quiet)
    elseif not quiet then
        NetworkMgr:runWhenOnline(guarded("sync after the network is back", function()
            self:syncNow(false)
        end))
    else
        self:markWaiting("Wi-Fi is off")
    end
end

function Bookx:markWaiting(why)
    if not self:configured() then return end
    self.state.waiting = { time = os.time(), why = why }
    self:save_state()
    logger.info("Bookx Sync: waiting:", why)
end

-- the pending automatic run after a highlight (one job, restarted by every change)
local AUTO_DELAY = 30

function Bookx:cancelAuto()
    if self._auto_job then
        UIManager:unschedule(self._auto_job)
        self._auto_job = nil
    end
end

function Bookx:scheduleAuto()
    self:cancelAuto()
    self._auto_job = guarded("automatic sync after a change", function()
        self._auto_job = nil
        if self.ui.document then self:syncWithNetwork(true) end
    end)
    UIManager:scheduleIn(AUTO_DELAY, self._auto_job)
end

function Bookx:onCloseDocument()
    self:cancelAuto()
    if self.settings:isTrue("auto") then self:syncWithNetwork(true) end
end

function Bookx:onSuspend()
    self:cancelAuto()
    if self.settings:isTrue("auto") and self.ui.document then self:syncWithNetwork(true) end
end

-- A highlight, note or bookmark was added, changed or deleted: sync shortly after the last change.
function Bookx:onAnnotationsModified()
    if self.settings:isTrue("auto") and self:configured() then self:scheduleAuto() end
end

-- A book was opened: send what waited for Wi-Fi, and pick up what Bookx has for it.
function Bookx:onReaderReady()
    if self.settings:isTrue("auto") and NetworkMgr:isOnline() then self:syncNow(true) end
end

-- Wi-Fi came up while a book is open and something waited.
function Bookx:onNetworkConnected()
    if self.settings:isTrue("auto") and self.state.waiting and self.ui.document then self:syncNow(true) end
end

function Bookx:showInbox()
    local doc = self:bookId()
    local list = doc and Sync.inbox_list(self.state, doc) or {}
    local text
    if #list == 0 then
        text = _("No notes from Bookx for this book yet.")
    else
        local parts = {}
        for _, r in ipairs(list) do
            local p = "\xE2\x80\x9C" .. r.text .. "\xE2\x80\x9D"
            if r.note ~= "" then p = p .. "\n" .. r.note end
            parts[#parts + 1] = p
        end
        text = table.concat(parts, "\n\n")
    end
    UIManager:show(TextViewer:new{ title = _("Notes from Bookx"), text = text })
end

function Bookx:showInfo(text)
    UIManager:show(InfoMessage:new{ text = text })
end

function Bookx:has_key()
    return (self.settings:readSetting("key") or "") ~= ""
end

-- Stores what the form holds. The password is never written: only its md5 (`key`), and only when the
-- field was filled; an empty field keeps the saved key. `new_key` overrides (used after Register).
function Bookx:applyFields(f, new_key)
    local before = (self.settings:readSetting("server") or "") .. "\0" .. (self.settings:readSetting("user") or "")
        .. "\0" .. (self.settings:readSetting("key") or "")
    self.settings:saveSetting("server", f[1])
    self.settings:saveSetting("user", f[2])
    local key = new_key or (f[3] ~= "" and md5(f[3]) or nil)
    if key then self.settings:saveSetting("key", key) end
    self.settings:saveSetting("device_name", f[4] ~= "" and f[4] or nil)
    local after = f[1] .. "\0" .. f[2] .. "\0" .. (self.settings:readSetting("key") or "")
    if before ~= after then
        self.state.test = nil
        self:save_state()
    end
    self.settings:flush()
end

function Bookx:savedMessage()
    local text = _("Saved. Password is kept as a hash; the field will be empty next time.")
    text = text .. "\n" .. _("Server: ") .. tostring(self.settings:readSetting("server") or "")
        .. "\n" .. _("User: ") .. tostring(self.settings:readSetting("user") or "")
    if not self:has_key() then text = text .. "\n" .. _("No password saved yet.") end
    return text
end

function Bookx:recordTest(ok, message)
    self.state.test = { time = os.time(), ok = ok, message = message }
    self:save_state()
    logger.info("Bookx Sync test:", ok, message)
end

function Bookx:testConnection()
    if not self:configured() then
        self:showInfo(_("Set the Bookx server, user name and password first (Server and account)."))
        return
    end
    local function run()
        local ctx = self:identity()
        ctx.request = request
        local ran, ok, message = logic("Sync.test_connection", Sync.test_connection, ctx)
        if not ran then ok, message = false, ok end
        self:recordTest(ok, message)
        self:showInfo(_(message))
    end
    if NetworkMgr:isOnline() then run() else NetworkMgr:runWhenOnline(guarded("test connection callback", run)) end
end

function Bookx:registerAccount(f, on_done)
    local server, user, pw = f[1], f[2], f[3]
    if server == "" or user == "" or pw == "" then
        self:showInfo(_("To register, fill in the server address, a user name and a password."))
        return
    end
    local function run()
        local key = md5(pw)
        local ctx = self:identity()
        ctx.server, ctx.user, ctx.key, ctx.request = server, user, key, request
        local ran, ok, message = logic("Sync.register", Sync.register, ctx)
        if not ran then ok, message = false, ok end
        if not ok then
            self:showInfo(_(message))
            return
        end
        self:applyFields(f, key)
        if on_done then on_done() end
        self:testConnection()
    end
    if NetworkMgr:isOnline() then run() else NetworkMgr:runWhenOnline(guarded("register callback", run)) end
end

function Bookx:editSettings()
    local dialog
    local function fields()
        local f = dialog:getFields()
        return { f[1] or "", f[2] or "", f[3] or "", f[4] or "" }
    end
    local pw_hint = self:has_key() and _("Saved \xE2\x80\x94 leave empty to keep")
        or _("Progress-sync password (not saved yet)")
    dialog = MultiInputDialog:new{
        title = _("Bookx server"),
        fields = {
            { text = self.settings:readSetting("server") or "", hint = "http://192.168.1.20:8765/bookx", description = _("Server address: the same address as in Progress sync, ending in /bookx") },
            { text = self.settings:readSetting("user") or "", hint = _("Progress-sync user name"), description = _("User name of your progress-sync account (not the Share Library password)") },
            { text = "", hint = pw_hint, text_type = "password", description = self:has_key()
                and _("Password: saved as a one-way hash, so this field is empty. Leave it empty to keep the saved one")
                or _("Password of that account. Only a one-way hash is stored") },
            { text = self.settings:readSetting("device_name") or "", hint = _("Name of this device"), description = _("Device name: only a label shown in Bookx") },
        },
        buttons = {
            {
                { text = _("Cancel"), callback = guarded("Cancel button", function()
                    UIManager:close(dialog)
                end) },
                { text = _("Save"), is_enter_default = true, callback = guarded("Save button", function()
                    self:applyFields(fields())
                    UIManager:close(dialog)
                    self:showInfo(self:savedMessage())
                end) },
            },
            {
                { text = _("Test connection"), callback = guarded("Test connection button", function()
                    self:applyFields(fields())
                    UIManager:close(dialog)
                    self:testConnection()
                end) },
                { text = _("Register account"), callback = guarded("Register account button", function()
                    self:registerAccount(fields(), guarded("close after register", function()
                        UIManager:close(dialog)
                    end))
                end) },
            },
        },
    }
    UIManager:show(dialog)
    dialog:onShowKeyboard()
end

function Bookx:diagnosticsText()
    local koreader = "?"
    pcall(function()
        local Version = require("version")
        koreader = Version:getCurrentRevision() or "?"
    end)
    return Sync.diagnostics({
        koreader = koreader, device = require("device").model,
        server = self.settings:readSetting("server"), user = self.settings:readSetting("user"),
        has_key = self:has_key(), auto = self.settings:isTrue("auto"), configured = self:configured(),
        state = self.state, now = os.time(),
    })
end

function Bookx:showDiagnostics()
    UIManager:show(TextViewer:new{
        title = _("Bookx Sync: About / Diagnostics"),
        text = self:diagnosticsText(),
        buttons_table = {
            {
                { text = _("Copy diagnostics"), callback = guarded("Copy diagnostics button", function()
                    self:copyDiagnostics()
                end) },
            },
        },
    })
end

-- Writes the diagnostics to a file in KOReader's settings folder (plus the clipboard when KOReader has one).
function Bookx:copyDiagnostics()
    local text = self:diagnosticsText()
    local path = DataStorage:getSettingsDir() .. "/bookx_diagnostics.txt"
    local f, err = io.open(path, "w")
    if not f then
        self:showInfo(_("Could not write the diagnostics: ") .. tostring(err))
        return
    end
    f:write(text, "\n")
    f:close()
    pcall(function()
        local Device = require("device")
        if Device.input and Device.input.setClipboardText then Device.input.setClipboardText(text) end
    end)
    self:showInfo(_("Diagnostics saved to: ") .. path)
end

function Bookx:addToMainMenu(menu_items)
    menu_items.bookx = {
        text = _("Bookx Sync"),
        sorting_hint = "more_tools",
        sub_item_table = {
            {
                text_func = guarded("menu status", function()
                    return Sync.status_line(self.state, os.time(), self:configured())
                end, ""),
                keep_menu_open = true,
                callback = function() end,
            },
            { text = _("Server and account"), keep_menu_open = true,
                callback = guarded("menu: Server and account", function() self:editSettings() end) },
            { text = _("Test connection"),
                callback = guarded("menu: Test connection", function() self:testConnection() end) },
            {
                text = _("Sync now"),
                enabled_func = guarded("menu: Sync now enabled", function() return self.ui.document ~= nil end, false),
                callback = guarded("menu: Sync now", function() self:syncWithNetwork(false) end),
            },
            {
                text = _("Sync automatically"),
                checked_func = guarded("menu: Sync automatically checked", function()
                    return self.settings:isTrue("auto")
                end, false),
                callback = guarded("menu: Sync automatically", function()
                    self.settings:toggle("auto")
                    self.settings:flush()
                end),
            },
            {
                text = _("Notes from Bookx for this book"),
                enabled_func = guarded("menu: Notes enabled", function() return self.ui.document ~= nil end, false),
                callback = guarded("menu: Notes from Bookx", function() self:showInbox() end),
            },
            { text = _("About / Diagnostics"), keep_menu_open = true,
                callback = guarded("menu: About / Diagnostics", function() self:showDiagnostics() end) },
        },
    }
end

-- KOReader calls these methods from its event loop: one uncaught Lua error is shown as a crash.
-- Wrap the methods here, at the boundary, so a bad setting, a missing widget or an old frontend
-- cannot panic the reader; the details go to the log and a short message goes to the user.
-- Only the entry points are wrapped; helpers (bookId, configured, save_state, ...) let an error rise to the
-- entry point, which reports it once with the real message instead of hiding it behind a wrong answer.
for _, name in ipairs({
    "syncNow", "syncWithNetwork", "onCloseDocument", "onSuspend", "onAnnotationsModified", "onReaderReady",
    "onNetworkConnected", "markWaiting", "showInbox", "testConnection",
    "registerAccount", "editSettings", "addToMainMenu", "showDiagnostics", "copyDiagnostics",
}) do
    -- the automatic hooks and `syncNow(true)` / `syncWithNetwork(true)` are silent when they fail
    local quiet_arg = (name == "syncNow" or name == "syncWithNetwork") and 2 or nil
    local always_quiet = (name == "onCloseDocument" or name == "onSuspend" or name == "onAnnotationsModified"
        or name == "onReaderReady" or name == "onNetworkConnected" or name == "markWaiting")
    Bookx[name] = guarded("Bookx:" .. name, Bookx[name], nil, quiet_arg, always_quiet)
end

return Bookx
