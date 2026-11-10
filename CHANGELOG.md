# Changelog

Stable releases have a version number, raised only for notable changes. Smaller stable builds keep
the number and get a new build number. Each version needs its own section here before it can be
released.

## Unreleased

- Game APIs: player names and pings from the 7 Days to Die web API (plus its in-game clock), a reliable
  player list from Project Zomboid RCON, and player names from VEIN's HTTP API. The setup asks about them.
- FiveM and RedM: project name and description, game type and server version.
- Minecraft: the second line of the message of the day; Bedrock game mode.
- Rust: time since the last wipe. DayZ: in-game time, and DayZ itself is now in the game list.
- Server controls on the dashboard: start, stop, restart and messages, through commands on this
  machine, a game mod, the 7 Days to Die web API or telnet, or Project Zomboid RCON. Also
  `lighthouse control` on the command line.
- Moderation page: who's online, kicks, bans (Lighthouse's own ban list for FiveM/RedM, the game's
  for 7 Days to Die and Project Zomboid), and a history of bans, warnings, kicks and restarts,
  including txAdmin's.
- Live logs: follow log files as they grow, and a live console from the 7 Days to Die web API or a mod.
- Real uptime: from the 7 Days to Die web API, VEIN's HTTP API, the server's process on this
  machine, or a mod.
- Game mods: the Lighthouse resource for FiveM and RedM (txAdmin events, scheduled restarts, bans),
  a server-side 7 Days to Die mod, and an updated Project Zomboid mod. They report to a new
  heartbeat listener (`[ingest]`).
- Unlisted 7 Days to Die servers (which don't answer the status check) show correctly through the
  web API.
- Project Zomboid's default status port is now the game port (16261), as on Build 42.
- `/status` slash command: the board, or one server's card picked from an autocomplete list, shown only to
  the person who asked.

## 0.1.0

- First release.
- Discord status posts for game servers, edited in place, with one bot per game or one bot for all games, a rotating status line, and optional up/down alerts.
- Full support for 7 Days to Die, Project Zomboid (Build 42) and VEIN without any game mods; experimental support for 100+ more games through the Steam server query, Minecraft Java and Bedrock, and FiveM/RedM.
- Guided setup in the terminal, including creating the Discord bot, or a quick setup that finishes in the web dashboard.
- Web dashboard: servers at a glance, uptime and outages, population reports, server logs, users with per-page and per-server access, invite-only sign-up, and blocks for addresses that guess invite codes.
- Automatic updates from this project's releases, on the stable or bleeding-edge channel.
