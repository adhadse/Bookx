-- A small JSON encoder/decoder for Bookx Sync (own code, MIT licence).
-- Works in Lua 5.1 to 5.5 and LuaJIT; no dependencies.
local M = {}

M.null = setmetatable({}, { __tostring = function() return "null" end })

local escapes = {
    ['"'] = '\\"', ["\\"] = "\\\\", ["\b"] = "\\b", ["\f"] = "\\f",
    ["\n"] = "\\n", ["\r"] = "\\r", ["\t"] = "\\t",
}

local function escape_char(c)
    return escapes[c] or string.format("\\u%04x", c:byte())
end

local function is_array(t)
    if getmetatable(t) == M.object_mt then return false end
    local n = 0
    for _ in pairs(t) do n = n + 1 end
    if n == 0 then return true end
    for i = 1, n do
        if t[i] == nil then return false end
    end
    return true
end

M.object_mt = {}

--- Marks an (empty) table to be written as `{}`.
function M.object(t)
    return setmetatable(t or {}, M.object_mt)
end

local function encode(v, out, depth)
    if depth > 64 then error("json: nested too deeply") end
    local t = type(v)
    if v == nil or v == M.null then
        out[#out + 1] = "null"
    elseif t == "boolean" then
        out[#out + 1] = v and "true" or "false"
    elseif t == "number" then
        if v ~= v or v == math.huge or v == -math.huge then error("json: number is not finite") end
        if v == math.floor(v) and math.abs(v) < 2 ^ 53 then
            out[#out + 1] = string.format("%d", v)
        else
            out[#out + 1] = string.format("%.14g", v)
        end
    elseif t == "string" then
        out[#out + 1] = '"' .. v:gsub('[%c"\\]', escape_char) .. '"'
    elseif t == "table" then
        if is_array(v) then
            out[#out + 1] = "["
            for i = 1, #v do
                if i > 1 then out[#out + 1] = "," end
                encode(v[i], out, depth + 1)
            end
            out[#out + 1] = "]"
        else
            out[#out + 1] = "{"
            local keys = {}
            for k in pairs(v) do
                if type(k) ~= "string" then error("json: object keys must be strings") end
                keys[#keys + 1] = k
            end
            table.sort(keys)
            for i, k in ipairs(keys) do
                if i > 1 then out[#out + 1] = "," end
                out[#out + 1] = '"' .. k:gsub('[%c"\\]', escape_char) .. '":'
                encode(v[k], out, depth + 1)
            end
            out[#out + 1] = "}"
        end
    else
        error("json: cannot encode a " .. t)
    end
end

function M.encode(v)
    local out = {}
    encode(v, out, 0)
    return table.concat(out)
end

local function utf8_char(cp)
    if cp < 0x80 then
        return string.char(cp)
    elseif cp < 0x800 then
        return string.char(0xC0 + math.floor(cp / 64), 0x80 + cp % 64)
    elseif cp < 0x10000 then
        return string.char(0xE0 + math.floor(cp / 4096), 0x80 + math.floor(cp / 64) % 64, 0x80 + cp % 64)
    else
        return string.char(0xF0 + math.floor(cp / 262144), 0x80 + math.floor(cp / 4096) % 64,
            0x80 + math.floor(cp / 64) % 64, 0x80 + cp % 64)
    end
end

local function skip(s, i)
    return s:find("[^ \t\r\n]", i) or #s + 1
end

local decode_value

local function decode_string(s, i)
    local out = {}
    i = i + 1
    while true do
        local j = s:find('["\\%c]', i)
        if not j then error("json: unterminated string") end
        out[#out + 1] = s:sub(i, j - 1)
        local c = s:sub(j, j)
        if c == '"' then
            return table.concat(out), j + 1
        elseif c == "\\" then
            local e = s:sub(j + 1, j + 1)
            local map = { b = "\b", f = "\f", n = "\n", r = "\r", t = "\t", ['"'] = '"', ["\\"] = "\\", ["/"] = "/" }
            if map[e] then
                out[#out + 1] = map[e]
                i = j + 2
            elseif e == "u" then
                local hex = s:match("^%x%x%x%x", j + 2)
                if not hex then error("json: bad \\u escape") end
                local cp = tonumber(hex, 16)
                i = j + 6
                if cp >= 0xD800 and cp < 0xDC00 then
                    local lo = s:match("^\\u(%x%x%x%x)", i)
                    local l = lo and tonumber(lo, 16)
                    if l and l >= 0xDC00 and l < 0xE000 then
                        cp = 0x10000 + (cp - 0xD800) * 1024 + (l - 0xDC00)
                        i = i + 6
                    else
                        cp = 0xFFFD
                    end
                elseif cp >= 0xDC00 and cp < 0xE000 then
                    cp = 0xFFFD
                end
                out[#out + 1] = utf8_char(cp)
            else
                error("json: bad escape")
            end
        else
            error("json: control character in string")
        end
    end
end

function decode_value(s, i, depth)
    if depth > 64 then error("json: nested too deeply") end
    i = skip(s, i)
    local c = s:sub(i, i)
    if c == "{" then
        local obj = {}
        i = skip(s, i + 1)
        if s:sub(i, i) == "}" then return obj, i + 1 end
        while true do
            i = skip(s, i)
            if s:sub(i, i) ~= '"' then error("json: object key expected") end
            local k
            k, i = decode_string(s, i)
            i = skip(s, i)
            if s:sub(i, i) ~= ":" then error("json: ':' expected") end
            local v
            v, i = decode_value(s, i + 1, depth + 1)
            obj[k] = v
            i = skip(s, i)
            local d = s:sub(i, i)
            if d == "}" then return obj, i + 1 end
            if d ~= "," then error("json: ',' or '}' expected") end
            i = i + 1
        end
    elseif c == "[" then
        local arr = {}
        i = skip(s, i + 1)
        if s:sub(i, i) == "]" then return arr, i + 1 end
        while true do
            local v
            v, i = decode_value(s, i, depth + 1)
            arr[#arr + 1] = v == nil and M.null or v
            i = skip(s, i)
            local d = s:sub(i, i)
            if d == "]" then return arr, i + 1 end
            if d ~= "," then error("json: ',' or ']' expected") end
            i = i + 1
        end
    elseif c == '"' then
        return decode_string(s, i)
    elseif s:sub(i, i + 3) == "true" then
        return true, i + 4
    elseif s:sub(i, i + 4) == "false" then
        return false, i + 5
    elseif s:sub(i, i + 3) == "null" then
        return M.null, i + 4
    else
        local num = s:match("^-?%d+%.?%d*[eE]?[+-]?%d*", i)
        if not num or num == "" or not tonumber(num) then error("json: unexpected character") end
        return tonumber(num), i + #num
    end
end

--- Returns the value, or nil and a message when the text is not JSON.
function M.decode(s)
    local ok, v, i = pcall(decode_value, s, 1, 0)
    if not ok then return nil, tostring(v) end
    if skip(s, i) <= #s then return nil, "json: extra text after the value" end
    return v
end

return M
