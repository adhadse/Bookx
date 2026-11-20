-- MD5 for the Bookx Sync plugin (own code, MIT licence): a small, dependency-free implementation.
-- KOReader's `ffi/sha2` is used by KOReader itself, but a plugin must not depend on an FFI module
-- being present: a missing one used to make saving a password raise an error and panic KOReader.
-- This module uses only arithmetic, so it runs on Lua 5.1 to 5.4 and LuaJIT, with or without FFI.
-- It is used only to hash the progress-sync password once; it is not used for security beyond that.
local M = {}

local FLOOR, MOD = math.floor, math.fmod

local MASK = 4294967295
local POW2 = {}
for i = 0, 64 do POW2[i] = 2 ^ i end

local function u(x) return x % 4294967296 end

-- Bit operations on unsigned 32-bit numbers, without bit32/bit/FFI. Password hashes are short,
-- so the byte-by-byte loops are cheap enough and behave the same on every Lua version.
local function binop(a, b, f)
    local r, m = 0, 1
    for _ = 1, 32 do
        if f(MOD(a, 2) >= 1, MOD(b, 2) >= 1) then r = r + m end
        a, b, m = FLOOR(a / 2), FLOOR(b / 2), m * 2
    end
    return r
end

local function band(a, b) return binop(a, b, function(x, y) return x and y end) end
local function bor(a, b) return binop(a, b, function(x, y) return x or y end) end
local function bxor(a, b) return binop(a, b, function(x, y) return x ~= y end) end
local function bnot(a) return MASK - a end

local function rol(x, n)
    local high = FLOOR(x / POW2[32 - n])
    local low = (x - high * POW2[32 - n]) * POW2[n]
    return u(high + low)
end

local K = {
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
    0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
    0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
    0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
}
local S = {
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
}

local function hex32(n)
    local out = {}
    for i = 1, 4 do
        local b = MOD(n, 256)
        out[i] = string.format("%02x", b)
        n = FLOOR(n / 256)
    end
    return table.concat(out)
end

--- The 32 hex digits of the MD5 of `s` (bytes, not characters: UTF-8 is hashed as given).
function M.md5(s)
    s = tostring(s or "")
    local bytes = { s:byte(1, #s) }
    local bitlen = (#s % 4294967296) * 8 -- low 32 bits of the length, as MD5 specifies
    bytes[#bytes + 1] = 0x80
    while #bytes % 64 ~= 56 do bytes[#bytes + 1] = 0 end
    for i = 0, 7 do bytes[#bytes + 1] = MOD(FLOOR(bitlen / POW2[8 * i]), 256) end

    local a0, b0, c0, d0 = 0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476
    for off = 1, #bytes, 64 do
        local w = {}
        for i = 0, 15 do
            local j = off + i * 4
            w[i] = bytes[j] + bytes[j + 1] * 256 + bytes[j + 2] * 65536 + bytes[j + 3] * 16777216
        end
        local a, b, c, d = a0, b0, c0, d0
        for i = 0, 63 do
            local f, g
            if i < 16 then
                f, g = bor(band(b, c), band(bnot(b), d)), i
            elseif i < 32 then
                f, g = bor(band(d, b), band(bnot(d), c)), (5 * i + 1) % 16
            elseif i < 48 then
                f, g = bxor(bxor(b, c), d), (3 * i + 5) % 16
            else
                f, g = bxor(c, bor(b, bnot(d))), (7 * i) % 16
            end
            local tmp = d
            d, c, b = c, b, u(b + rol(u(a + f + K[i + 1] + w[g]), S[i + 1]))
            a = tmp
        end
        a0, b0, c0, d0 = u(a0 + a), u(b0 + b), u(c0 + c), u(d0 + d)
    end
    return hex32(a0) .. hex32(b0) .. hex32(c0) .. hex32(d0)
end

return M
