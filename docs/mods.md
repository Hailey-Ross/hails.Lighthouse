# Game APIs, mods and server controls

Lighthouse works without any of this: it reads every server over the network the way the in-game
server browser does. The extras below add real uptime, player names, a live console, moderation and
server controls (start, stop, restart, messages, kicks, bans) on the dashboard.

| Game | Without extras | With the game's own API | With the Lighthouse mod |
|---|---|---|---|
| 7 Days to Die | Name, players, day and time, blood moon | **Web API**: real uptime, player names and pings, exact blood moon day, live console, all controls | Same, plus works when the web API port can't be reached |
| Project Zomboid | Name, players, version | **RCON**: reliable player list, all controls except start | Real uptime and game time (same machine only) |
| VEIN | Name, players, version | **HTTP API**: real uptime, player names | (VEIN has no mods) |
| FiveM / RedM | Name, description, game type, players | — | **Lighthouse resource**: real uptime, player ids, txAdmin events, scheduled restarts, live console, bans, controls |

Every control action is recorded with who did it (Moderation › Activity). Dashboard users need the
matching permissions (Users page): *Start, stop and restart servers*, *Send messages*, *Kick
players*, *Ban players and lift bans*.

## 7 Days to Die

### Web API (recommended)

No mod needed, works on rented servers whose web dashboard port is reachable.

1. In `serverconfig.xml`: `WebDashboardEnabled` = `true` (port: `WebDashboardPort`, default 8080).
2. In the server console (telnet or your host's web console), make a token. The secret may only
   contain letters, digits and `_`:
   ```
   webtokens add lighthouse <secret> 0
   ```
3. In `lighthouse.toml`, on the server:
   ```toml
   native = { kind = "seven_days_web_api", port = 8080, token_name = "lighthouse", token_secret = "<secret>" }
   ```
   (`lighthouse setup` asks for this and makes a secret for you.)

Unlisted servers (`ServerVisibility` 0) don't answer the network status check at all; with the web
API set up they still show correctly.

### Telnet (controls only)

If the web API isn't available: `TelnetEnabled` `true`, a `TelnetPassword`, then
`telnet = { port = 8081, password = "..." }` on the server.

### The Lighthouse mod

A server-side mod (`mods/7dtd`) that sends Lighthouse a heartbeat every 30 seconds and carries out
dashboard commands. Use it when Lighthouse can't reach the server's web API (it only needs to reach
Lighthouse). Needs `[ingest]` in `lighthouse.toml`.

1. Build it (needs the .NET SDK and a 7 Days to Die dedicated server install for its DLLs):
   ```
   cd mods/7dtd
   dotnet build -c Release -p:SevenDaysServerDir="C:\path\to\7DaysToDieServer"
   ```
2. Copy `mods/7dtd/Lighthouse` to the server's `Mods` folder.
3. Copy `Config.example.xml` to `Config.xml` there and fill in Lighthouse's address, the ingest secret
   and the server's id.

## Project Zomboid

### RCON

Set `RCONPort` and `RCONPassword` in the server's `.ini`, then on the server in `lighthouse.toml`:

```toml
native = { kind = "rcon", port = 27015, password = "..." }
```

This works from anywhere that can reach the RCON port. Project Zomboid's mods can't add RCON commands
(the game's command list is fixed in its Java code) or make network requests, so real uptime and game
time need the status file below.

### The Lighthouse mod (same machine)

`mods/zomboid` writes `Zomboid/Lua/lighthouse_status.json` every 30 seconds. Install it like any
local mod (`Zomboid/mods/Lighthouse`, add `Lighthouse` to `Mods=` in the server's `.ini`), then point
the server's `status_file` in `lighthouse.toml` at that file. Project Zomboid pauses with nobody
online, so the file stops updating then; Lighthouse keeps using its start time.

Status (the network check) answers on the game port (`DefaultPort`, default 16261).

## VEIN

VEIN has no mods. Its HTTP API gives player names and real uptime:

1. In `Game.ini` (`Vein/Saved/Config/WindowsServer/` or `LinuxServer/`):
   ```ini
   [/Script/Vein.VeinGameSession]
   HTTPPort=8085
   ```
2. The API only listens on the server itself. To reach it from another machine, add to `Engine.ini`
   in the same folder:
   ```ini
   [HTTPServer.Listeners]
   DefaultBindAddress=0.0.0.0
   ```
   **The API has no password.** Open its port only to the machine Lighthouse runs on.
3. On the server in `lighthouse.toml`: `native = { kind = "vein_http", port = 8085 }`.

If Lighthouse runs on the same machine, `process = "VeinServer"` also gives the real uptime (when
only one VEIN server runs there).

## FiveM and RedM (with txAdmin)

The Lighthouse resource (`mods/fivem/lighthouse`) works on both and alongside txAdmin. Needs
`[ingest]` in `lighthouse.toml`.

1. Copy `mods/fivem/lighthouse` to the server's `resources` folder.
2. In `server.cfg`:
   ```
   set lighthouse_url "http://<lighthouse address>:8651"
   set lighthouse_secret "<ingest secret>"
   set lighthouse_server_id "<the server's id in lighthouse.toml>"
   add_ace resource.lighthouse command.quit allow
   ensure lighthouse
   ```
   Use `set`, never `setr`, for the secret. The `add_ace` line lets the dashboard restart the server
   (txAdmin starts it again after it quits); without it, restart and stop aren't offered.

What it reports: players with their identifiers, real uptime, the next scheduled txAdmin restart
(shown on the Discord card), console output, and every txAdmin ban, warning, kick, revoke,
announcement, scheduled or skipped restart and shutdown (Moderation page).

**Bans from the dashboard:** txAdmin doesn't let other resources create bans, so Lighthouse keeps
its own ban list and the resource enforces it: banned players are refused when they connect and
dropped if online. The resource keeps a copy, so bans hold while Lighthouse is down. A ban can cover
one server or all FiveM/RedM servers; ban online players from their row, or anyone by identifier
(`license:…`, `discord:…`, `steam:…`, `fivem:…`).

## Commands on the same machine

For servers Lighthouse can manage directly (a service, a script, a container), give it commands for
the Start, Stop and Restart buttons; they run through the system shell:

```toml
control = { start = "systemctl start vein", stop = "systemctl stop vein", restart = "systemctl restart vein" }
```

These take priority over the mod and the game console. `lighthouse control <server> <action>` runs
the same actions from the command line (except those that go through a mod).

## Live logs

Logs › a server:

- **Log files** (server on this machine): set `log_dir` to its install or data folder. *Follow*
  shows new lines as they're written.
- **Live console**: 7 Days to Die with the web API (read from the server), or any server whose mod
  sends console lines (the last 1000 are kept).
