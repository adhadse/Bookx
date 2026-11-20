-- Bookx Sync: KOReader glue (menu, settings, network). Own code, MIT licence.
-- The sync rules are in bookxsync.lua and are tested outside KOReader; this file only connects them
-- to KOReader's widgets. Its KOReader calls were checked by reading the frontend of KOReader v2026.07.2
-- (see docs/kobo-koreader-sync-runbook.md); it has not been run on a device yet.
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

-- A Lua error in a plugin callback is reported by KOReader as a crash. Keep every entry point
-- inside pcall, log the details for the runbook and show one short message instead of panicking.
local function report_error(where, err)
    -- even the error reporting must not throw: an error here was the panic on the device
    pcall(function() logger.err("Bookx Sync error in " .. where .. ": " .. tostring(err)) end)
    pcall(function()
        UIManager:show(InfoMessage:new{
            text = _("Bookx sync stopped because of an unexpected error. The details are in the KOReader log."),
        })
    end)
end

local function guarded(where, fn, fallback)
    return function(...)
        local ok, a, b, c = pcall(fn, ...)
        if ok then return a, b, c end
        pcall(report_error, where, a)
        return fallback
    end
end

function Bookx:init()
    local ok, err = pcall(function()
        self.settings = LuaSettings:open(DataStorage:getSettingsDir() .. "/bookx_sync.lua")
        self.state_file = LuaSettings:open(STATE_FILE)
        self.state = self.state_file:readSetting("state") or {}
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
    local ok, code = pcall(function()
        local _, c = http.request({
            method = method, url = url, headers = h,
            source = ltn12.source.string(body), sink = ltn12.sink.table(sink),
        })
        return c
    end)
    socketutil:reset_timeout()
    if not ok or type(code) ~= "number" then
        return nil, tostring(code)
    end
    return code, table.concat(sink)
end

function Bookx:document()
    local d = self.ui.doc_settings and self.ui.doc_settings:readSetting("partial_md5_checksum")
    if d then return d end
    local ok, util = pcall(require, "util")
    if ok and util.partialMD5 and self.ui.document then
        return util.partialMD5(self.ui.document.file)
    end
end

function Bookx:items()
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

function Bookx:syncNow(quiet)
    if not self.ui.document then return end
    if not self:configured() then
        if not quiet then UIManager:show(InfoMessage:new{ text = _("Set the Bookx server and your user name first.") }) end
        return
    end
    local doc, items = self:document(), self:items()
    if not doc or not items then
        if not quiet then
            UIManager:show(InfoMessage:new{ text = _("This KOReader version does not offer its annotations to plugins.") })
        end
        return
    end
    local props = self.ui.doc_settings and self.ui.doc_settings:readSetting("doc_props") or {}
    local now = os.time()
    local ran, success, detail = pcall(Sync.sync_book, {
        server = self.settings:readSetting("server"),
        user = self.settings:readSetting("user"),
        key = self.settings:readSetting("key"),
        device = self.settings:readSetting("device_name") or require("device").model or "KOReader",
        device_id = self.settings:readSetting("device_id"),
        document = doc, title = props.title or "", authors = props.authors or "",
        items = items, now = now, state = self.state, request = request,
    })
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

-- network first when needed, then sync
function Bookx:syncWithNetwork(quiet)
    if NetworkMgr:isOnline() then
        self:syncNow(quiet)
    elseif not quiet then
        NetworkMgr:runWhenOnline(guarded("sync after the network is back", function()
            self:syncNow(false)
        end))
    end
end

function Bookx:onCloseDocument()
    if self.settings:isTrue("auto") then self:syncWithNetwork(true) end
end

function Bookx:onSuspend()
    if self.settings:isTrue("auto") and self.ui.document then self:syncWithNetwork(true) end
end

function Bookx:showInbox()
    local doc = self:document()
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

function Bookx:info(text)
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
        self:info(_("Set the Bookx server, user name and password first (Server and account)."))
        return
    end
    local function run()
        local ran, ok, message = pcall(Sync.test_connection, {
            server = self.settings:readSetting("server"),
            user = self.settings:readSetting("user"),
            key = self.settings:readSetting("key"),
            request = request,
        })
        if not ran then ok, message = false, _("Cannot reach the server") end
        self:recordTest(ok, message)
        self:info(_(message))
    end
    if NetworkMgr:isOnline() then run() else NetworkMgr:runWhenOnline(guarded("test connection callback", run)) end
end

function Bookx:registerAccount(f, on_done)
    local server, user, pw = f[1], f[2], f[3]
    if server == "" or user == "" or pw == "" then
        self:info(_("To register, fill in the server address, a user name and a password."))
        return
    end
    local function run()
        local key = md5(pw)
        local ran, ok, message = pcall(Sync.register, {
            server = server, user = user, key = key, request = request })
        if not ran then ok, message = false, _("Cannot reach the server") end
        if not ok then
            self:info(_(message))
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
                    self:info(self:savedMessage())
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
        },
    }
end

-- KOReader calls these methods from its event loop: one uncaught Lua error is shown as a crash.
-- Wrap the methods here, at the boundary, so a bad setting, a missing widget or an old frontend
-- cannot panic the reader; the details go to the log and a short message goes to the user.
for _, name in ipairs({
    "document", "items", "save_state", "configured", "syncNow", "syncWithNetwork",
    "onCloseDocument", "onSuspend", "showInbox", "info", "has_key", "applyFields",
    "savedMessage", "recordTest", "testConnection", "registerAccount", "editSettings",
    "addToMainMenu",
}) do
    Bookx[name] = guarded("Bookx:" .. name, Bookx[name])
end

return Bookx
