-- The real main.lua (menu, dialogs, Register, Test connection, Sync now) in the fake KOReader, with the
-- HTTP transport replaced by curl against a REAL Bookx server. Started by the Rust test
-- `the_real_plugin_registers_and_syncs_against_a_real_server`; needs BOOKX_URL (http://host:port/bookx) and curl.
local here = (arg and arg[0] or "live.lua"):match("^(.*)/[^/]*$") or "."
local K = dofile(here .. "/fake_koreader.lua")
local T = K.runner("live")
local url = assert(os.getenv("BOOKX_URL"), "BOOKX_URL")

local function curl(req)
    local f = os.tmpname()
    local h = io.open(f, "wb"); h:write(req.body); h:close()
    local args = { "curl", "-s", "--max-time", "20", "-X", req.method, "-o", f .. ".out", "-w", "%{http_code}",
        "--data-binary", "@" .. f }
    for k, v in pairs(req.headers) do args[#args + 1] = "-H"; args[#args + 1] = "'" .. k .. ": " .. v .. "'" end
    args[#args + 1] = "'" .. req.url .. "'"
    local p = io.popen(table.concat(args, " "))
    local code = tonumber(p:read("*a")); p:close()
    local o = io.open(f .. ".out", "rb"); local text = o and o:read("*a") or ""; if o then o:close() end
    os.remove(f); os.remove(f .. ".out")
    if not code or code == 0 then return nil, "connection refused" end
    return code, text
end

local fixture = dofile(here .. "/fixtures/kobo_sidecar_excerpt.lua")
local items = {}
for _, a in ipairs(fixture.annotations) do items[#items + 1] = a end
items[#items + 1] = { datetime = "2026-11-21 10:00:00", text = "latin-1 caf\233 and \255 bytes\r\1", chapter = "Ch\nx",
    pos0 = "/live/p[1]", pos1 = "/live/p[1]/t", color = "teal", drawer = "wavy", note = "tab\tnote" }

T.test("register, test, sync, resync, wrong password, unreachable server", function()
    local w = K.world()
    w.http.script = function(req) return curl(req) end
    local p = w:plugin({ annotations = items })
    p:editSettings()
    local d = w.shown[#w.shown]
    d.values = { url, "zoe", "secret", "Live Kobo" }
    K.world().press(d, "Register account")
    T.eq(w:last_text(), "Connected as zoe", "register + test against the real server")
    T.eq(w:log_of("err"), "", "no error logged")
    w.shown = {}
    p:syncNow(false)
    T.has(w:last_text(), "Sent " .. #items, "everything accepted: " .. w:last_text())
    p:syncNow(false)
    T.has(w:last_text(), "Sent 0", "nothing sent twice")
    -- wrong password
    local saved = p.settings:readSetting("key")
    p.settings:saveSetting("key", "00000000000000000000000000000000")
    p:syncNow(false)
    T.has(w:last_text(), "wrong user name or password", "wrong key refused")
    p:testConnection()
    T.eq(w:last_text(), "Wrong user name or password", "test says so")
    p.settings:saveSetting("key", saved)
    -- annotation sync needs the right server; an address nobody listens on
    p.settings:saveSetting("server", "http://127.0.0.1:9/bookx")
    p:syncNow(false)
    T.has(w:last_text(), "cannot reach 127.0.0.1:9", "unreachable is named: " .. w:last_text())
    -- a path that is not Bookx's
    p.settings:saveSetting("server", (url:gsub("/bookx$", "/nothing-here")))
    p:syncNow(false)
    T.check(w:last_text():find("Bookx sync failed", 1, true) ~= nil, "a wrong path is a clear failure: " .. w:last_text())
    T.eq(w:log_of("err"), "", "no Lua error through all of it")
end)

T.finish()
