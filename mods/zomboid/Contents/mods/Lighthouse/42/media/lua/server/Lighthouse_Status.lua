-- Lighthouse: writes the server's status to Zomboid/Lua/lighthouse_status.json every 30 seconds
-- for Lighthouse to read (set `status_file` for this server in lighthouse.toml). Format: the
-- "status" object in docs/heartbeat.md. Project Zomboid's Lua can't make network requests, so a
-- file is the hand-off; Lighthouse must run on the same machine (or the file be synced to it).
--
-- Gives Lighthouse what the Steam query can't: real uptime, the in-game day and time, and player
-- names with Steam ids. Stop, restart, messages, kicks and bans go through RCON instead.

if not isServer() then return end

local STATUS_FILE = "lighthouse_status.json"
local INTERVAL_SECONDS = 30

local startedAt = getTimestamp() -- unix seconds; the Lua state starts with the server
local lastWrite = 0

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
        local entry = '{"name":' .. jsonString(p:getUsername())
        local ok, steamId = pcall(function() return p:getSteamID() end)
        if ok and steamId and tostring(steamId) ~= "0" then
            entry = entry .. ',"ids":[' .. jsonString("steam:" .. tostring(steamId)) .. "]"
        end
        parts[#parts + 1] = entry .. "}"
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

local function option(name)
    local ok, value = pcall(function() return getServerOptions():getOption(name) end)
    if ok and value and value ~= "" then return value end
    return nil
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
    local name = option("PublicName")
    if name then json = json .. ',"name":' .. jsonString(name) end
    local desc = option("PublicDescription")
    if desc then json = json .. ',"description":' .. jsonString(desc) end
    json = json .. "}"

    local writer = getFileWriter(STATUS_FILE, true, false)
    if writer then
        writer:write(json)
        writer:close()
    end
    lastWrite = now
end

Events.OnServerStarted.Add(function() writeStatus("online") end)

-- Real-time throttle: game-time events run at the speed of the in-game clock.
Events.OnTick.Add(function()
    if getTimestamp() - lastWrite >= INTERVAL_SECONDS then
        writeStatus("online")
    end
end)
