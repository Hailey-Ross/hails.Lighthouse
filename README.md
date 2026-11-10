<p align="center"><img src="assets/brand/lighthouse-banner.png" alt="Lighthouse" width="100%"></p>

# Lighthouse

Discord status boards for your game servers.

Lighthouse runs one Discord bot per game (or one "megabot" for all of them). Each bot keeps a tidy
status post for every server in a channel and updates it in place:

- server name and online/offline state
- players online / max, and who is playing
- uptime since the last restart
- map, version, in-game day and time (and the blood moon countdown in 7 Days to Die)
- join address (optional)

Optional up/down alerts with a role ping, configurable per server.

An optional web dashboard (off by default) shows every server at a glance, uptime history and
outages, population reports (players over time, peaks, player-hours, when people play, CSV export)
and server log files. Accounts are by invitation only: the superadmin, created during setup, invites
sub-admins and decides exactly which pages and servers each one can see. Addresses that guess invite codes are blocked
for 30 minutes, then 3 hours, 7 days, 30 days, and finally until an admin lifts the block.

> **Status:** early development. The status polling, Discord bots and dashboard run; expect rough edges until the first release.

## Supported games

Over 100 games, in two tiers ([full list](docs/games.md)):

- **Fully supported:** 7 Days to Die, Project Zomboid (Build 42) and VEIN. Tested against live
  servers, with game-specific details such as the 7 Days to Die in-game day and blood moon countdown.
- **Experimental:** 100+ games that answer the Steam server query (Valheim, Rust, ARK, Counter-Strike 2,
  Conan Exiles, DayZ, Enshrouded, V Rising and many more), plus Minecraft Java and Bedrock,
  and FiveM/RedM. These should work out of the box; their default ports are not verified yet.

No game mods are needed. Server-side mods for 7 Days to Die and Project Zomboid, which will add uptime
and more detail, are planned.

Test any server from the command line:

```sh
lighthouse query 7dtd 203.0.113.10 26900
lighthouse games valheim
```

## Platforms

Lighthouse is a single program for **Linux** and **Windows**. It can run on the same machine as your
game servers or anywhere else. Each community runs its own copy; there is no hosted service.

## Layout

| Path | What |
|---|---|
| `crates/lighthouse` | The `lighthouse` program |
| `crates/lighthouse-core` | Shared types: status, config, heartbeat format, game adapter trait |
| `crates/lighthouse-games` | Game adapters |
| `crates/lighthouse-discord` | Status card rendering and the Discord bots |
| `mods/7dtd` | Server-side 7 Days to Die mod (C#) |
| `mods/zomboid` | Server-side Project Zomboid mod (Lua, Build 42) |
| `docs/architecture.md` | How Lighthouse fits together |
| `docs/heartbeat.md` | The format mods and agents use to report status |
| `docs/games.md` | Supported games and default ports |
| `crates/lighthouse-dashboard` | Web dashboard; each page is a module in `src/modules/` |
| `crates/lighthouse-store` | SQLite storage: users, invites, sessions, status history |
| `assets/brand/` | Icon, logo, banner and animated icon (`tools/brand/render_brand.py` renders them) |
| `deploy/` | systemd unit (Linux); `Dockerfile` at the root |
| `lighthouse.example.toml` | Example configuration |

## Getting started

```sh
lighthouse setup   # guided first-time setup: servers, bots, dashboard, updates
lighthouse run
```

The setup asks for each server's game (type to search), address, port and display name, checks
that the server answers, then helps you connect the Discord bots. The web dashboard is off unless you
turn it on during setup or later with `lighthouse set-password`.

Lighthouse checks GitHub for new releases once a day. It can install them automatically (`[updates]` in
the config), or you can run `lighthouse update` yourself.

## Building

```sh
cargo build --release
cargo test
```

Requires Rust 1.85 or newer.

## Licence

Created by [Hailey Ross](https://github.com/Hailey-Ross).

Apache License 2.0, see [LICENSE](LICENSE) and [NOTICE](NOTICE). You are free to use, modify and
redistribute Lighthouse; keep the copyright and NOTICE file so the original authors are credited.
