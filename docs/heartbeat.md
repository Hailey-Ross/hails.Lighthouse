# Heartbeat format (v1)

Game mods report to Lighthouse with a **heartbeat**: a small JSON document sent every 30 seconds.
It carries what the network check can't see (real uptime, player names and ids, the in-game clock,
console output, bans and kicks), and Lighthouse's reply carries commands back from the dashboard.

The ready-made senders:

| Game | Sender | Where |
|---|---|---|
| FiveM, RedM | Lighthouse resource (works with txAdmin) | `mods/fivem/lighthouse` |
| 7 Days to Die | Lighthouse server mod | `mods/7dtd` |
| Project Zomboid | Status file, read by Lighthouse on the same machine | `mods/zomboid` |

Turn heartbeats on with an `[ingest]` section in `lighthouse.toml` (see
[lighthouse.example.toml](../lighthouse.example.toml)).

## Sending

```
POST http://<lighthouse-host>:8651/v1/heartbeat
Authorization: Bearer <ingest secret from lighthouse.toml>
Content-Type: application/json
```

Use HTTPS (a reverse proxy in front of Lighthouse) whenever the heartbeat crosses the internet: the
secret travels with every heartbeat. Wrong secrets get the sender's address blocked, with the same
escalating blocks as guessed dashboard invites.

`GET /v1/health` answers `ok`, for checking that the port is reachable.

## Body

```json
{
  "v": 1,
  "server_id": "rp-main",
  "game": "fivem",
  "sent_at_unix": 1767225600,
  "status": {
    "state": "online",
    "name": "Red City RP",
    "players_online": 2,
    "players_max": 48,
    "players": [
      { "name": "Alice", "ping_ms": 40, "id": "12", "ids": ["license:0a1b…", "discord:1234"] },
      { "name": "Bob" }
    ],
    "started_at_unix": 1767214800,
    "next_restart_unix": 1767229200,
    "observed_at_unix": 1767225600
  },
  "accepts": ["say", "kick", "restart"],
  "log": ["[script:chat] Alice: hi"],
  "events": [
    { "kind": "ban", "player": "Mallory", "author": "AdminJoe", "reason": "Cheating",
      "action_id": "ABCD-1234", "expires_unix": 1767830400, "ids": ["license:9f8e…"] }
  ],
  "bans_rev": ""
}
```

| Field | Required | Notes |
|---|---|---|
| `v` | yes | Format version, currently `1`. |
| `server_id` | yes | The `id` of the server in `lighthouse.toml`. |
| `game` | yes | Must match the server's `game` in `lighthouse.toml` (`fivem`, `redm`, `7dtd`, `pz`, …). |
| `sent_at_unix` | yes | Unix time in seconds. |
| `status.state` | yes | `online`, `starting` or `offline`. Send `offline` when shutting down: the server shows offline at once. |
| `status.observed_at_unix` | yes | Unix time in seconds. |
| `status.started_at_unix` | no | When the server process started. Uptime is measured from here. |
| `status.players[].id` | no | The game's id for the player this session (FiveM server id, 7DTD entity id); used for kicks. |
| `status.players[].ids` | no | Lasting identifiers; used for bans. |
| `status.next_restart_unix` | no | Next scheduled restart, shown on the status card. |
| other `status` fields | no | Anything in Lighthouse's status (`map`, `version`, `game_time`, `description`, …). Leave out what the game doesn't know; the network check fills gaps. |
| `accepts` | no | Commands this mod can carry out: `stop`, `restart`, `say`, `kick`, `ban`. The dashboard offers only these. |
| `log` | no | New console lines since the last heartbeat. Shown live on the dashboard's Logs page (the last 1000 are kept). |
| `events` | no | What happened since the last heartbeat; see below. Resend them if a heartbeat fails. |
| `bans_rev` | no | Only for mods that enforce Lighthouse bans: the revision of the ban list they hold (`""` at first). |

A server counts as online while heartbeats arrive, even if its query port is blocked. Heartbeats
count for three check intervals (at least 90 seconds).

### Events

`kind` is one of `ban`, `warn`, `kick`, `revoke` (a ban or warn lifted), `announcement`,
`restart_scheduled`, `restart_skipped`, `shutdown`, `start`. Optional fields: `at_unix`, `player`,
`author`, `reason`, `action_id` (the tool's own id, so a later `revoke` can be matched),
`expires_unix` (bans; none = permanent), `ids`. Bans, warns, kicks and lifts are kept a year,
everything else 30 days. They're listed on the dashboard's Moderation page.

## Reply

```json
{
  "v": 1,
  "commands": [
    { "id": 7, "action": "say", "message": "Restart in 5 minutes" },
    { "id": 8, "action": "kick", "player": "12", "reason": "AFK" },
    { "id": 9, "action": "ban", "player": "Bob", "reason": "Griefing", "duration_secs": 86400 },
    { "id": 10, "action": "restart" }
  ],
  "bans": [
    { "id": 3, "player": "Mallory", "ids": ["license:9f8e…"], "reason": "Cheating",
      "author": "admin", "created_unix": 1767225000, "expires_unix": 1767830400 }
  ],
  "bans_rev": "1:3"
}
```

- `commands`: carry them out in order. Each is sent once; commands not collected within two minutes
  are dropped. `player` is the player's `id` if the status had one, else their name.
  `duration_secs` missing on a ban means permanent. `ban` commands only go to games that keep their
  own ban list (7 Days to Die); FiveM and RedM use `bans` instead.
- `bans` (only when the sent `bans_rev` was out of date): the complete list of active Lighthouse bans
  for this server. Replace the held list, refuse matching players when they connect, drop matching
  players who are online, and send `bans_rev` back from now on.

## File hand-off (Project Zomboid)

Project Zomboid's Lua can't make network requests. The Lighthouse Zomboid mod writes only the
`status` object to `Zomboid/Lua/lighthouse_status.json` every 30 seconds. Set the server's
`status_file` in `lighthouse.toml` to that path; Lighthouse reads it while it's fresh (under two
minutes old). Controls for Project Zomboid go through RCON.
