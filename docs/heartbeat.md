# Heartbeat format (v1)

Game mods and the Lighthouse agent report server status with a **heartbeat**: a small JSON document
sent every 15–60 seconds. Lighthouse marks a server offline when heartbeats stop and the Steam query
also fails.

## Sending

```
POST http://<lighthouse-host>:8651/v1/heartbeat
Authorization: Bearer <ingest secret from lighthouse.toml>
Content-Type: application/json
```

Use HTTPS (a reverse proxy in front of Lighthouse) whenever the heartbeat crosses the internet.

## Body

```json
{
  "v": 1,
  "server_id": "7dtd-pve",
  "game": "seven_days_to_die",
  "sent_at_unix": 1767225600,
  "status": {
    "state": "online",
    "name": "PvE Main",
    "players_online": 2,
    "players_max": 8,
    "players": [{ "name": "Alice", "ping_ms": 40 }, { "name": "Bob" }],
    "started_at_unix": 1767214800,
    "map": "Navezgane",
    "version": "V 2.4 (b7)",
    "game_time": { "day": 14, "hour": 21, "minute": 5 },
    "blood_moon_in_days": 0,
    "password_protected": false,
    "observed_at_unix": 1767225600
  }
}
```

| Field | Required | Notes |
|---|---|---|
| `v` | yes | Format version, currently `1`. |
| `server_id` | yes | The `id` of the server in `lighthouse.toml`. |
| `game` | yes | `seven_days_to_die` (or `7dtd`), `project_zomboid` (or `pz`), `vein`. |
| `sent_at_unix` | yes | Unix time in seconds. |
| `status.state` | yes | `online`, `starting`, `offline`, `unknown`. Send `offline` on clean shutdown. |
| `status.observed_at_unix` | yes | Unix time in seconds. |
| `status.started_at_unix` | no | When the server process started. Uptime is measured from here. |
| everything else | no | Leave out what the game does not know. Lighthouse fills gaps from the Steam query. |

## File hand-off (Project Zomboid)

Project Zomboid's Lua mods cannot make network requests. The Lighthouse Zomboid mod writes only the
`status` object to `Zomboid/Lua/lighthouse_status.json`. The Lighthouse agent on the same machine
reads that file, wraps it in a heartbeat and sends it.
