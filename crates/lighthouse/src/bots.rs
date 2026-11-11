//! Discord bot setup: a step-by-step Developer Portal walkthrough, token and channel checks, and
//! switching between one megabot and one bot per game.

use std::collections::BTreeMap;
use std::path::Path;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Password, Select};
use lighthouse_core::{BotConfig, BotMode, Config, GameKind, ServerConfig};
use serde::Deserialize;

/// View Channels, Send Messages, Embed Links, Attach Files, Read Message History.
pub const BOT_PERMISSIONS: u64 = (1 << 10) | (1 << 11) | (1 << 14) | (1 << 15) | (1 << 16);
const PERMISSION_NAMES: &str = "View Channels, Send Messages, Embed Links, Attach Files, Read Message History";
const PORTAL: &str = "https://discord.com/developers/applications";
const API: &str = "https://discord.com/api/v10";

pub fn heading(text: &str) {
    println!("\n── {text} ──");
}

pub fn invite_url(application_id: &str) -> String {
    format!(
        "https://discord.com/oauth2/authorize?client_id={application_id}&scope=bot%20applications.commands&permissions={BOT_PERMISSIONS}&integration_type=0"
    )
}

fn label(game: Option<GameKind>) -> String {
    match game {
        Some(g) => format!("{g} bot"),
        None => "Lighthouse bot (all games)".to_string(),
    }
}

fn app_name(game: Option<GameKind>) -> String {
    match game {
        Some(g) => format!("Lighthouse {g}"),
        None => "Lighthouse".to_string(),
    }
}

fn pause(theme: &ColorfulTheme) -> anyhow::Result<()> {
    Input::<String>::with_theme(theme)
        .with_prompt("Press Enter when done")
        .allow_empty(true)
        .report(false)
        .interact_text()?;
    Ok(())
}

/// Servers grouped by game, in catalogue order (fully supported games first).
pub fn plan(servers: &[ServerConfig]) -> BTreeMap<GameKind, Vec<&ServerConfig>> {
    let mut groups: BTreeMap<GameKind, Vec<&ServerConfig>> = BTreeMap::new();
    for s in servers {
        groups.entry(s.game).or_default().push(s);
    }
    groups
}

fn server_label(s: &ServerConfig) -> String {
    s.name.clone().unwrap_or_else(|| s.id.clone())
}

fn print_plan(servers: &[ServerConfig]) {
    for (game, list) in plan(servers) {
        let names: Vec<String> = list.iter().map(|s| server_label(s)).collect();
        println!("  • {game} bot → {}", names.join(", "));
    }
}

// ---------- Discord checks ----------

#[derive(Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
}

#[derive(Deserialize)]
struct DiscordChannel {
    #[serde(default)]
    name: Option<String>,
}

enum Check<T> {
    Ok(T),
    BadToken,
    Forbidden,
    NotFound,
    Unreachable(String),
}

async fn discord_get<T: for<'de> Deserialize<'de>>(token: &str, path: &str) -> Check<T> {
    let resp = match crate::http::client()
        .get(format!("{API}{path}"))
        .header("Authorization", format!("Bot {token}"))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return Check::Unreachable(e.to_string()),
    };
    match resp.status().as_u16() {
        200 => match resp.json::<T>().await {
            Ok(v) => Check::Ok(v),
            Err(e) => Check::Unreachable(e.to_string()),
        },
        401 => Check::BadToken,
        403 => Check::Forbidden,
        404 => Check::NotFound,
        code => Check::Unreachable(format!("HTTP {code}")),
    }
}

pub(crate) fn ask_id(theme: &ColorfulTheme, prompt: &str, optional: bool) -> anyhow::Result<Option<u64>> {
    let text: String = Input::with_theme(theme)
        .with_prompt(prompt)
        .allow_empty(optional)
        .validate_with(move |s: &String| {
            let s = s.trim();
            if (optional && s.is_empty()) || s.parse::<u64>().is_ok_and(|n| n > 0) {
                Ok(())
            } else {
                Err("paste the numeric ID (right-click → Copy Channel ID, with Developer Mode on)")
            }
        })
        .interact_text()?;
    let text = text.trim();
    Ok(if text.is_empty() { None } else { Some(text.parse()?) })
}

/// Ask for a channel ID until the bot can see the channel (or the user chooses to keep it anyway).
async fn ask_channel(theme: &ColorfulTheme, token: &str, prompt: &str, optional: bool) -> anyhow::Result<Option<u64>> {
    loop {
        let Some(id) = ask_id(theme, prompt, optional)? else {
            return Ok(None);
        };
        let problem = match discord_get::<DiscordChannel>(token, &format!("/channels/{id}")).await {
            Check::Ok(ch) => {
                println!("  ✓ The bot can see #{}.", ch.name.unwrap_or_else(|| id.to_string()));
                return Ok(Some(id));
            }
            Check::Forbidden => "The bot can't see that channel. Make sure you finished the invite step, and that \
                                 the channel lets the bot View Channel and Send Messages."
                .to_string(),
            Check::NotFound => "Discord doesn't know that channel for this bot. Check the ID, and that the bot was \
                                invited to the server the channel is in."
                .to_string(),
            Check::BadToken => "Discord rejected the bot token.".to_string(),
            Check::Unreachable(e) => format!("Could not check the channel ({e})."),
        };
        println!("  ✗ {problem}");
        let choice = Select::with_theme(theme)
            .with_prompt("What now?")
            .items(&["Try again", "Keep this ID anyway"])
            .default(0)
            .interact()?;
        if choice == 1 {
            return Ok(Some(id));
        }
    }
}

/// Pick a channel from the ones the bot can see; fall back to typing its ID (needs Discord's
/// Developer Mode) when the list can't be fetched or the channel isn't in it.
async fn pick_channel(theme: &ColorfulTheme, token: &str, prompt: &str, optional: bool) -> anyhow::Result<Option<u64>> {
    let channels = lighthouse_discord::setup::channels(token).await.unwrap_or_default();
    if !channels.is_empty() {
        let mut items: Vec<String> = channels.iter().map(|c| format!("{} › #{}", c.guild, c.name)).collect();
        items.push("Another channel (type its ID)".to_string());
        if optional {
            items.push("None".to_string());
        }
        let choice = Select::with_theme(theme)
            .with_prompt(prompt)
            .items(&items)
            .default(0)
            .interact()?;
        if choice < channels.len() {
            println!("  ✓ #{}", channels[choice].name);
            return Ok(Some(channels[choice].id));
        }
        if optional && choice == channels.len() + 1 {
            return Ok(None);
        }
    } else {
        println!("  The bot isn't in a Discord server yet (finish the invite step), or Discord didn't answer.");
    }
    println!(
        "  To get a channel's ID: in Discord turn on Developer Mode (User Settings → Advanced), then\n  \
         right-click the channel and choose \"Copy Channel ID\"."
    );
    ask_channel(theme, token, &format!("{prompt} ID"), optional).await
}

// ---------- the walkthrough ----------

/// Walk the user through creating one bot in the Developer Portal and return its config.
pub async fn guided_bot(theme: &ColorfulTheme, game: Option<GameKind>) -> anyhow::Result<BotConfig> {
    heading(&label(game));

    println!(
        "Step 1 of 6: create the application\n  \
         Open {PORTAL}\n  \
         Click \"New Application\", name it \"{}\", accept the terms and click Create.",
        app_name(game)
    );
    pause(theme)?;

    println!(
        "Step 2 of 6: Bot settings (\"Bot\" in the left menu)\n  \
         • Public Bot: OFF\n  \
         • Requires OAuth2 Code Grant: OFF\n  \
         • Presence Intent, Server Members Intent, Message Content Intent: all OFF\n  \
         • Click Save Changes"
    );
    pause(theme)?;

    println!(
        "Step 3 of 6: Installation settings (\"Installation\" in the left menu)\n  \
         • Installation Contexts: untick \"User Install\", keep \"Guild Install\"\n  \
         • Install Link: None\n  \
         • Click Save Changes"
    );
    pause(theme)?;

    println!("Step 4 of 6: the token\n  On the Bot page click \"Reset Token\", confirm, then copy the token.");
    let (token, app_id) = loop {
        let token = Password::with_theme(theme)
            .with_prompt("Paste the token (hidden)")
            .interact()?;
        let token = token.trim().to_string();
        match discord_get::<DiscordUser>(&token, "/users/@me").await {
            Check::Ok(user) => {
                println!("  ✓ Token works: this bot is \"{}\".", user.username);
                break (token, Some(user.id));
            }
            Check::BadToken => println!("  ✗ Discord rejected that token. Reset it again and copy the new one."),
            other => {
                let why = match other {
                    Check::Unreachable(e) => e,
                    _ => "unexpected reply".to_string(),
                };
                println!("  ? Could not check the token ({why}).");
                if Confirm::with_theme(theme)
                    .with_prompt("Use it anyway?")
                    .default(true)
                    .interact()?
                {
                    break (token, None);
                }
            }
        }
    };

    println!("Step 5 of 6: add the bot to your Discord server");
    match &app_id {
        Some(id) => println!(
            "  Open this link, choose your server and click Authorize:\n  {}\n  \
             (It grants exactly: {PERMISSION_NAMES}.)",
            invite_url(id)
        ),
        None => println!(
            "  In the portal open OAuth2 → URL Generator. Tick the scopes \"bot\" and\n  \
             \"applications.commands\", then the permissions {PERMISSION_NAMES}.\n  \
             Open the generated link, choose your server and click Authorize."
        ),
    }
    pause(theme)?;

    println!("Step 6 of 6: the status channel\n  Pick the channel the status post should go in.");
    let status_channel_id = pick_channel(theme, &token, "Status channel", false)
        .await?
        .expect("required channel");

    let (alert_channel_id, alert_role_id) = if Confirm::with_theme(theme)
        .with_prompt("Also post alerts when a server goes down or comes back?")
        .default(false)
        .interact()?
    {
        let channel = pick_channel(theme, &token, "Alert channel (can be the same channel)", false).await?;
        let role = ask_id(
            theme,
            "Role to ping with alerts (right-click role → Copy Role ID; empty = none)",
            true,
        )?;
        (channel, role)
    } else {
        (None, None)
    };

    let title = if game.is_none() {
        let t: String = Input::with_theme(theme)
            .with_prompt("Board title (for example your community's name)")
            .default(lighthouse_discord::render::DEFAULT_TITLE.to_string())
            .interact_text()?;
        Some(t.trim().to_string()).filter(|t| !t.is_empty() && t != lighthouse_discord::render::DEFAULT_TITLE)
    } else {
        None
    };

    let env_name = match game {
        Some(g) => format!("LIGHTHOUSE_{}_TOKEN", g.short_name()),
        None => "LIGHTHOUSE_TOKEN".to_string(),
    };
    let in_file = Select::with_theme(theme)
        .with_prompt("Where should the token be kept?")
        .items(&[
            "In the config file (simplest; keep the file private)",
            format!("In the environment variable {env_name}").as_str(),
        ])
        .default(0)
        .interact()?
        == 0;
    let (token_field, token_env) = if in_file {
        (Some(token), None)
    } else {
        println!("  Remember to set {env_name} before starting Lighthouse.");
        (None, Some(env_name))
    };

    println!("  ✓ {} is ready.", label(game));
    Ok(BotConfig {
        game,
        token: token_field,
        token_env,
        status_channel_id,
        title,
        alert_channel_id,
        alert_role_id,
    })
}

/// Setup step: choose megabot or one bot per game, then walk through creating each bot.
pub async fn setup_bots(theme: &ColorfulTheme, servers: &[ServerConfig]) -> anyhow::Result<(BotMode, Vec<BotConfig>)> {
    let games: Vec<GameKind> = plan(servers).into_keys().collect();
    let mut mode = BotMode::Mega;
    if games.len() > 1 {
        let pick = Select::with_theme(theme)
            .with_prompt("How many Discord bots?")
            .items(&[
                "One bot for all games (megabot): one board with every server",
                "One bot per game: each game gets its own bot, board and status line",
            ])
            .default(0)
            .interact()?;
        if pick == 1 {
            println!("Your servers will be split by game:");
            print_plan(servers);
            if Confirm::with_theme(theme)
                .with_prompt(format!("Set up {} bots like this?", games.len()))
                .default(true)
                .interact()?
            {
                mode = BotMode::PerGame;
            } else {
                println!("OK, using one bot for all games. Switch later with `lighthouse bot-mode per-game`.");
            }
        }
    }

    println!(
        "\nYou'll now create {} in Discord's Developer Portal. Each step says exactly what to click.",
        if mode == BotMode::PerGame {
            format!("{} bots", games.len())
        } else {
            "the bot".to_string()
        }
    );
    let mut bots = Vec::new();
    match mode {
        BotMode::PerGame => {
            let total = games.len();
            for (i, game) in games.into_iter().enumerate() {
                println!("\nBot {} of {total}", i + 1);
                bots.push(guided_bot(theme, Some(game)).await?);
            }
        }
        BotMode::Mega => bots.push(guided_bot(theme, None).await?),
    }
    Ok((mode, bots))
}

// ---------- `lighthouse bot-mode` ----------

/// Switch an existing config between one megabot and one bot per game.
pub async fn switch_mode(path: &Path, target: BotMode) -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();
    let mut config = Config::load(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    if config.servers.is_empty() {
        anyhow::bail!("no servers configured yet; run `lighthouse setup` first");
    }
    let games: Vec<GameKind> = plan(&config.servers).into_keys().collect();

    match target {
        BotMode::PerGame => {
            if config.mode == BotMode::PerGame && config.bots.iter().all(|b| b.game.is_some()) {
                println!("Already one bot per game.");
                return Ok(());
            }
            heading("Split into one bot per game");
            println!("Your servers will be split by game:");
            print_plan(&config.servers);
            if !Confirm::with_theme(&theme)
                .with_prompt("Continue?")
                .default(true)
                .interact()?
            {
                return Ok(());
            }

            let mut bots: Vec<BotConfig> = Vec::new();
            if let Some(current) = config.bots.first().cloned() {
                let mut options: Vec<String> = games
                    .iter()
                    .map(|g| format!("Keep it for {g}, set up new bots for the rest"))
                    .collect();
                options.push("Set up a new bot for every game".to_string());
                let pick = Select::with_theme(&theme)
                    .with_prompt("What should happen to your current bot?")
                    .items(&options)
                    .default(0)
                    .interact()?;
                if let Some(&game) = games.get(pick) {
                    bots.push(BotConfig {
                        game: Some(game),
                        ..current
                    });
                    println!(
                        "  The current bot now shows only {game}. Delete its old combined board message in Discord;\n  \
                         it will post a fresh one for each server."
                    );
                } else {
                    println!(
                        "  Your current bot stops posting. Delete its old board message in Discord and remove the\n  \
                         bot from your server if you no longer need it."
                    );
                }
            }
            let remaining: Vec<GameKind> = games
                .iter()
                .copied()
                .filter(|g| !bots.iter().any(|b| b.game == Some(*g)))
                .collect();
            println!(
                "\nNext: {} new bot(s), one per game. Each step says exactly what to click.",
                remaining.len()
            );
            for (i, game) in remaining.iter().enumerate() {
                println!("\nBot {} of {}", i + 1, remaining.len());
                bots.push(guided_bot(&theme, Some(*game)).await?);
            }
            config.mode = BotMode::PerGame;
            config.bots = bots;
        }
        BotMode::Mega => {
            if config.mode == BotMode::Mega {
                println!("Already using one bot for all games.");
                return Ok(());
            }
            heading("Combine into one bot for all games");
            let keep = if config.bots.is_empty() {
                None
            } else {
                let names: Vec<String> = config.bots.iter().map(|b| label(b.game)).collect();
                let i = Select::with_theme(&theme)
                    .with_prompt("Which bot should become the bot for all games?")
                    .items(&names)
                    .default(0)
                    .interact()?;
                Some(config.bots[i].clone())
            };
            let bot = match keep {
                Some(b) => BotConfig { game: None, ..b },
                None => guided_bot(&theme, None).await?,
            };
            if config.bots.len() > 1 {
                println!(
                    "  The other bots stop posting. Delete their old status messages in Discord and remove\n  \
                     those bots from your server if you no longer need them."
                );
            }
            config.mode = BotMode::Mega;
            config.bots = vec![bot];
        }
    }

    config.save(path)?;
    println!("Saved {}. Restart Lighthouse to apply.", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str, game: &str) -> ServerConfig {
        ServerConfig {
            id: id.into(),
            game: GameKind::from_id(game).unwrap(),
            name: None,
            host: "127.0.0.1".into(),
            game_port: 1,
            query_port: None,
            show_address: true,
            show_players: true,
            alerts: false,
            native: None,
            log_dir: None,
            control: None,
            process: None,
            status_file: None,
            telnet: None,
        }
    }

    #[test]
    fn plan_groups_by_game_in_catalogue_order() {
        let servers = [
            server("vein", "vein"),
            server("athens", "7dtd"),
            server("pz", "pz"),
            server("oceania", "7dtd"),
        ];
        let p = plan(&servers);
        let order: Vec<&str> = p.keys().map(|g| g.id()).collect();
        assert_eq!(order, ["seven_days_to_die", "project_zomboid", "vein"]);
        let sdtd: Vec<&str> = p.values().next().unwrap().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(sdtd, ["athens", "oceania"]);
    }

    #[test]
    fn invite_link_has_scopes_and_permissions() {
        let url = invite_url("123");
        assert!(url.contains("client_id=123"));
        assert!(url.contains("scope=bot%20applications.commands"));
        assert!(url.contains("permissions=117760"));
    }
}
