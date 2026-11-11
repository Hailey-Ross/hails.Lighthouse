# Game extras

Lighthouse works without any of this. These extras let it see more of each game: the **real uptime**
(not just "since Lighthouse noticed"), **player names**, a **live console**, and **buttons on the
dashboard** to restart a server, send a message, kick or ban.

Pick your game:

- [7 Days to Die](#7-days-to-die): switch on its built-in web dashboard (two settings). Recommended.
- [Project Zomboid](#project-zomboid): switch on RCON (two settings).
- [VEIN](#vein): switch on its HTTP API (one setting).
- [FiveM and RedM](#fivem-and-redm-works-with-txadmin): copy the Lighthouse resource onto the server.

All of these are done in two places: your **game host's control panel** (or the server's files),
and the **Setup** page of Lighthouse's dashboard. Each section says exactly what to do in each.

Who may press the buttons is up to you: on the dashboard's **Users** page, tick *Start, stop and
restart servers*, *Send messages*, *Kick players* or *Ban players* for each admin. Everything anyone
does is listed under **Moderation › Activity**, with their name.

---

## 7 Days to Die

7 Days to Die has a web dashboard built in. Lighthouse can use it to read player names, the real
uptime, the exact blood moon day and the server console, and to restart, message, kick and ban.
Nothing to install.

**In your game host's control panel:**

1. Open the server's settings (the file is called `serverconfig.xml`; many hosts show it as a list of
   settings).
2. Set **WebDashboardEnabled** to `true`. Note **WebDashboardPort** (usually `8080`).
3. Restart the game server.
4. Open the server **console** (many hosts call it *Web Console* or *Console*) and type this, with a
   password of your own instead of `MySecret_123` (use only letters, digits and `_`):
   ```
   webtokens add lighthouse MySecret_123 0
   ```

**On Lighthouse's dashboard:** **Setup** → click the server → under **Game API** choose
**7 Days to Die web API** → enter the port from step 2, the token name `lighthouse` and your
password → click **Test**, then **Save**.

If the test says the API didn't answer, the host may block that port from outside: ask them to open
it, or check their panel's port settings.

<details><summary>Advanced: telnet, and the Lighthouse mod</summary>

- **Telnet** (buttons only, if the web dashboard isn't available): set `TelnetEnabled` to `true` and a
  `TelnetPassword`, then enter the telnet port and password on the Setup page.
- **The Lighthouse mod** (`mods/7dtd`): for servers whose web dashboard Lighthouse can't reach. The mod
  reports to Lighthouse instead (Lighthouse's **Game mods** switch on the Setup page must be on, and
  the game server must be able to reach Lighthouse). It has to be built from source with the .NET SDK
  against your server's own game files:
  `dotnet build -c Release -p:SevenDaysServerDir="C:\path\to\7DaysToDieServer"`, then copy
  `mods/7dtd/Lighthouse` into the server's `Mods` folder and fill in `Config.xml` (copy it from
  `Config.example.xml`).

Unlisted servers (`ServerVisibility` 0) don't answer the usual status check; with the web API set up
Lighthouse still shows them correctly.

</details>

---

## Project Zomboid

Project Zomboid's **RCON** gives Lighthouse a reliable player list and the buttons (restart, stop,
message, kick, ban).

**In your game host's control panel**, in the server's settings (the file ending in `.ini`, e.g.
`servertest.ini`):

1. Set **RCONPort** (often `27015`) and **RCONPassword** to a password of your own.
2. Restart the game server.

**On Lighthouse's dashboard:** **Setup** → click the server → **Game API**: **Project Zomboid RCON**
→ enter the port and password → **Test** → **Save**.

Bans from the dashboard use Project Zomboid's own ban list, and are permanent.

<details><summary>Advanced: the Lighthouse mod (same computer only)</summary>

Project Zomboid's mods can't talk to the network, so the real uptime and in-game time only work when
Lighthouse runs on the same computer as the game server. Download `lighthouse-zomboid-mod.zip` from
the [releases page](https://github.com/Hailey-Ross/hails.Lighthouse/releases/latest), unzip it into
`Zomboid/mods/`, add `Lighthouse` to `Mods=` in the server's `.ini`, and on the Setup page put the
path of `Zomboid/Lua/lighthouse_status.json` in **Mod status file**.

The status check uses the game port (`DefaultPort`, usually 16261).

</details>

---

## VEIN

VEIN's HTTP API gives Lighthouse the real uptime and player names. (VEIN has no mods.)

**In the server's files** (folder `Vein/Saved/Config/WindowsServer/`, or `LinuxServer/`):

1. In `Game.ini`, under the line `[/Script/Vein.VeinGameSession]`, add:
   ```ini
   HTTPPort=8085
   ```
2. If Lighthouse runs on **another** computer, also add to `Engine.ini` in the same folder:
   ```ini
   [HTTPServer.Listeners]
   DefaultBindAddress=0.0.0.0
   ```
   **This API has no password.** Ask your host (or set your firewall) to allow that port only from
   the computer Lighthouse runs on.
3. Restart the game server.

**On Lighthouse's dashboard:** **Setup** → click the server → **Game API**: **VEIN HTTP API** → port
`8085` → **Test** → **Save**.

---

## FiveM and RedM (works with txAdmin)

The **Lighthouse resource** runs on your FiveM or RedM server. It gives Lighthouse the real uptime,
the players, the console, and every txAdmin ban, warning, kick and scheduled restart (shown on the
**Moderation** page and the Discord post). It also lets you **restart**, **message**, **kick** and
**ban** from the dashboard.

**1. Switch on Lighthouse's game mod connection.** On the dashboard: **Setup** → **Game mods** →
**Turn on**. The page then shows the lines for step 3.

**2. Put the resource on the server.** Download `lighthouse-fivem-redm.zip` from the
[releases page](https://github.com/Hailey-Ross/hails.Lighthouse/releases/latest) and unzip it. Upload
the `lighthouse` folder into the server's `resources` folder (with your host's file manager, or FTP).

**3. Add the lines to `server.cfg`.** Copy them from the Setup page; they look like this:

```
set lighthouse_url "http://<Lighthouse's address>:8651"
set lighthouse_secret "<the secret shown on the Setup page>"
set lighthouse_server_id "<the server's id on the Setup page>"
add_ace resource.lighthouse command.quit allow
ensure lighthouse
```

- `<Lighthouse's address>` is the public address of the computer Lighthouse runs on.
- The `add_ace` line allows restarts from the dashboard (txAdmin starts the server again afterwards).
  Without it, the restart and stop buttons aren't offered.
- Keep the secret private, and use `set` (never `setr`) for it.

**4. Restart the FiveM/RedM server.** Within a minute the Setup page's test and the dashboard show it.

**The one technical bit:** the game server must be able to reach Lighthouse on port **8651**. If
Lighthouse runs on a home internet connection, forward port 8651 to that computer in your router
(search the web for your router's name and "port forwarding"). A small rented server (VPS) usually
needs nothing.

**Bans from the dashboard:** txAdmin doesn't let other resources create bans, so Lighthouse keeps its
own ban list and the resource enforces it: banned players can't join and are removed if they're
online. The resource keeps a copy, so bans still work while Lighthouse is offline. A ban can cover one
server or all your FiveM/RedM servers. Ban someone from the **Moderation** page: from their row while
they're online, or anytime by their identifier (`license:…`, `discord:…`; txAdmin's player page shows
them).

---

## For servers on the same computer as Lighthouse

- **Live logs:** on the Setup page, put the game server's folder in **Log folder**. The **Logs** page
  then shows its log files and follows them live.
- **Start, stop and restart commands:** if you manage the server with a service or script, put the
  commands in **Start / Stop / Restart command** on the Setup page. They're used before anything else.
- **Real uptime without extras:** put the server program's name (e.g. `VeinServer`) in
  **Program name**. Works when only one copy of that program runs on the computer.

The format the mods use to report, for developers: [heartbeat.md](heartbeat.md).
