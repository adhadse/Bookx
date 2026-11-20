-- The sync logic of Bookx Sync, without any KOReader user interface so it can be tested outside
-- KOReader (own code, MIT licence).
--
-- What it does for one book:
--   1. turns KOReader's annotations into the wire format of Bookx's server,
--   2. compares them with what was sent last time (kept in `state`): new or changed ones are sent,
--      ones that vanished are sent as tombstones (`deleted = true`),
--   3. PUTs them to  <server>/annotations/<document>  (server = http://<ip>:<port>/bookx) and reads the answer,
--   4. keeps the annotations that were made in Bookx in `state.inbox` to show in the menu.
-- A failed request changes nothing in `state`, so the next sync simply repeats it (the server
-- treats a repeated request as no change).
local json = require("bookxjson")

local M = {}

M.MAX_ITEMS_PER_REQUEST = 500

-- names go to the server without control characters; KOReader keeps several authors in one string
-- separated by new lines ("Noah Gift\nAlfredo Deza"), which the server refuses
function M.clean_name(s)
    s = tostring(s or ""):gsub("[\r\n]+", "; "):gsub("%c", " ")
    return s
end

local function trim(s)
    return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

--- "2024-05-17 20:15:03" (device local time) -> seconds since the epoch, or nil.
function M.parse_datetime(s)
    if type(s) ~= "string" then return nil end
    local y, mo, d, h, mi, se = s:match("^(%d+)-(%d+)-(%d+)[ T](%d+):(%d+):?(%d*)")
    if not y then return nil end
    return os.time({ year = tonumber(y), month = tonumber(mo), day = tonumber(d),
        hour = tonumber(h), min = tonumber(mi), sec = tonumber(se) or 0 })
end

--- Two 32-bit hashes of `s` as 16 hex digits (not a security measure: an id only needs to be stable).
function M.hash(s)
    local h1, h2 = 5381, 52711
    for i = 1, #s do
        local b = s:byte(i)
        h1 = (h1 * 33 + b) % 4294967296
        h2 = (h2 * 131 + b + i) % 4294967291
    end
    return string.format("%08x%08x", h1, h2)
end

-- an xpointer (EPUB) is a string; a PDF position is a table, written in a stable way
local function pos_text(p)
    if type(p) == "table" then
        return string.format("%s:%s:%s", tostring(p.page or ""), tostring(p.x or ""), tostring(p.y or ""))
    end
    return tostring(p or "")
end

local drawers = { lighten = true, underscore = true, strikeout = true, invert = true }

--- One KOReader annotation (an entry of `ui.annotation.annotations`) as a wire annotation.
function M.to_wire(item, now)
    local pos0 = pos_text(item.pos0)
    local pos1 = pos_text(item.pos1)
    local page = item.pageno
    if type(page) ~= "number" and type(item.page) == "number" then page = item.page end
    local created = M.parse_datetime(item.datetime) or 0
    local kind = (pos0 ~= "" and pos1 ~= "" and trim(item.text) ~= "") and "highlight" or "bookmark"
    local a = {
        id = "ko-" .. M.hash(tostring(item.datetime or "") .. "|" .. pos0 .. "|" .. pos1 .. "|" .. pos_text(item.page)),
        kind = kind,
        text = tostring(item.text or ""),
        note = tostring(item.note or ""),
        color = tostring(item.color or ""),
        drawer = drawers[item.drawer] and item.drawer or "",
        chapter = tostring(item.chapter or ""),
        pos0 = pos0,
        pos1 = pos1,
        created = math.min(created, now),
        updated = math.min(M.parse_datetime(item.datetime_updated) or created, now),
    }
    if type(page) == "number" then a.page = math.floor(page) end
    if kind == "bookmark" then a.text = "" end
    return a
end

--- What identifies the content of an annotation, to notice changes.
function M.fingerprint(a)
    return M.hash(table.concat({ a.text, a.note, a.color, a.drawer, a.kind }, "\31"))
end

--- Compares the current annotations with `synced` (id -> fingerprint of what the server has).
--- Returns the list to send and the new `synced` table that is valid once the send succeeded.
function M.diff(items, synced, now)
    local send, new_synced, seen = {}, {}, {}
    for _, item in ipairs(items) do
        local a = M.to_wire(item, now)
        if not seen[a.id] then
            seen[a.id] = true
            local fp = M.fingerprint(a)
            new_synced[a.id] = fp
            if synced[a.id] ~= fp then
                -- a change that KOReader did not date (a new note on an old highlight) counts as now
                if synced[a.id] ~= nil then a.updated = now end
                send[#send + 1] = a
            end
        end
    end
    for id in pairs(synced) do
        if not seen[id] then
            send[#send + 1] = { id = id, kind = "highlight", text = "", deleted = true, updated = now, created = 0 }
        end
    end
    table.sort(send, function(x, y) return x.id < y.id end)
    return send, new_synced
end

local function base(url)
    return (trim(url):gsub("/+$", ""))
end

--- The address new installs suggest; the umbrella endpoint of Bookx's server.
M.DEFAULT_PATH = "/bookx"

--- The URL of the annotation route of one book. An address with a path (`.../bookx`, or a reverse
--- proxy path) is used as it is; a bare host (no path) gets `/bookx`.
function M.base_url(server)
    local b = base(server)
    if not b:match("^https?://[^/]+/.") then b = b .. M.DEFAULT_PATH end
    return b
end

function M.build_url(server, document, since)
    return M.base_url(server) .. "/annotations/" .. document .. "?since=" .. tostring(since or 0)
end

--- The URL that checks a login: <server>/bookx/users/auth (same base rules as `build_url`).
function M.auth_url(server) return M.base_url(server) .. "/users/auth" end

--- The URL that creates an account.
function M.create_url(server) return M.base_url(server) .. "/users/create" end

local function is_timeout(err) return tostring(err):lower():find("timeout", 1, true) ~= nil end

--- Checks the saved login. ctx = { server, user, key, request }. Returns ok, message
--- (neither the key nor the password is ever part of a message).
function M.test_connection(ctx)
    local status, err = ctx.request("GET", M.auth_url(ctx.server), {
        ["Accept"] = "application/json", ["x-auth-user"] = ctx.user, ["x-auth-key"] = ctx.key }, "")
    if status == 200 then return true, "Connected as " .. tostring(ctx.user) end
    if status == nil then
        if is_timeout(err) then return false, "The server did not answer in time (timeout)" end
        return false, "Cannot reach the server"
    end
    if status == 401 then return false, "Wrong user name or password" end
    if status == 404 then return false, "Bookx sync is not enabled at that address (404)" end
    if status == 429 then return false, "Too many requests, try again in a minute" end
    return false, "The server answered " .. tostring(status)
end

--- Creates an account. ctx = { server, user, key (md5 of the password), request }. Returns ok, message.
function M.register(ctx)
    local body = json.encode({ username = ctx.user, password = ctx.key })
    local status, err = ctx.request("POST", M.create_url(ctx.server), {
        ["Accept"] = "application/json", ["Content-Type"] = "application/json" }, body)
    if status == 201 or status == 200 then return true, "Account " .. tostring(ctx.user) .. " created" end
    if status == nil then
        if is_timeout(err) then return false, "The server did not answer in time (timeout)" end
        return false, "Cannot reach the server"
    end
    if status == 402 then return false, "That name is taken: choose another name, or use the password of your existing account" end
    if status == 403 then return false, "Registration is closed on that server" end
    if status == 404 then return false, "Bookx sync is not enabled at that address (404)" end
    if status == 429 then return false, "Too many requests, try again in a minute" end
    return false, "The server answered " .. tostring(status)
end

--- ctx = {
---   server, user, key (md5 of the password), device, device_id,
---   document (partial md5), title, authors, items (KOReader annotations), now,
---   state (table, changed in place), request(method, url, headers, body) -> status, body }
--- Returns true, summary or false, message.
function M.sync_book(ctx)
    local state = ctx.state
    state.docs = state.docs or {}
    local doc = state.docs[ctx.document] or { synced = {}, seq = 0 }
    state.docs[ctx.document] = doc
    doc.synced = doc.synced or {}

    local send, new_synced = M.diff(ctx.items, doc.synced, ctx.now)
    local sent, deleted = 0, 0
    local headers = {
        ["Accept"] = "application/json",
        ["Content-Type"] = "application/json",
        ["x-auth-user"] = ctx.user,
        ["x-auth-key"] = ctx.key,
    }
    local answer
    local i = 1
    repeat
        local chunk = {}
        for k = i, math.min(i + M.MAX_ITEMS_PER_REQUEST - 1, #send) do chunk[#chunk + 1] = send[k] end
        i = i + M.MAX_ITEMS_PER_REQUEST
        local body = json.encode({
            origin = "koreader", device = ctx.device, device_id = ctx.device_id,
            title = M.clean_name(ctx.title), authors = M.clean_name(ctx.authors), annotations = chunk,
        })
        local url = M.build_url(ctx.server, ctx.document, doc.seq)
        local status, text = ctx.request("PUT", url, headers, body)
        if status ~= 200 then
            local msg = "server answered " .. tostring(status)
            local decoded = type(text) == "string" and json.decode(text) or nil
            if status == 401 then msg = "wrong user name or password"
            elseif status == 404 then msg = "the Bookx server has no KOReader sync here (switch it on in Sync, and check the address ends in /bookx)"
            elseif status == 429 then msg = "too many requests, try again in a minute"
            elseif type(decoded) == "table" and type(decoded.message) == "string" then msg = decoded.message end
            return false, msg
        end
        answer = json.decode(text)
        if type(answer) ~= "table" or type(answer.annotations) ~= "table" then
            return false, "the server's answer was not understood"
        end
        for _, a in ipairs(chunk) do
            if a.deleted then deleted = deleted + 1 else sent = sent + 1 end
        end
    until i > #send

    -- everything was accepted: remember it
    doc.synced = new_synced
    if type(answer.seq) == "number" then doc.seq = answer.seq end
    state.inbox = state.inbox or {}
    local inbox = state.inbox[ctx.document] or {}
    state.inbox[ctx.document] = inbox
    local received = 0
    for _, r in ipairs(answer.annotations) do
        if type(r) == "table" and r.origin == "bookx" and type(r.id) == "string" then
            if r.deleted then
                inbox[r.id] = nil
            else
                if inbox[r.id] == nil then received = received + 1 end
                inbox[r.id] = { text = r.text or "", note = r.note or "", updated = r.updated or 0, kind = r.kind }
            end
        end
    end
    doc.title = ctx.title
    return true, { sent = sent, deleted = deleted, received = received }
end

--- The annotations from Bookx for the open book, newest first.
function M.inbox_list(state, document)
    local list = {}
    for id, r in pairs((state.inbox or {})[document] or {}) do
        list[#list + 1] = { id = id, text = r.text, note = r.note, updated = r.updated }
    end
    table.sort(list, function(a, b)
        if a.updated ~= b.updated then return a.updated > b.updated end
        return a.id < b.id
    end)
    return list
end

local function ago_text(ago)
    if ago < 90 then return "just now"
    elseif ago < 5400 then return string.format("%d min ago", math.floor(ago / 60 + 0.5))
    elseif ago < 172800 then return string.format("%d h ago", math.floor(ago / 3600 + 0.5))
    end
    return string.format("%d days ago", math.floor(ago / 86400))
end

--- One short line for the menu. `configured` (server, user and key saved) defaults to true when omitted.
--- Shows the newest of: the last sync (`state.last`) and the last connection test (`state.test`).
function M.status_line(state, now, configured)
    if configured == false then return "Not configured" end
    local s, t = state.last, state.test
    if s and (not t or (s.time or 0) >= (t.time or 0)) then
        local when = ago_text(math.max(0, now - (s.time or 0)))
        if s.ok then return "Last sync " .. when .. ": " .. tostring(s.message or "ok") end
        return "Last sync failed " .. when .. ": " .. tostring(s.message or "")
    end
    if t then
        local when = ago_text(math.max(0, now - (t.time or 0)))
        if t.ok then return "Last test OK " .. when end
        return "Last test failed " .. when .. ": " .. tostring(t.message or "")
    end
    if configured == true then return "Not tested yet" end
    return "Not synced yet"
end

--- The sentence for a finished sync.
function M.summary_text(r)
    return string.format("Sent %d, removed %d, received %d from Bookx", r.sent, r.deleted, r.received)
end

return M
