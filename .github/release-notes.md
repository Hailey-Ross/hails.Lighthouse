## Setup

**What you need:** a Windows 10/11 or Linux PC (or anything that runs Docker) that stays on and can reach your game servers, and a Discord server you manage.

**Where to run it:** Lighthouse checks your servers over the network, so it doesn't have to run on the game server. With rented or managed game servers (only a control panel and FTP), run it on a home PC or a small VPS; nothing is installed on the game server. On your own machine you can run it right next to the games.

### Windows

1. **Download** `lighthouse-x86_64-pc-windows-msvc.exe` from the list above and rename it to `lighthouse.exe`. Put it in a folder of its own, such as `C:\Lighthouse`.
2. **Open a terminal there.** In that folder, right-click an empty space and choose **Open in Terminal**.
3. **Run the setup:** type `.\lighthouse setup` and press Enter. It asks for your game servers (type to search for the game), checks each one answers, then walks you through creating the Discord bot step by step.
   - If Windows shows **"Windows protected your PC"**, click **More info**, then **Run anyway**. This appears because Lighthouse is a free project without a paid signing certificate.
4. **Start it:** `.\lighthouse run`. Leave the window open; Lighthouse keeps watch while it runs.

### Linux

1. **Download** the file for your machine (`lighthouse-x86_64-unknown-linux-gnu` for most PCs and servers) and make it runnable:
   ```sh
   chmod +x lighthouse-x86_64-unknown-linux-gnu && mv lighthouse-x86_64-unknown-linux-gnu lighthouse
   ```
2. **Run the setup:** `./lighthouse setup`
3. **Start it:** `./lighthouse run`
4. **To keep it running in the background**, use the `lighthouse.service` file from this release: the comments at its top show the four commands to install it with systemd. Updates then install and restart on their own if you turned automatic updates on.

### Docker

```sh
docker build -t lighthouse https://github.com/Hailey-Ross/hails.Lighthouse.git
docker run -it --rm -v ./data:/data lighthouse setup
docker run -d --name lighthouse --restart unless-stopped -v ./data:/data lighthouse
```

**Quick setup:** the first question of `lighthouse setup` lets you set just a dashboard password and finish everything else in the web dashboard.

## The Discord bot

The setup tells you exactly what to click in Discord's Developer Portal, checks your bot token, and gives you the link that adds the bot to your server with the right permissions. You can run one bot for all your games, or one bot per game; switch any time with `lighthouse bot-mode per-game` or `lighthouse bot-mode mega`.

The bot keeps one status post per server (or one board for all of them) up to date in your status channel, and its status line rotates through your servers. Optional alerts post when a server goes down or comes back.

## Supported games

| Support | Games |
|---|---|
| **Full** (tested on live servers) | 7 Days to Die (in-game day, time and blood moon), Project Zomboid Build 42, VEIN |
| **Experimental** (should work, defaults not verified) | 100+ games that answer the Steam server query, such as Valheim, Rust, ARK, Counter-Strike 2, Conan Exiles, DayZ, Enshrouded and V Rising, plus Minecraft Java and Bedrock, and FiveM/RedM |

Run `lighthouse games` for the full list, or `lighthouse query <game> <address> [port]` to test a server before adding it. No game mods are needed.

**A server shows offline?** Some games answer status on a different port from the one players join: VEIN usually on the game port + 1. Find it with `lighthouse query`, then set `query_port` for that server. The [README](https://github.com/Hailey-Ross/hails.Lighthouse#troubleshooting) has more fixes.

## What's in it

- **Status posts** that update in place: online or offline, players, time since the last restart, map, version, and the join address if you want it shown.
- **A web dashboard** (off unless you turn it on): every server at a glance, uptime history and outages, population reports (players over time, peaks, player-hours, a heatmap of when people play, CSV export) and server log files when the game server runs on the same machine. Four calm colour themes, and it works on phones.
- **Users and permissions:** a superadmin made during setup invites other admins and picks exactly which pages and servers each can see. Sign-up is by invitation only; addresses that guess invite codes are blocked for longer each time.
- **Small and quiet:** a single program with no other software to install, one database file, and very little memory or disk use.
- **Updates:** Lighthouse checks this project's releases once a day and can install a new build and restart by itself. Choose **stable** or **bleeding-edge** under `[updates]` in `lighthouse.toml`.

Configuration reference: [`lighthouse.example.toml`](https://github.com/Hailey-Ross/hails.Lighthouse/blob/main/lighthouse.example.toml). Changes per version: [CHANGELOG.md](https://github.com/Hailey-Ross/hails.Lighthouse/blob/main/CHANGELOG.md).
