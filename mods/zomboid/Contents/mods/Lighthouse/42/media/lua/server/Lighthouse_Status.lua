-- Lighthouse: writes the server status to Zomboid/Lua/lighthouse_status.json
-- for the Lighthouse agent to pick up. Format: docs/heartbeat.md ("status" object).
-- Not yet tested on a live Build 42 server.

if not isServer() then return end

local STATUS_FILE = "lighthouse_status.json"
local startedAt = getTimestamp() -- unix seconds

local function jsonString(value)
    local s = tostring(value or "")
    s = s:gsub("\\", "\\\\")
    s = s:gsub('"', '\\"')
    s = s:gsub("%c", " ")
    return '"' .. s .. '"'
end

local function playersJson(players)
    local parts = {}
    for i = 0, players:size() - 1 do
        local p = players:get(i)
        parts[#parts + 1] = '{"name":' .. jsonString(p:getUsername()) .. '}'
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

local function writeStatus(state)
    local now = getTimestamp()
    local players = getOnlinePlayers()
    local gameTime = getGameTime()
    local day = math.floor(gameTime:getWorldAgeHours() / 24) + 1

    local json = "{"
        .. '"state":' .. jsonString(state)
        .. ',"players_online":' .. tostring(players:size())
        .. ',"players_max":' .. tostring(getServerOptions():getInteger("MaxPlayers"))
        .. ',"players":' .. playersJson(players)
        .. ',"started_at_unix":' .. tostring(startedAt)
        .. ',"game_time":{"day":' .. tostring(day)
        .. ',"hour":' .. tostring(gameTime:getHour())
        .. ',"minute":' .. tostring(gameTime:getMinutes()) .. "}"
        .. ',"observed_at_unix":' .. tostring(now)
        .. "}"

    local writer = getFileWriter(STATUS_FILE, true, false)
    if writer then
        writer:write(json)
        writer:close()
    end
end

Events.OnServerStarted.Add(function() writeStatus("online") end)
Events.EveryTenMinutes.Add(function() writeStatus("online") end)
