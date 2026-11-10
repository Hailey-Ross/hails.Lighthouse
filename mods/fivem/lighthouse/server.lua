-- Lighthouse resource for FiveM and RedM.
--
-- Every 30 seconds it sends Lighthouse a heartbeat: status (players with ids, real uptime, the
-- next txAdmin restart), new console lines, and what happened since the last one (txAdmin bans,
-- warns, kicks, announcements, restarts). Lighthouse answers with commands from its dashboard
-- (restart, message, kick) and, when it changed, the list of Lighthouse bans, which this resource
-- keeps and enforces: banned players are refused when they connect and dropped if online.
--
-- server.cfg:
--   set lighthouse_url "http://203.0.113.10:8651"     # where Lighthouse listens for heartbeats
--   set lighthouse_secret "the ingest secret"          # `set`, never `setr`: players must not see it
--   set lighthouse_server_id "rp-main"                 # the server's id in lighthouse.toml
--   add_ace resource.lighthouse command.quit allow     # lets the dashboard restart/stop the server
--   ensure lighthouse
-- Optional:
--   set lighthouse_send_console "true"                 # forward console output (default true)
--   set lighthouse_send_ips "false"                    # include ip: identifiers (default false)

local VERSION = 1
local INTERVAL_MS = 30000
local MAX_LOG_LINES = 400        -- per heartbeat
local MAX_QUEUED_EVENTS = 500    -- kept while Lighthouse is unreachable

local url = GetConvar('lighthouse_url', ''):gsub('/+$', '')
local secret = GetConvar('lighthouse_secret', '')
local serverId = GetConvar('lighthouse_server_id', '')
local sendConsole = GetConvar('lighthouse_send_console', 'true') ~= 'false'
local sendIps = GetConvar('lighthouse_send_ips', 'false') == 'true'
local game = (GetConvar('gamename', 'gta5') == 'rdr3') and 'redm' or 'fivem'
local underTxAdmin = GetConvar('txAdmin-version', '') ~= ''
-- Stopping needs permission to run `quit`; without it, don't offer stop/restart.
local canQuit = IsPrincipalAceAllowed('resource.' .. GetCurrentResourceName(), 'command.quit')

-- GetGameTimer counts milliseconds since the server process started: real uptime, even when only
-- this resource restarts.
local startedAt = os.time() - math.floor(GetGameTimer() / 1000)
local nextRestartAt = nil
local shuttingDown = false

local events = {}
local logLines = {}
local partialLine = ''
local bans = {}
local bansRev = ''

local function log(msg)
    print(('^5[Lighthouse]^7 %s'):format(msg))
end

if url == '' or secret == '' or serverId == '' then
    log('^1not configured^7: set lighthouse_url, lighthouse_secret and lighthouse_server_id in server.cfg')
    return
end

-- ---------- helpers ----------

local function trim(s)
    return (tostring(s or ''):gsub('^%s+', ''):gsub('%s+$', ''))
end

local function nonEmpty(s)
    s = trim(s)
    if s == '' then return nil end
    return s
end

-- Remove ^0-^9 colour codes from names.
local function plain(s)
    return trim((tostring(s or ''):gsub('%^%d', '')))
end

--- An empty Lua table encodes as `{}`, which Lighthouse doesn't accept as a list: leave it out.
local function list(t)
    if type(t) ~= 'table' or #t == 0 then return nil end
    return t
end

local function identifiers(src)
    local out = {}
    for _, id in ipairs(GetPlayerIdentifiers(src) or {}) do
        if sendIps or not id:find('^ip:') then
            out[#out + 1] = id
        end
    end
    return list(out)
end

local function pushEvent(e)
    e.at_unix = e.at_unix or os.time()
    e.ids = list(e.ids)
    events[#events + 1] = e
    while #events > MAX_QUEUED_EVENTS do
        table.remove(events, 1)
    end
end

-- ---------- console ----------

if sendConsole and RegisterConsoleListener then
    RegisterConsoleListener(function(channel, message)
        local text = partialLine .. tostring(message or '')
        partialLine = ''
        for line, nl in text:gmatch('([^\n]*)(\n?)') do
            if nl == '' then
                partialLine = line
                break
            end
            line = line:gsub('%^%d', ''):gsub('\27%[[%d;]*m', '')
            if line ~= '' then
                logLines[#logLines + 1] = (channel and channel ~= '' and ('[' .. channel .. '] ') or '') .. line
                if #logLines > MAX_LOG_LINES then
                    table.remove(logLines, 1)
                end
            end
        end
    end)
end

-- ---------- Lighthouse bans ----------

local function loadBans()
    local raw = GetResourceKvpString('lighthouse:bans')
    if raw and raw ~= '' then
        local ok, list = pcall(json.decode, raw)
        if ok and type(list) == 'table' then bans = list end
    end
    bansRev = GetResourceKvpString('lighthouse:bans_rev') or ''
end

local function saveBans()
    SetResourceKvp('lighthouse:bans', json.encode(bans))
    SetResourceKvp('lighthouse:bans_rev', bansRev)
end

--- The active ban matching any of these identifiers, if one.
local function findBan(ids)
    local now = os.time()
    for _, ban in ipairs(bans) do
        if not ban.expires_unix or ban.expires_unix > now then
            for _, banned in ipairs(ban.ids or {}) do
                for _, id in ipairs(ids) do
                    if id == banned then return ban end
                end
            end
        end
    end
    return nil
end

local function banMessage(ban)
    local untilText = 'permanently'
    if ban.expires_unix then
        untilText = 'until ' .. os.date('!%Y-%m-%d %H:%M UTC', ban.expires_unix)
    end
    return ('You are banned from this server %s. Reason: %s (ban #%s)'):format(untilText, ban.reason or '-', ban.id or '?')
end

local function enforceBans()
    for _, src in ipairs(GetPlayers()) do
        local ban = findBan(GetPlayerIdentifiers(src) or {})
        if ban then
            pushEvent({ kind = 'kick', player = plain(GetPlayerName(src)), author = 'Lighthouse', reason = 'Lighthouse ban #' .. tostring(ban.id) })
            log(('dropping %s: Lighthouse ban #%s'):format(GetPlayerName(src) or src, ban.id))
            DropPlayer(src, banMessage(ban))
        end
    end
end

AddEventHandler('playerConnecting', function(name, setKickReason, deferrals)
    local src = source
    deferrals.defer()
    Wait(0)
    local ban = findBan(GetPlayerIdentifiers(src) or {})
    if ban then
        log(('refused %s: Lighthouse ban #%s'):format(plain(name), ban.id))
        deferrals.done(banMessage(ban))
    else
        deferrals.done()
    end
end)

-- ---------- txAdmin events ----------
-- https://github.com/tabarra/txAdmin/blob/master/docs/events.md

AddEventHandler('txAdmin:events:playerBanned', function(d)
    pushEvent({
        kind = 'ban', player = nonEmpty(d.targetName), author = nonEmpty(d.author), reason = nonEmpty(d.reason),
        action_id = nonEmpty(d.actionId), expires_unix = (type(d.expiration) == 'number') and d.expiration or nil,
        ids = d.targetIds,
    })
end)

AddEventHandler('txAdmin:events:playerWarned', function(d)
    pushEvent({
        kind = 'warn', player = nonEmpty(d.targetName), author = nonEmpty(d.author), reason = nonEmpty(d.reason),
        action_id = nonEmpty(d.actionId), ids = d.targetIds,
    })
end)

AddEventHandler('txAdmin:events:playerKicked', function(d)
    local target = tonumber(d.target)
    pushEvent({
        kind = 'kick', player = target and plain(GetPlayerName(target)) or nil, author = nonEmpty(d.author),
        reason = nonEmpty(d.reason), ids = target and identifiers(target) or nil,
    })
end)

AddEventHandler('txAdmin:events:actionRevoked', function(d)
    pushEvent({
        kind = 'revoke', player = nonEmpty(d.playerName), author = nonEmpty(d.revokedBy),
        reason = ('%s lifted: %s'):format(tostring(d.actionType or 'action'), tostring(d.actionReason or '')),
        action_id = nonEmpty(d.actionId), ids = d.playerIds,
    })
end)

AddEventHandler('txAdmin:events:announcement', function(d)
    pushEvent({ kind = 'announcement', author = nonEmpty(d.author), reason = nonEmpty(d.message) })
end)

AddEventHandler('txAdmin:events:scheduledRestart', function(d)
    local secs = tonumber(d.secondsRemaining)
    if secs then nextRestartAt = os.time() + secs end
    pushEvent({ kind = 'restart_scheduled', author = 'txAdmin', reason = nonEmpty(d.translatedMessage) })
end)

AddEventHandler('txAdmin:events:skippedNextScheduledRestart', function(d)
    nextRestartAt = nil
    pushEvent({ kind = 'restart_skipped', author = 'txAdmin', reason = d.temporary and 'Skipped once' or 'Skipped' })
end)

-- Defined below; the shutdown handler needs it.
local sendHeartbeat

AddEventHandler('txAdmin:events:serverShuttingDown', function(d)
    pushEvent({ kind = 'shutdown', author = nonEmpty(d.author) or 'txAdmin', reason = nonEmpty(d.message) })
    shuttingDown = true
    sendHeartbeat()
end)

-- ---------- commands from Lighthouse ----------

local function say(message)
    TriggerClientEvent('chat:addMessage', -1, { color = { 120, 200, 210 }, multiline = true, args = { 'Server', message } })
end

--- A player by server id, or by exact (case-insensitive) name.
local function findPlayer(who)
    who = trim(who)
    if tonumber(who) and GetPlayerName(who) then return who end
    for _, src in ipairs(GetPlayers()) do
        if plain(GetPlayerName(src)):lower() == who:lower() then return src end
    end
    return nil
end

local function shutdown(reason)
    if not canQuit then
        log('^1cannot ' .. reason:lower() .. '^7: add `add_ace resource.' .. GetCurrentResourceName() .. ' command.quit allow` to server.cfg')
        return
    end
    say(reason .. ' in 10 seconds.')
    SetTimeout(10000, function()
        for _, src in ipairs(GetPlayers()) do DropPlayer(src, reason .. '.') end
        -- Tell Lighthouse right before quitting, so the server shows offline at once.
        pushEvent({ kind = 'shutdown', author = 'Lighthouse', reason = reason })
        shuttingDown = true
        sendHeartbeat()
        SetTimeout(2000, function()
            ExecuteCommand('quit "' .. reason:gsub('"', '') .. ' (Lighthouse)"')
        end)
        -- Still running long after: the quit didn't happen. Report online again.
        SetTimeout(30000, function()
            shuttingDown = false
            pushEvent({ kind = 'control', author = 'Lighthouse', reason = reason .. ' failed: the server did not quit' })
            log('^1' .. reason:lower() .. ' failed^7: the server is still running')
        end)
    end)
end

local function runCommand(c)
    log(('command from Lighthouse: %s'):format(tostring(c.action)))
    if c.action == 'say' and c.message then
        say(c.message)
    elseif c.action == 'kick' and c.player then
        local src = findPlayer(c.player)
        if src then DropPlayer(src, c.reason or 'Kicked by an admin') else log('kick: no player ' .. c.player) end
    elseif c.action == 'restart' then
        -- txAdmin starts the server again after it quits.
        shutdown('Server restarting')
    elseif c.action == 'stop' then
        shutdown('Server shutting down')
    end
end

-- ---------- heartbeat ----------

local function status()
    local players = {}
    for _, src in ipairs(GetPlayers()) do
        players[#players + 1] = {
            name = plain(GetPlayerName(src)), ping_ms = GetPlayerPing(src), id = tostring(src), ids = identifiers(src),
        }
    end
    if nextRestartAt and nextRestartAt < os.time() then nextRestartAt = nil end
    return {
        state = shuttingDown and 'offline' or 'online',
        name = nonEmpty(plain(GetConvar('sv_projectName', ''))) or nonEmpty(plain(GetConvar('sv_hostname', ''))),
        description = nonEmpty(plain(GetConvar('sv_projectDesc', ''))),
        players_online = #players,
        players_max = GetConvarInt('sv_maxclients', 32),
        players = list(players),
        started_at_unix = startedAt,
        next_restart_unix = nextRestartAt,
        observed_at_unix = os.time(),
    }
end

local inFlight = false

sendHeartbeat = function()
    if inFlight then return end
    inFlight = true
    local sentEvents, sentLines = #events, #logLines
    local accepts = { 'say', 'kick' }
    if canQuit then
        accepts[#accepts + 1] = 'restart'
        -- Under txAdmin a quit is followed by a restart, so "stop" would not stay stopped.
        if not underTxAdmin then accepts[#accepts + 1] = 'stop' end
    end
    local body = json.encode({
        v = VERSION, server_id = serverId, game = game, sent_at_unix = os.time(),
        status = status(), accepts = accepts, log = list({ table.unpack(logLines, 1, sentLines) }),
        events = list({ table.unpack(events, 1, sentEvents) }), bans_rev = bansRev,
    })
    PerformHttpRequest(url .. '/v1/heartbeat', function(code, text)
        inFlight = false
        if code ~= 200 then
            log(('heartbeat failed (%s): %s'):format(tostring(code), tostring(text or 'no answer')))
            return
        end
        -- Delivered: drop what was sent, keep what arrived meanwhile.
        for _ = 1, sentEvents do table.remove(events, 1) end
        for _ = 1, math.min(sentLines, #logLines) do table.remove(logLines, 1) end
        local ok, reply = pcall(json.decode, text or '')
        if not ok or type(reply) ~= 'table' then return end
        if type(reply.bans) == 'table' then
            bans = reply.bans
            bansRev = tostring(reply.bans_rev or '')
            saveBans()
            log(('ban list updated: %d active'):format(#bans))
            enforceBans()
        end
        for _, c in ipairs(reply.commands or {}) do runCommand(c) end
    end, 'POST', body, { ['Content-Type'] = 'application/json', ['Authorization'] = 'Bearer ' .. secret })
end

loadBans()
log(('reporting to %s as "%s" (%s%s)'):format(url, serverId, game, underTxAdmin and ', txAdmin' or ''))
if not canQuit then
    log('restart/stop from the dashboard is off: add `add_ace resource.' .. GetCurrentResourceName() .. ' command.quit allow` to server.cfg')
end

CreateThread(function()
    while true do
        sendHeartbeat()
        Wait(INTERVAL_MS)
    end
end)
