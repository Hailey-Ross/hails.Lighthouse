<p align="center"><img src="assets/brand/lighthouse-banner.png" alt="Lighthouse" width="100%"></p>

# Lighthouse

**Calm, clear game server status for Discord.**

Lighthouse keeps a tidy status post in your Discord channel for each of your game servers: is it
online, who's playing, how long since the last restart, and for 7 Days to Die the in-game day and
the next blood moon. The post updates itself; nobody has to do anything. People can also type
`/status` in Discord to check the servers.

It comes with a web page for you and your admins (the **dashboard**): uptime history, player
reports, server logs, and buttons to restart servers or ban players.

> **Status:** early. It works and is tested on live servers; expect rough edges until version 1.0.

**New here? Follow [Getting started](#getting-started) from top to bottom.** No programming needed.

## Contents

- [Getting started](#getting-started)
  1. [Make a Discord bot](#1-make-a-discord-bot)
  2. [Install Lighthouse](#2-install-lighthouse)
  3. [Answer the setup questions](#3-answer-the-setup-questions)
  4. [Open the dashboard](#4-open-the-dashboard)
- [Extras for your games](#extras-for-your-games)
- [Updates](#updates)
- [Something isn't working](#something-isnt-working)
- [Supported games](#supported-games)
- [Reference](#reference) (commands, settings file, running it yourself)
- [For developers](#for-developers)

## Getting started

### What you need

- **A computer that stays on**, running **Windows 10 or 11** or **Linux**. It does **not** have to be
  the computer your game servers run on: your own PC is fine as long as it's on, or a small rented
  server (a "VPS"). If your game servers are rented from a host, run Lighthouse on any other computer;
  nothing gets installed on the game servers.
- **A Discord server** where you're an admin (you can add bots and channels).
- For each game server: **its address and port**. Your game host's control panel shows them, usually
  as something like `203.0.113.10:26900` (the part before the `:` is the address, the number after it
  is the port).

### 1. Make a Discord bot

Lighthouse posts through a Discord bot that belongs to you. Making one takes about three minutes:

1. Open the [Discord Developer Portal](https://discord.com/developers/applications) and sign in with
   your normal Discord account.
2. Click **New Application** (top right). Give it a name people will see, like *Server Status*, tick
   the box to agree, and click **Create**.
3. On the left, click **Bot**. Under the bot's name, click **Reset Token**, then **Yes, do it!**, then
   **Copy**. That long text is the bot's **token**: Lighthouse needs it in a moment.
   - Keep the token secret: anyone who has it can control your bot. If it ever leaks, click
     **Reset Token** again and give Lighthouse the new one.
   - You don't need to change anything else on that page.
4. Keep this page open; you'll come back for the invite link (the setup gives it to you).

### 2. Install Lighthouse

#### On Windows

1. Download the installer:
   [**install-windows.ps1**](https://github.com/Hailey-Ross/hails.Lighthouse/releases/download/stable/install-windows.ps1)
   (it's small; your browser may ask whether to keep it: choose **Keep**).
2. Open your **Downloads** folder, **right-click** `install-windows.ps1` and choose
   **Run with PowerShell**.
   - Windows asks *"Do you want to allow this app to make changes?"*: click **Yes**. (Lighthouse needs
     this once, to start by itself with Windows.)
   - If a blue or black window asks about an *"Execution Policy"*, type `Y` and press **Enter**.
   - If nothing happens: right-click the file, choose **Properties**, tick **Unblock** at the bottom,
     click **OK**, and try again.
3. A window opens and asks you a few questions. Go on to [step 3](#3-answer-the-setup-questions).

Lighthouse goes into `C:\Lighthouse` and runs quietly in the background from then on, also after
the computer restarts. There's no window to keep open.

#### On Linux

Open a terminal on that machine (or connect to it over SSH) and run these two lines:

```sh
curl -fsSLO https://github.com/Hailey-Ross/hails.Lighthouse/releases/download/stable/install-linux.sh
sudo sh install-linux.sh
```

It asks a few questions (next step), then runs Lighthouse as a background service that starts by
itself at boot.

### 3. Answer the setup questions

The installer asks you, one at a time. Type your answer and press **Enter**; where you get a list,
use the **arrow keys** and press **Enter**.

- **Quick or full setup?** The *quick setup* only asks for a dashboard password and you do the rest
  in your browser (step 4). The *full setup* asks everything here. Either is fine; pick quick if
  you're unsure.
- **Your game servers** (full setup): start typing the game's name and pick it from the list, then
  enter the **address**, the **port** and a **name** to show in Discord. Lighthouse checks the server
  answers before it moves on. If it doesn't, see
  [A server shows offline](#a-server-shows-offline-but-people-are-playing).
- **Your Discord bot** (full setup): paste the **token** from step 1 (right-click to paste in the
  window). Lighthouse gives you an **invite link**: open it in your browser, pick your Discord server
  and click **Authorize**. Then choose the **channel** for the status post.
- **The dashboard**: answer **yes**, then choose a **username and password**. You'll use them to sign
  in to the dashboard.
- **Updates**: say yes. Lighthouse then keeps itself up to date.

When it says **All done**, Lighthouse is running. Within a minute the status post appears in your
channel.

### 4. Open the dashboard

On the computer running Lighthouse, open <http://127.0.0.1:8650> in your browser (the Windows
installer opens it for you) and sign in with the username and password from step 3.

- **Did the quick setup?** Go to **Setup** (in the menu on the left):
  - **Add a server** for each game server: pick the game, enter the address and port, click **Test**,
    then **Save**.
  - **Add a bot**: paste the token from step 1 and click **Continue**. Open the **invite link** it
    shows, add the bot to your Discord server, come back, click **Refresh**, pick the channel and
    click **Save the bot**.
- **Servers** shows everything at a glance. **Uptime** and **Reports** show history and when people
  play.
- **Users**: invite other admins with a link and tick exactly what each one may see and do.

The dashboard only opens on the computer running Lighthouse. On a Linux server, the installer shows
a one-line command that brings it to your own computer.

## Extras for your games

Out of the box Lighthouse shows who's online and how long each server has been up. Each game can
give it more, such as **real uptime**, **player names**, a **live console**, and **buttons to restart,
message, kick and ban**:

| Game | What to set up | Gives you |
|---|---|---|
| 7 Days to Die | Its built-in web dashboard (two settings) | Real uptime, player names, live console, all buttons |
| Project Zomboid | RCON (two settings) | Reliable player list, all buttons |
| VEIN | Its HTTP API (one setting) | Real uptime, player names |
| FiveM / RedM | The Lighthouse resource (a folder you copy) | Real uptime, txAdmin bans and restarts, bans from the dashboard, buttons |

Step-by-step for each: **[Game extras guide](docs/mods.md)**. The FiveM/RedM resource and the
Project Zomboid mod are ready to download on the
[releases page](https://github.com/Hailey-Ross/hails.Lighthouse/releases/latest).

## Updates

If you said yes to updates during setup, there's nothing to do: Lighthouse checks once a day, installs
new versions by itself and restarts in a few seconds. Your settings and history stay.

**Updating an older install by hand** (for example one where updates were turned off): run the
installer from [step 2](#2-install-lighthouse) again. It sees Lighthouse is already there, backs up
your settings and data, swaps in the new version, checks everything came through unchanged, and
starts it again. Nothing else changes.

Prefer to try new features early? Lighthouse has two channels: **stable** (the default, tested) and
**bleeding-edge** (every change as soon as it's made; may break). Switch with `channel` under
`[updates]` in the settings file.

## Something isn't working

#### A server shows offline but people are playing

Lighthouse is asking the wrong port, or a firewall is in the way. Some games answer status questions
on a different port from the one players join:

| Game | Port Lighthouse asks |
|---|---|
| 7 Days to Die | the game port (usually 26900) |
| Project Zomboid | the game port (usually 16261) |
| VEIN | the game port **+ 1** (e.g. 7778 for a server on 7777) |
| Others | see the [game list](docs/games.md); often the game port, game port + 1, or 27015 |

Your game host's control panel often lists a **query port**: if it does, that's the one. On the
dashboard, open **Setup**, click the server, put it in **Status port**, click **Test**, then **Save**.

#### The bot doesn't post anything

- Check the bot is in your Discord server and may post in that channel: in Discord, right-click the
  channel → **Edit Channel** → **Permissions** → add the bot and allow **View Channel**,
  **Send Messages** and **Read Message History**.
- Look at the log: on Windows `C:\Lighthouse\lighthouse.log` (open it with Notepad); on Linux
  `journalctl -u lighthouse -n 50`. It says in plain words what's wrong.
- "Discord rejected the token": reset the token in the Developer Portal (step 1) and add the bot again
  on the dashboard's **Setup** page.

#### I can't open the dashboard

- Open it on the computer running Lighthouse: <http://127.0.0.1:8650>.
- "Access blocked": that address tried wrong invite links too often. On the computer running
  Lighthouse, open a terminal in its folder and run `lighthouse unblock <the address>`.
- Forgot the password? In the same place run `lighthouse set-password` and choose a new one.

#### Something else

Open an [issue](https://github.com/Hailey-Ross/hails.Lighthouse/issues) and include the last lines
of the log.

## Supported games

- **Fully supported** (tested on live servers): **7 Days to Die**, **Project Zomboid** (Build 42),
  **VEIN**, **FiveM** and **RedM**.
- **Experimental** (should work, not every default checked): 100+ more games, including Valheim, Rust,
  ARK, Counter-Strike 2, Conan Exiles, DayZ, Enshrouded, V Rising and **Minecraft** (Java and Bedrock).

Full list with their usual ports: [docs/games.md](docs/games.md).

What each game shows besides name, players and uptime:

| Game | Extra details |
|---|---|
| 7 Days to Die | In-game day and time, blood moon countdown; with its web dashboard on, player names and the exact blood moon day |
| Project Zomboid | Player names |
| VEIN | With its HTTP API: player names |
| FiveM / RedM | Server name and description, game type, version, player names; with the resource, the next scheduled restart |
| Minecraft | Message of the day, version, some player names |
| Rust | Time since the last wipe |
| DayZ | In-game time |

## Reference

### Where Lighthouse keeps things

Everything is in one folder (`C:\Lighthouse` on Windows, `/var/lib/lighthouse` on Linux):

| File | What it is |
|---|---|
| `lighthouse.toml` | Your settings. The dashboard's **Setup** page edits it for you; every setting is explained in [lighthouse.example.toml](lighthouse.example.toml). |
| `lighthouse.db` | History, dashboard users and bans. |
| `lighthouse.state.json` | Which Discord posts Lighthouse keeps up to date. |
| `lighthouse.log` | What it's doing (Windows; on Linux: `journalctl -u lighthouse`). |

Back up the folder to keep everything.

### Commands

For people comfortable with a terminal. Run them in Lighthouse's folder (on Windows `.\lighthouse`).

| Command | What it does |
|---|---|
| `lighthouse setup` | The setup questions (first time, or to start over). |
| `lighthouse run` | Start Lighthouse in this window (stop with Ctrl+C). The installers run it in the background for you. |
| `lighthouse check-config` | Check the settings file for mistakes. |
| `lighthouse query <game> <address> [port]` | Ask one server for its status right now; handy for finding the right port. |
| `lighthouse games [name]` | List supported games and their usual ports. |
| `lighthouse control <server> <action>` | start, stop, restart, say, kick or ban, from the command line. |
| `lighthouse bot-mode per-game` / `mega` | One bot per game, or one bot for everything. |
| `lighthouse set-password` | Turn on the dashboard or reset the main admin's password. |
| `lighthouse unblock <address>` | Lift a dashboard block. |
| `lighthouse update` | Check for and install a new version now. |
| `lighthouse --version` | Show the version. |

### Running it yourself

Rather not use the installers? Download the program for your system from the
[releases page](https://github.com/Hailey-Ross/hails.Lighthouse/releases/latest)
(`lighthouse-x86_64-pc-windows-msvc.exe` → rename to `lighthouse.exe`; on Linux
`lighthouse-x86_64-unknown-linux-gnu` → `lighthouse`, then `chmod +x lighthouse`), put it in a folder
of its own, run `lighthouse setup`, then `lighthouse run`. A systemd service file is in
[deploy/systemd](deploy/systemd/lighthouse.service), and there's a [Dockerfile](Dockerfile).

### Reaching the dashboard from other computers

Put it behind a reverse proxy with HTTPS (Caddy or nginx), set `bind` under `[dashboard]` to the
address the proxy connects to, and set `trust_proxy = true` so blocks apply to visitors rather than
the proxy.

## For developers

```sh
cargo build --release
cargo test
```

Rust 1.85 or newer. Releases are built by GitHub Actions: pushes to `dev` publish the
**bleeding-edge** pre-release, pushes to `main` publish **stable** (and a permanent `vX.Y.Z` release
when the version in `Cargo.toml` is new; it needs a section in [CHANGELOG.md](CHANGELOG.md)).

| Path | What |
|---|---|
| `crates/lighthouse` | The `lighthouse` program: setup, updates, command line, heartbeat listener |
| `crates/lighthouse-core` | Shared types: server status, config, the game catalogue, controls |
| `crates/lighthouse-games` | Game adapters: Steam query, RCON, telnet, Minecraft, FiveM, game APIs |
| `crates/lighthouse-discord` | Polling, status cards, slash commands and the Discord bots |
| `crates/lighthouse-dashboard` | Web dashboard; each page is a module in `src/modules/` |
| `crates/lighthouse-store` | SQLite storage: users, invites, sessions, history, events, bans |
| `mods/` | The FiveM/RedM resource, the 7 Days to Die mod and the Project Zomboid mod ([guide](docs/mods.md)) |
| `deploy/` | Installers, updaters, systemd unit |
| `docs/` | [Architecture](docs/architecture.md), [games](docs/games.md), [game extras](docs/mods.md), [heartbeat format](docs/heartbeat.md) |
| `assets/brand/` | Icon, logo, banner and animated icon (`tools/brand/render_brand.py` renders them) |

## Licence

Created by [Hailey Ross](https://github.com/Hailey-Ross).

Apache License 2.0, see [LICENSE](LICENSE) and [NOTICE](NOTICE). You're free to use, change and share
Lighthouse; keep the copyright and NOTICE file so the original authors are credited.
