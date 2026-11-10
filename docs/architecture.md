# Architecture

Lighthouse is one program, `lighthouse`, that every community runs for itself. It watches game
servers over the network and keeps Discord status posts up to date. An optional web dashboard shows
history and reports.

```
                    ┌──────────────────────────── lighthouse ─────────────────────────────┐
 game servers ◄─────┤ pollers ──► monitor ──► status posts ─┐                             │
  (Steam query,     │              │    └───► alerts ───────┼──► Discord bots ──────────► Discord
   Minecraft ping,  │              │    └───► status line ──┘   (one per game, or one)   │
   FiveM HTTP)      │              ▼                                                     │
                    │           SQLite ◄──── web dashboard (optional, local by default) ◄── browser
                    │  (history, users, invites, blocks)                                  │
                    └─────────────────────────────────────────────────────────────────────┘
```

## Parts

- **Pollers** (`lighthouse-games`): one task per configured server. Every `poll_interval_secs` the
  game's adapter asks the server for its status: the Steam server query (A2S) for most games, the
  Minecraft server-list ping, or FiveM's `dynamic.json`. Game-specific adapters add details, such as
  the 7 Days to Die world time and blood moon.
- **Monitor** (`lighthouse-discord`): keeps the latest status of every server, decides when a server
  is offline (see below), keeps uptime, and records history.
- **Status posts, status line and alerts** (`lighthouse-discord`): each bot edits its posts in place
  (only when the content changed), re-creates posts that were deleted, rotates its status line through
  its servers, and posts alerts for servers with `alerts = true`. Message ids live in
  `lighthouse.state.json`.
- **Store** (`lighthouse-store`): one SQLite file for status history (30 days), dashboard users,
  sessions, invites and blocked addresses.
- **Dashboard** (`lighthouse-dashboard`): server-rendered pages, each a module (see below).
- **The program** (`lighthouse`): command line, guided setup, bot-mode switching and the updater.

Settings live in `lighthouse.toml`. `lighthouse setup`, `lighthouse bot-mode` and
`lighthouse set-password` write it for you; dashboard users and invites live in the database.

## Offline detection

A server is shown **offline** after two failed checks in a row, so one lost packet doesn't make the
board flicker. Alerts fire on the change from online to offline and back.

## Uptime

Without a game mod, uptime is "online since Lighthouse first saw the server come up". It is kept in
`lighthouse.state.json`, so restarting Lighthouse doesn't reset it. A server going offline resets it.

## History

Each check is a sample, but only some are written: a change of state at once, otherwise at most one
row per minute while the player count moves and one every two minutes when nothing changes. Samples
older than 30 days are removed, and the database hands the space back.

## Dashboard modules

Every page implements the `Module` trait (`crates/lighthouse-dashboard/src/module.rs`): an id (the URL
prefix), a title and icon for the menu, the permission needed to open it, any extra permissions it
defines, and its routes. The router mounts each module under `/<id>` behind its permission and builds
the menu from the pages a user may open. To add a page, write `src/modules/<name>.rs` and add it to
`modules::all()`.

Security: sessions are random tokens in an `HttpOnly`, `SameSite=Strict` cookie; form posts must come
from the dashboard itself; sign-up needs a single-use invite (256-bit code, stored only as a hash);
addresses that guess codes are blocked for 30 minutes, then 3 hours, 7 days, 30 days, then until
lifted.

## Discord limits we design around

- Message edits only when something changed, at most once per check per post.
- Components V2 messages can't carry `content` or `embeds`; all text lives in components, at most
  4000 characters per message.
- Every request honours Discord's `retry-after` on HTTP 429 (handled by the Discord library).

## Adding a game

Most games need no code: add a row to the catalogue in `crates/lighthouse-core/src/catalog.rs` with
its protocol and default ports, then run `python tools/gen_games_md.py
crates/lighthouse-core/src/catalog.rs docs/games.md` to update the list. A game that needs special
handling also gets an adapter in `crates/lighthouse-games/src/` registered in `adapter_for`.

## Planned

- **Game APIs** for richer data: the 7 Days to Die web API (player names), Project Zomboid RCON
  (a reliable player list) and VEIN's HTTP API. The config already accepts them (`native = …`) but they
  aren't used yet.
- **Server-side mods and an agent** that report true uptime and more, sent as
  [heartbeats](heartbeat.md). The `mods/` folder holds the first drafts.
- Adding servers and bots from the dashboard, and Discord slash commands.
