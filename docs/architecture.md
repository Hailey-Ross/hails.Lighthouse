# Architecture

Lighthouse is one program, `lighthouse`, that every community runs for itself. It watches game
servers and keeps Discord status boards up to date.

```
                      ┌──────────────────────── lighthouse ────────────────────────┐
 game server ◄─poll── │ pollers ──► status store ──► board updater ──► Discord bots │ ──► Discord
   (Steam query,      │   ▲            (SQLite)   ──► alerter       ──► (one per game│
    native API)       │   │                       ──► presence          or one mega)│
                      │ ingest ◄── heartbeats                                       │
 mod / agent ──push──►│ (HTTP)                    dashboard (local web UI)          │
                      └─────────────────────────────────────────────────────────────┘
```

## Parts

- **Pollers** — one task per configured server. Every `poll_interval_secs` the game's adapter queries
  the server: the Steam query (A2S) first, then the game's own API if one is configured.
- **Ingest** — an HTTP endpoint that accepts heartbeats from game mods and agents
  ([format](heartbeat.md)). Heartbeats carry what a poll cannot see, such as uptime and in-game time.
- **Status store** — merges the sources for each server (heartbeat > native API > Steam query), keeps
  the latest status in memory and a history in SQLite.
- **Board updater** — renders each server's card and edits its message in the status channel. It only
  edits when the card changed, re-creates messages that were deleted, and keeps message ids in SQLite.
- **Alerter** — posts state changes (online → offline, back online, restarted) for servers with
  `alerts = true`.
- **Presence** — each bot's Discord activity shows a summary, e.g. "4/5 servers up · 37 players".
- **Dashboard** — a small local web page for editing servers and settings, bound to localhost by
  default.

Configuration can come from three places, all writing the same settings: `lighthouse.toml`, Discord
slash commands (`/lighthouse …`, server managers only) and the dashboard.

## Offline detection

A server is **offline** when its poll fails twice in a row **and** no heartbeat arrived within three
heartbeat intervals. One missed poll is not enough, which keeps boards from flickering. A clean
shutdown heartbeat (`state = "offline"`) marks it offline at once.

## Uptime

Uptime is the time since the server process last started. Only mods and agents know it, through
`started_at_unix`. Without a mod Lighthouse shows the time since it first saw the server come online,
which equals uptime as long as Lighthouse itself was running at the time.

## Roles and deployment

The same program can run in two roles, alone or together:

| Role | Does | Where |
|---|---|---|
| `bot` | pollers, store, Discord, dashboard, ingest | anywhere with internet access |
| `agent` | reads local game data (status file, localhost API) and sends heartbeats | next to a game server |

On a single machine both roles run in one process and pass heartbeats internally. With game servers on
several machines, run `bot` once and an `agent` beside each game server that needs one.

| Game | Needs an agent? |
|---|---|
| 7 Days to Die | No. The mod sends heartbeats itself. |
| Project Zomboid | Yes, to read the file the Lua mod writes. |
| VEIN | Only to use the built-in HTTP API, which must not be exposed to the internet. |

## Discord limits we design around

- Message edits: only on change, at most once per poll interval per message.
- Channel renames (optional counter channels): Discord allows 2 per 10 minutes per channel.
- Components V2 messages cannot carry `content` or `embeds`; all text lives in components, at most
  4000 characters per message.
- Every request honours Discord's `retry-after` on HTTP 429.

## Adding a game

1. Add a variant to `GameKind` (`crates/lighthouse-core/src/game.rs`).
2. Implement `GameAdapter` in `crates/lighthouse-games/src/<game>.rs` and register it in `adapter_for`.
3. If the game has its own API, add a `NativeApi` variant.
4. Optionally write a server mod that sends [heartbeats](heartbeat.md).
