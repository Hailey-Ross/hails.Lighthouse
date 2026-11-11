## Install

**What you need:** a Windows 10/11 or Linux computer that stays on (your own PC is fine; it doesn't have to be the game server), a Discord server where you're an admin, and each game server's address and port (your host's control panel shows them).

**First, make your Discord bot** (3 minutes): in the [Discord Developer Portal](https://discord.com/developers/applications) click **New Application**, give it a name, then on the **Bot** page click **Reset Token** and **Copy**. The installer asks for that token.

### Windows

1. Download **install-windows.ps1** from the list above.
2. In your Downloads folder, **right-click** it and choose **Run with PowerShell**. Click **Yes** when Windows asks for permission.
3. Answer the questions. Not sure? Pick the **quick setup** and do the rest in the dashboard that opens at the end.

Lighthouse goes into `C:\Lighthouse`, runs in the background and starts by itself with Windows.

### Linux

```sh
curl -fsSLO https://github.com/Hailey-Ross/hails.Lighthouse/releases/download/stable/install-linux.sh
sudo sh install-linux.sh
```

Answer the questions; Lighthouse then runs as a service that starts at boot.

**Already installed?** Run the installer again: it updates Lighthouse and keeps every setting and file (it backs them up and checks they came through unchanged).

The full walkthrough, with what each question means: [Getting started](https://github.com/Hailey-Ross/hails.Lighthouse#getting-started).

### By hand or with Docker

Download the program for your system above, put it in a folder of its own (on Windows rename it to `lighthouse.exe`; on Linux to `lighthouse` and `chmod +x` it), run `lighthouse setup`, then `lighthouse run`. Docker:

```sh
docker build -t lighthouse https://github.com/Hailey-Ross/hails.Lighthouse.git
docker run -it --rm -v ./data:/data lighthouse setup
docker run -d --name lighthouse --restart unless-stopped -v ./data:/data lighthouse
```

If Windows shows **"Windows protected your PC"** for the program, click **More info**, then **Run anyway**: Lighthouse is a free project without a paid signing certificate.

## Game extras

Want real uptime, player names, a live console, and restart/kick/ban buttons on the dashboard? Each game has an optional extra, explained step by step in the [game extras guide](https://github.com/Hailey-Ross/hails.Lighthouse/blob/main/docs/mods.md). The **FiveM / RedM resource** and the **Project Zomboid mod** are in the downloads above, ready to copy onto the server.

## Supported games

| Support | Games |
|---|---|
| **Full** (tested on live servers) | 7 Days to Die, Project Zomboid (Build 42), VEIN, FiveM, RedM |
| **Experimental** (should work, defaults not verified) | 100+ more, such as Valheim, Rust, ARK, Counter-Strike 2, Conan Exiles, DayZ, Enshrouded, V Rising and Minecraft (Java and Bedrock) |

**A server shows offline?** Some games answer status on a different port from the one players join (VEIN: the game port + 1). Your host's panel may list a "query port". See [Something isn't working](https://github.com/Hailey-Ross/hails.Lighthouse#something-isnt-working).

## What's in it

- **Status posts** in Discord that update themselves: online or offline, players, uptime, map, version, and for 7 Days to Die the in-game day and blood moon. `/status` lets anyone check the servers.
- **A dashboard** in your browser: every server at a glance, uptime history, player reports, live logs, server controls and moderation (kicks, bans, txAdmin history), and a **Setup** page to add servers and bots.
- **Admins:** invite others by link and choose exactly what each can see and do.
- **Small and quiet:** one program, one database file, very little memory.
- **Updates** install by themselves if you said yes during setup.

Every setting: [`lighthouse.example.toml`](https://github.com/Hailey-Ross/hails.Lighthouse/blob/main/lighthouse.example.toml). Changes per version: [CHANGELOG.md](https://github.com/Hailey-Ross/hails.Lighthouse/blob/main/CHANGELOG.md).
