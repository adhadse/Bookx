-- Replays real KOReader sidecars (metadata.*.lua) through the plugin logic (bookxsync.lua) to a Bookx
-- server, the way KOReader would: `lua replay_sidecar.lua <metadata.lua>...` with BOOKX_URL, BOOKX_USER
-- and BOOKX_KEY set. curl is the transport. Used by the Rust test `a_real_shaped_sidecar_keeps_every_look_over_the_wire`.
package.path = "../bookx.koplugin/?.lua;" .. package.path
local Sync = require("bookxsync")
local url, user, key = os.getenv("BOOKX_URL"), os.getenv("BOOKX_USER"), os.getenv("BOOKX_KEY")
local function curl(method, u, headers, body)
    local f = os.tmpname()
    local h = io.open(f, "w"); h:write(body); h:close()
    local args = { "curl", "-s", "-S", "--max-time", "30", "-X", method, "-o", f .. ".out", "-w", "%{http_code}",
        "--data-binary", "@" .. f }
    for k, v in pairs(headers) do args[#args + 1] = "-H"; args[#args + 1] = "'" .. k .. ": " .. v .. "'" end
    args[#args + 1] = "'" .. u .. "'"
    local p = io.popen(table.concat(args, " "))
    local code = tonumber(p:read("*a")); p:close()
    local o = io.open(f .. ".out", "r")
    local text = o and o:read("*a") or ""
    if o then o:close() end
    os.remove(f); os.remove(f .. ".out")
    return code, text
end
local state = {}
for i = 1, #arg do
    local t = dofile(arg[i])
    local ok, r = Sync.sync_book({
        server = url, user = user, key = key, device = "Replay", device_id = "replay",
        document = t.partial_md5_checksum, title = t.doc_props.title, authors = t.doc_props.authors or "",
        now = os.time(), state = state, request = curl, items = t.annotations,
    })
    print(t.doc_props.title, ok, ok and Sync.summary_text(r) or r)
    if not ok then os.exit(1) end
end
