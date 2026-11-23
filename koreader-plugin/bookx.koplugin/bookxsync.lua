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
M.MAX_BODY_BYTES = 600000 -- the server refuses a body over 1 MiB

--- The version of this plugin (shown in About / Diagnostics) and the wire protocol it speaks. The server
--- announces its own in `GET <server>/info`; see `check_server`.
M.VERSION = "0.3.1"
M.PROTOCOL = 1

-- C1 control characters (U+0080..U+009F) are two bytes in UTF-8: \194\128..\194\159
local function strip_c1(s, repl) return (s:gsub("\194[\128-\159]", repl)) end

local function truncate_chars(s, max)
    local n = 0
    for pos in s:gmatch("()[^\128-\191]") do
        n = n + 1
        if n > max then return s:sub(1, pos - 1) end
    end
    return s
end

--- Text for a free text field (quote, note): valid UTF-8, no control characters but new lines and tabs, at most `max` characters.
function M.clean_text(s, max)
    s = json.clean_utf8(tostring(s or ""))
    s = s:gsub("[%z\1-\8\11-\31\127]", "")
    return truncate_chars(strip_c1(s, ""), max or 20000)
end

--- Text for a one-line field (chapter, position, name): control characters become spaces.
function M.clean_field(s, max)
    s = json.clean_utf8(tostring(s or ""))
    s = s:gsub("%c", " ")
    return truncate_chars(strip_c1(s, " "), max or 128)
end

local function trim(s)
    return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

--- The name this device sends: the one the user typed in the plugin, else KOReader's `Device.model`
--- (a codename on Android, `Kobo_nova` on a Kobo, `KindlePaperWhite5` on a Kindle...), else "KOReader".
--- Never rewritten here: Bookx turns known model codes into the name on the box (and shows an
--- unknown one as reported, so the user can rename it there).
function M.device_label(custom, model)
    local c = trim(M.clean_field(custom, 128))
    if c ~= "" then return c end
    local m = trim(M.clean_field(model, 128))
    if m ~= "" then return m end
    return "KOReader"
end

-- names go to the server without control characters; KOReader keeps several authors in one string
-- separated by new lines ("Noah Gift\nAlfredo Deza"), which the server refuses
function M.clean_name(s)
    s = json.clean_utf8(tostring(s or "")):gsub("[\r\n]+", "; ")
    return M.clean_field(s, 512)
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

-- The look is sent as KOReader wrote it. A word this version does not know (a newer KOReader adds colours
-- and drawers) is passed on verbatim, so Bookx keeps it instead of a default; only control characters are
-- dropped and the length is limited to what the server accepts (32), so one odd value cannot refuse a request.
local function word(v)
    if type(v) ~= "string" then return "" end
    return (json.clean_utf8(v):gsub("%c", ""):sub(1, 32))
end

--- One KOReader annotation (an entry of `ui.annotation.annotations`) as a wire annotation.
function M.to_wire(item, now)
    local pos0 = pos_text(item.pos0)
    local pos1 = pos_text(item.pos1)
    local page = item.pageno
    if type(page) ~= "number" and type(item.page) == "number" then page = item.page end
    local created = M.parse_datetime(item.datetime) or 0
    local kind = (pos0 ~= "" and pos1 ~= "" and trim(M.clean_text(item.text)) ~= "") and "highlight" or "bookmark"
    local a = {
        id = "ko-" .. M.hash(tostring(item.datetime or "") .. "|" .. pos0 .. "|" .. pos1 .. "|" .. pos_text(item.page)),
        kind = kind,
        text = M.clean_text(item.text),
        note = M.clean_text(item.note),
        color = word(item.color),
        drawer = word(item.drawer),
        chapter = M.clean_field(item.chapter, 1000),
        pos0 = M.clean_field(pos0, 1000),
        pos1 = M.clean_field(pos1, 1000),
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
        local a = type(item) == "table" and M.to_wire(item, now) or nil
        if a and not seen[a.id] then
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

-- header values are plain ASCII, at most 128 characters (the full device name also goes in the request body)
local function ascii(s, max)
    return (tostring(s or ""):gsub("[^\32-\126]", "?"):sub(1, max or 128))
end

--- Tells the server who is asking: the device (name and stable id), KOReader and plugin version, protocol.
--- Adds the headers to `h` (a table) and returns it. A server that does not know them ignores them.
function M.client_headers(ctx, h)
    h = h or {}
    if (ctx.device or "") ~= "" then h["X-Bookx-Device"] = ascii(ctx.device) end
    if (ctx.device_id or "") ~= "" then h["X-Bookx-Device-Id"] = ascii(ctx.device_id) end
    h["X-Bookx-Client"] = "KOReader/" .. (ascii(ctx.koreader or "unknown", 40):gsub("%s", "_"))
        .. " bookx-plugin/" .. M.VERSION
    h["X-Bookx-Protocol"] = tostring(M.PROTOCOL)
    return h
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

--- "192.168.1.5:8080" from a server address.
local function host_of(server)
    return trim(server):match("^https?://([^/]+)") or trim(server)
end

--- One short reason for a socket error ("connection refused", "timed out", "unknown host", ...).
function M.network_reason(err)
    local e = tostring(err or ""):lower()
    if e:find("refused", 1, true) then return "connection refused" end
    if e:find("timeout", 1, true) or e:find("timed out", 1, true) then return "timed out" end
    if e:find("not known", 1, true) or e:find("resolve", 1, true) or e:find("host not found", 1, true) then
        return "unknown host"
    end
    if e:find("unreachable", 1, true) then return "network unreachable" end
    if e:find("closed", 1, true) or e:find("reset", 1, true) then return "connection closed" end
    if e:find("certificate", 1, true) or e:find("ssl", 1, true) or e:find("tls", 1, true) then return "TLS problem" end
    if e == "" or e == "nil" then return "no answer" end
    return (e:gsub("%c", " "):sub(1, 60))
end

--- The message for a request that got no HTTP answer at all.
function M.unreachable(server, err)
    return "cannot reach " .. host_of(server) .. " (" .. M.network_reason(err) .. ")"
end

--- A short, human message for an HTTP status that is not a success. `decoded` is the JSON body, if any.
function M.status_message(status, decoded)
    if status == 401 then return "wrong user name or password" end
    if status == 404 then return "the Bookx server has no KOReader sync here (switch it on in Sync, and check the address ends in /bookx)" end
    if status == 429 then return "too many requests, try again in a minute" end
    if status == 413 then return "the server refused the request as too large (413)" end
    if type(decoded) == "table" and type(decoded.message) == "string" then
        return "the server refused it (" .. tostring(status) .. "): " .. decoded.message:sub(1, 120)
    end
    if status >= 500 then return "the server failed (" .. tostring(status) .. "), see the Bookx log" end
    return "server answered " .. tostring(status)
end

local function capital(s) return (s:gsub("^%l", string.upper)) end

--- Asks `<server>/info` what the server speaks. Returns "ok", "unknown" (an older Bookx without /info),
--- "server-old" (it cannot take this plugin) or "plugin-old" (the plugin is older than the server needs),
--- and a message for the last three.
function M.check_server(ctx)
    local status, text = ctx.request("GET", M.info_url(ctx.server), M.client_headers(ctx, { ["Accept"] = "application/json" }), "")
    if status == nil then return "unknown", nil end
    if status ~= 200 then return "unknown", nil end
    local info = type(text) == "string" and json.decode(text) or nil
    if type(info) ~= "table" or type(info.protocol) ~= "number" then return "unknown", nil end
    if info.annotations == false then
        return "server-old", "The Bookx server has the KOReader annotation sync switched off (Sync in Bookx)."
    end
    if type(info.min_plugin_protocol) == "number" and info.min_plugin_protocol > M.PROTOCOL then
        return "plugin-old", "Bookx needs a newer Bookx Sync plugin (server protocol " .. tostring(info.protocol)
            .. ", plugin " .. tostring(M.PROTOCOL) .. "): update the plugin from Bookx Preferences > Sync."
    end
    if info.protocol < M.PROTOCOL then
        return "server-old", "This Bookx is too old for the plugin (protocol " .. tostring(info.protocol)
            .. ", plugin " .. tostring(M.PROTOCOL) .. "): update Bookx."
    end
    return "ok", nil
end

--- The URL of the server's version information.
function M.info_url(server) return M.base_url(server) .. "/info" end

--- Checks the saved login. ctx = { server, user, key, request }. Returns ok, message
--- (neither the key nor the password is ever part of a message).
function M.test_connection(ctx)
    local status, err = ctx.request("GET", M.auth_url(ctx.server), M.client_headers(ctx, {
        ["Accept"] = "application/json", ["x-auth-user"] = ctx.user, ["x-auth-key"] = ctx.key }), "")
    if status == nil then
        if M.network_reason(err) == "timed out" then
            return false, "The server did not answer in time (timeout): " .. host_of(ctx.server)
        end
        return false, capital(M.unreachable(ctx.server, err))
    end
    if status == 401 then return false, "Wrong user name or password" end
    if status == 404 then return false, "Bookx sync is not enabled at that address (404)" end
    if status == 429 then return false, "Too many requests, try again in a minute" end
    if status ~= 200 then return false, capital(M.status_message(status)) end
    local verdict, msg = M.check_server(ctx)
    if verdict == "server-old" or verdict == "plugin-old" then return false, msg end
    return true, "Connected as " .. tostring(ctx.user)
end

--- Creates an account. ctx = { server, user, key (md5 of the password), request }. Returns ok, message.
function M.register(ctx)
    local body = json.encode({ username = ctx.user, password = ctx.key })
    local status, err = ctx.request("POST", M.create_url(ctx.server), M.client_headers(ctx, {
        ["Accept"] = "application/json", ["Content-Type"] = "application/json" }), body)
    if status == 201 or status == 200 then return true, "Account " .. tostring(ctx.user) .. " created" end
    if status == nil then
        if M.network_reason(err) == "timed out" then
            return false, "The server did not answer in time (timeout): " .. host_of(ctx.server)
        end
        return false, capital(M.unreachable(ctx.server, err))
    end
    if status == 402 then return false, "That name is taken: choose another name, or use the password of your existing account" end
    if status == 403 then return false, "Registration is closed on that server" end
    if status == 404 then return false, "Bookx sync is not enabled at that address (404)" end
    if status == 429 then return false, "Too many requests, try again in a minute" end
    return false, capital(M.status_message(status))
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
    local headers = M.client_headers(ctx, {
        ["Accept"] = "application/json",
        ["Content-Type"] = "application/json",
        ["x-auth-user"] = ctx.user,
        ["x-auth-key"] = ctx.key,
    })
    local answer
    local highlights, bookmarks = 0, 0
    local i = 1
    -- split by count and by bytes (the server refuses a body over 1 MiB)
    local encoded = {}
    for k, a in ipairs(send) do encoded[k] = #json.encode(a) + 1 end
    repeat
        local chunk, bytes = {}, 0
        while i <= #send and #chunk < M.MAX_ITEMS_PER_REQUEST and (#chunk == 0 or bytes + encoded[i] <= M.MAX_BODY_BYTES) do
            chunk[#chunk + 1] = send[i]
            bytes = bytes + encoded[i]
            i = i + 1
        end
        local body = json.encode({
            origin = "koreader", device = M.clean_field(ctx.device, 128), device_id = M.clean_field(ctx.device_id, 128),
            title = M.clean_name(ctx.title), authors = M.clean_name(ctx.authors), annotations = chunk,
        })
        local url = M.build_url(ctx.server, ctx.document, doc.seq)
        local status, text = ctx.request("PUT", url, headers, body)
        if status == nil then return false, M.unreachable(ctx.server, text) end
        if status ~= 200 then
            local decoded = type(text) == "string" and text ~= "" and json.decode(text) or nil
            return false, M.status_message(status, decoded)
        end
        answer = type(text) == "string" and json.decode(text) or nil
        if type(answer) ~= "table" or type(answer.annotations) ~= "table" then
            return false, "the server's answer was not understood (is the address the Bookx one, ending in /bookx?)"
        end
        for _, a in ipairs(chunk) do
            if a.deleted then deleted = deleted + 1
            else
                sent = sent + 1
                if a.kind == "bookmark" then bookmarks = bookmarks + 1 else highlights = highlights + 1 end
            end
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
    -- what the server says about the book: Bookx may not have it (then nothing can be imported there)
    state.unmatched = state.unmatched or {}
    if answer.in_library == false then
        state.unmatched[ctx.document] = { title = M.clean_name(ctx.title), time = ctx.now }
    elseif answer.in_library == true then
        state.unmatched[ctx.document] = nil
    end
    local stats = state.stats or { highlights = 0, bookmarks = 0, received = 0 }
    state.stats = stats
    stats.highlights = (stats.highlights or 0) + highlights
    stats.bookmarks = (stats.bookmarks or 0) + bookmarks
    stats.received = (stats.received or 0) + received
    state.waiting = nil
    return true, { sent = sent, deleted = deleted, received = received, highlights = highlights,
        bookmarks = bookmarks, in_library = answer.in_library }
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
    local w = state.waiting
    if w and (not s or (w.time or 0) >= (s.time or 0)) and (not t or (w.time or 0) > (t.time or 0)) then
        return "Waiting for Wi-Fi since " .. ago_text(math.max(0, now - (w.time or 0))) .. ": will sync when a book is open with Wi-Fi on"
    end
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

local function plural(n, one, many) return string.format("%d %s", n, n == 1 and one or many) end

--- The sentence for a finished sync: "Sent 3 highlights, received 1 note from Bookx".
function M.summary_text(r)
    local parts
    if (r.bookmarks or 0) > 0 then
        parts = { "Sent " .. (r.sent or 0) .. ": " .. plural(r.highlights or 0, "highlight", "highlights")
            .. ", " .. plural(r.bookmarks, "bookmark", "bookmarks") }
    else
        parts = { "Sent " .. plural(r.highlights or r.sent or 0, "highlight", "highlights") }
    end
    if (r.deleted or 0) > 0 then parts[#parts + 1] = "removed " .. r.deleted end
    parts[#parts + 1] = "received " .. plural(r.received or 0, "note", "notes") .. " from Bookx"
    local text = table.concat(parts, ", ")
    if r.in_library == false then
        text = text .. "\nBookx has no matching book in your library, so these highlights are stored but not imported yet. Add this book to your Bookx library."
    end
    return text
end

--- A short single-line version of an error text for the screen.
function M.short_error(err, max)
    local line = tostring(err or ""):match("^[^\r\n]*") or ""
    max = max or 140
    if #line > max then line = line:sub(1, max - 3) .. "..." end
    return line
end

--- The text of About / Diagnostics. `d` = { koreader, device, server, user, has_key, auto, configured, state, now }.
--- The key and the password are never part of it.
function M.diagnostics(d)
    local st = d.state or {}
    local function when(t) return os.date("%Y-%m-%d %H:%M:%S", t or 0) end
    local lines = {
        "Bookx Sync " .. M.VERSION .. " (protocol " .. M.PROTOCOL .. ")",
        "KOReader: " .. tostring(d.koreader or "?") .. ", device: " .. tostring(d.device or "?"),
        "Server: " .. ((d.server or "") ~= "" and d.server or "(not set)") .. "  user: "
            .. ((d.user or "") ~= "" and d.user or "(not set)") .. "  password: " .. (d.has_key and "saved (hash)" or "not saved"),
        "Sync automatically: " .. tostring(d.auto == true),
        "Status: " .. M.status_line(st, d.now or os.time(), d.configured),
    }
    if st.last then
        lines[#lines + 1] = "Last sync: " .. (st.last.ok and "ok" or "failed") .. " (" .. when(st.last.time) .. "): " .. tostring(st.last.message)
    end
    if st.test then
        lines[#lines + 1] = "Last test: " .. (st.test.ok and "ok" or "failed") .. " (" .. when(st.test.time) .. "): " .. tostring(st.test.message)
    end
    local stats = st.stats
    if stats then
        lines[#lines + 1] = "Sent so far: " .. plural(stats.highlights or 0, "highlight", "highlights") .. ", "
            .. plural(stats.bookmarks or 0, "bookmark", "bookmarks") .. "; received " .. plural(stats.received or 0, "note", "notes")
    else
        lines[#lines + 1] = "Sent so far: nothing"
    end
    if st.waiting then
        lines[#lines + 1] = "Waiting for Wi-Fi since " .. when(st.waiting.time) .. " (" .. tostring(st.waiting.why or "") .. ")"
    end
    local missing = {}
    for doc, u in pairs(st.unmatched or {}) do
        missing[#missing + 1] = (u.title ~= "" and u.title or "(no title)") .. " [" .. doc:sub(1, 8) .. "]"
    end
    table.sort(missing)
    if #missing > 0 then
        lines[#lines + 1] = "Books Bookx could not match in its library (" .. #missing .. "): " .. table.concat(missing, "; ")
    else
        lines[#lines + 1] = "Books Bookx could not match: none reported"
    end
    lines[#lines + 1] = "Automatic sync runs: 30 s after a highlight or note change, when a book closes, when the device suspends, "
        .. "and when a book opens with Wi-Fi on (it never switches Wi-Fi on by itself)."
    local e = st.last_error
    if e then
        lines[#lines + 1] = "Last error (" .. when(e.time) .. ") in " .. tostring(e.where) .. ": " .. tostring(e.message)
        if e.traceback and e.traceback ~= "" then lines[#lines + 1] = tostring(e.traceback) end
    else
        lines[#lines + 1] = "Last error: none"
    end
    lines[#lines + 1] = "Full details: KOReader's crash.log, lines with \"Bookx Sync\"."
    return table.concat(lines, "\n")
end

return M
