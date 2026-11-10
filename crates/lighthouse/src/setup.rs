//! `lighthouse setup`: interactive first-time setup that writes `lighthouse.toml`.

use std::collections::HashSet;
use std::path::Path;

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, Password, Select};
use lighthouse_core::{
    Config, DEFAULT_UPDATE_REPO, DashboardConfig, GameKind, IngestConfig, NativeApi, ServerConfig, UpdateConfig,
};

use crate::bots::{self, heading};
use crate::{password, update};

pub async fn run(path: &Path) -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();

    println!("Lighthouse setup");
    println!(
        "This walks you through creating {}. Press Ctrl+C at any time to cancel.",
        path.display()
    );

    if path.exists() {
        let replace = Confirm::with_theme(&theme)
            .with_prompt(format!("{} already exists. Replace it?", path.display()))
            .default(false)
            .interact()?;
        if !replace {
            println!("Setup cancelled; nothing was changed.");
            return Ok(());
        }
    }

    let quick = Select::with_theme(&theme)
        .with_prompt("How do you want to set up Lighthouse?")
        .items(&[
            "Full setup here in the terminal (servers, Discord bots, everything)",
            "Quick: set a dashboard password now and finish in the web dashboard",
        ])
        .default(0)
        .interact()?
        == 1;
    if quick {
        return quick_setup(&theme, path);
    }

    heading("Game servers");
    println!("Add each game server Lighthouse should watch. More can be added later in the config file.");
    let mut servers: Vec<ServerConfig> = Vec::new();
    loop {
        let taken: HashSet<String> = servers.iter().map(|s| s.id.clone()).collect();
        servers.push(ask_server(&theme, &taken).await?);
        if !Confirm::with_theme(&theme)
            .with_prompt("Add another server?")
            .default(false)
            .interact()?
        {
            break;
        }
    }
    heading("Discord bots");
    let (mode, bots) = bots::setup_bots(&theme, &servers).await?;

    heading("Status from game mods");
    println!(
        "The Lighthouse mods and agent send extra details (uptime, in-game time). If they run on another\n\
         machine they need a network address and a shared secret to reach this Lighthouse."
    );
    let ingest = if Confirm::with_theme(&theme)
        .with_prompt("Accept status from mods or agents on other machines?")
        .default(false)
        .interact()?
    {
        let bind: String = Input::with_theme(&theme)
            .with_prompt("Listen address")
            .default("0.0.0.0:8651".to_string())
            .interact_text()?;
        let secret = password::random_secret(32)?;
        println!("Shared secret (put this in each mod/agent config):\n  {secret}");
        Some(IngestConfig { bind, secret })
    } else {
        None
    };

    heading("Web dashboard");
    let (dashboard, admin) = if Confirm::with_theme(&theme)
        .with_prompt("Enable the web dashboard? (off by default, you can turn it on later)")
        .default(false)
        .interact()?
    {
        let (d, a) = ask_dashboard(&theme)?;
        (d, Some(a))
    } else {
        (DashboardConfig::default(), None)
    };

    heading("Updates");
    let mut updates = UpdateConfig {
        check: Confirm::with_theme(&theme)
            .with_prompt("Check GitHub for new Lighthouse versions?")
            .default(true)
            .interact()?,
        ..UpdateConfig::default()
    };
    if updates.check {
        if update::self_update_allowed() && update::asset_name().is_some() {
            updates.auto_install = Confirm::with_theme(&theme)
                .with_prompt("Install new versions automatically and restart?")
                .default(true)
                .interact()?;
        } else {
            println!("Automatic install is not available here; new versions will be reported in the log.");
        }
        updates.channel = match Select::with_theme(&theme)
            .with_prompt("Which builds?")
            .items(&[
                "Stable: the latest working build (recommended)",
                "Bleeding edge: every change as it lands; may break",
            ])
            .default(0)
            .interact()?
        {
            0 => "stable".to_string(),
            _ => "bleeding-edge".to_string(),
        };
        updates.repo = Input::with_theme(&theme)
            .with_prompt("Release source (GitHub owner/repository)")
            .default(DEFAULT_UPDATE_REPO.to_string())
            .validate_with(|r: &String| match r.split_once('/') {
                Some((o, n)) if !o.is_empty() && !n.is_empty() && !n.contains('/') => Ok(()),
                _ => Err("use the form owner/repository"),
            })
            .interact_text()?;
    }

    let config = Config {
        mode,
        poll_interval_secs: 30,
        database: "lighthouse.db".to_string(),
        bots,
        ingest,
        dashboard,
        updates,
        servers,
    };
    config.save(path)?;
    restrict_permissions(path);
    if let Some(admin) = &admin {
        store_superadmin(&config, path, admin)?;
    }

    heading("Done");
    println!("Saved {}.", path.display());
    println!("\nStart Lighthouse with:\n  lighthouse --config {} run", path.display());
    Ok(())
}

/// Superadmin login for the dashboard: the one account that can do everything.
pub(crate) struct Admin {
    pub username: String,
    pub password: String,
}

pub(crate) fn ask_superadmin(theme: &ColorfulTheme) -> anyhow::Result<Admin> {
    println!("The superadmin can do everything in the dashboard, including creating other admins.");
    let username: String = Input::with_theme(theme)
        .with_prompt("Superadmin username")
        .default("admin".to_string())
        .validate_with(|u: &String| {
            let ok = (2..=32).contains(&u.trim().len())
                && u.trim().chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
            if ok {
                Ok(())
            } else {
                Err("2-32 letters, digits, '-', '_' or '.'")
            }
        })
        .interact_text()?;
    let password = Password::with_theme(theme)
        .with_prompt("Superadmin password")
        .with_confirmation("Repeat password", "The passwords don't match")
        .validate_with(|p: &String| {
            if p.chars().count() >= 8 {
                Ok(())
            } else {
                Err("use at least 8 characters")
            }
        })
        .interact()?;
    Ok(Admin {
        username: username.trim().to_string(),
        password,
    })
}

/// Save the superadmin into the database named in `config` (created if missing).
pub(crate) fn store_superadmin(config: &Config, config_path: &Path, admin: &Admin) -> anyhow::Result<()> {
    let db = config.database_path(config_path);
    let store = lighthouse_store::Store::open(&db)?;
    store.set_superadmin(&admin.username, &admin.password, crate::now_unix())?;
    println!("Superadmin \"{}\" saved in {}.", admin.username, db.display());
    Ok(())
}

/// Ask for the dashboard address and the superadmin login; returns an enabled dashboard config.
fn ask_dashboard(theme: &ColorfulTheme) -> anyhow::Result<(DashboardConfig, Admin)> {
    let mut dashboard = DashboardConfig {
        enabled: true,
        ..DashboardConfig::default()
    };
    dashboard.bind = Input::with_theme(theme)
        .with_prompt("Dashboard address (127.0.0.1 = this machine only)")
        .default(dashboard.bind.clone())
        .interact_text()?;
    Ok((dashboard, ask_superadmin(theme)?))
}

/// Quick setup: only the dashboard, everything else is done in the browser.
fn quick_setup(theme: &ColorfulTheme, path: &Path) -> anyhow::Result<()> {
    heading("Quick setup");
    println!("Set a password for the web dashboard. You'll add servers and bots there.");
    let (dashboard, admin) = ask_dashboard(theme)?;
    let config = Config {
        dashboard,
        ..Config::default()
    };
    config.save(path)?;
    restrict_permissions(path);
    store_superadmin(&config, path, &admin)?;
    heading("Done");
    println!("Saved {}.", path.display());
    println!(
        "Start Lighthouse with:\n  lighthouse --config {} run\nthen open http://{} to finish the setup.",
        path.display(),
        config.dashboard.bind
    );
    Ok(())
}

/// Turn a display name into a unique config id, e.g. "Athens PvE" -> "athens-pve".
fn unique_id(name: &str, game: GameKind, taken: &HashSet<String>) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_end_matches('-').to_string();
    let base = if base.is_empty() {
        game.short_name().to_ascii_lowercase().replace('_', "-")
    } else {
        base
    };
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(id))
        .expect("unbounded range")
}

async fn ask_server(theme: &ColorfulTheme, taken: &HashSet<String>) -> anyhow::Result<ServerConfig> {
    let games = GameKind::all();
    let labels: Vec<String> = games
        .iter()
        .map(|g| {
            if g.is_experimental() {
                format!("{g} (experimental)")
            } else {
                g.to_string()
            }
        })
        .collect();
    let game = games[FuzzySelect::with_theme(theme)
        .with_prompt("Game (start typing to search)")
        .items(&labels)
        .default(0)
        .interact()?];

    let host: String = Input::with_theme(theme)
        .with_prompt("Server IP or hostname")
        .validate_with(|s: &String| {
            let s = s.trim();
            let has_port = s.contains(':') && !s.contains("::");
            if s.is_empty() || s.contains(char::is_whitespace) || has_port {
                Err("enter just the address, without a port")
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    let game_port: u16 = Input::with_theme(theme)
        .with_prompt("Port (the one players connect to)")
        .default(game.default_game_port())
        .interact_text()?;
    let name: String = Input::with_theme(theme)
        .with_prompt("Display name (empty = use the name the server reports)")
        .allow_empty(true)
        .interact_text()?;
    let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());

    let mut server = ServerConfig {
        id: unique_id(name.as_deref().unwrap_or_default(), game, taken),
        game,
        name,
        host: host.trim().to_string(),
        game_port,
        query_port: None,
        show_address: true,
        show_players: true,
        alerts: false,
        native: None,
        log_dir: None,
    };

    // Check the server right away so a wrong port is caught now, not after the bot is running.
    let adapter = lighthouse_games::adapter_for(game);
    let mut query_port = game.query_port_for(game_port);
    loop {
        server.query_port = (query_port != game.query_port_for(game_port)).then_some(query_port);
        print!("Checking {}:{query_port} ... ", server.host);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        match adapter.query(&server, crate::now_unix()).await {
            Ok(status) => {
                let players = match (status.players_online, status.players_max) {
                    (Some(n), Some(m)) => format!("{n}/{m} players"),
                    (Some(n), None) => format!("{n} players"),
                    _ => "player count unknown".to_string(),
                };
                println!(
                    "found \"{}\", {players}.",
                    status.name.as_deref().unwrap_or("unnamed server")
                );
                break;
            }
            Err(e) => {
                println!("no answer ({e}).");
                let choice = Select::with_theme(theme)
                    .with_prompt("What now?")
                    .items(&[
                        "Try a different query port",
                        "Keep it anyway (the server may be offline)",
                    ])
                    .default(0)
                    .interact()?;
                if choice != 0 {
                    break;
                }
                query_port = Input::with_theme(theme)
                    .with_prompt("Query port (often the game port, game port + 1, or 27015)")
                    .default(query_port)
                    .interact_text()?;
            }
        }
    }

    server.show_address = Confirm::with_theme(theme)
        .with_prompt("Show the join address on the status card?")
        .default(true)
        .interact()?;
    if adapter.capabilities(&server).player_names {
        server.show_players = Confirm::with_theme(theme)
            .with_prompt("List player names?")
            .default(true)
            .interact()?;
    }
    server.alerts = Confirm::with_theme(theme)
        .with_prompt("Post alerts when this server goes down or comes back?")
        .default(false)
        .interact()?;
    server.native = ask_native(theme, game)?;
    println!("Added {} server \"{}\".", game, server.id);
    Ok(server)
}

fn ask_native(theme: &ColorfulTheme, game: GameKind) -> anyhow::Result<Option<NativeApi>> {
    let question = if game.is(GameKind::SEVEN_DAYS_TO_DIE) {
        "Also use the built-in web API? (player names) [optional]"
    } else if game.is(GameKind::PROJECT_ZOMBOID) {
        "Also use RCON? (reliable player list) [optional]"
    } else if game.is(GameKind::VEIN) {
        "Also use VEIN's HTTP API? Only when Lighthouse runs on the same machine [optional]"
    } else {
        return Ok(None);
    };
    if !Confirm::with_theme(theme)
        .with_prompt(question)
        .default(false)
        .interact()?
    {
        return Ok(None);
    }
    let api = if game.is(GameKind::SEVEN_DAYS_TO_DIE) {
        println!(
            "In the server console run:  webtokens add lighthouse <secret> 0\n\
             (and set WebDashboardEnabled=true in serverconfig.xml)"
        );
        NativeApi::SevenDaysWebApi {
            port: Input::with_theme(theme)
                .with_prompt("Web dashboard port")
                .default(8080)
                .interact_text()?,
            token_name: Input::with_theme(theme)
                .with_prompt("Token name")
                .default("lighthouse".to_string())
                .interact_text()?,
            token_secret: Password::with_theme(theme).with_prompt("Token secret").interact()?,
        }
    } else if game.is(GameKind::PROJECT_ZOMBOID) {
        NativeApi::Rcon {
            port: Input::with_theme(theme)
                .with_prompt("RCON port (RCONPort)")
                .default(27015)
                .interact_text()?,
            password: Password::with_theme(theme)
                .with_prompt("RCON password (RCONPassword)")
                .interact()?,
        }
    } else {
        NativeApi::VeinHttp {
            port: Input::with_theme(theme)
                .with_prompt("HTTP API port (HTTPPort)")
                .default(8080)
                .interact_text()?,
        }
    };
    Ok(Some(api))
}

/// The config can hold tokens: make it readable by its owner only.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!("warning: could not restrict permissions on {}: {e}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_names() {
        let mut taken = HashSet::new();
        let game = GameKind::get(GameKind::SEVEN_DAYS_TO_DIE);
        assert_eq!(unique_id("[US] Athens PvE!", game, &taken), "us-athens-pve");
        assert_eq!(unique_id("", game, &taken), "7dtd");
        taken.insert("7dtd".to_string());
        assert_eq!(unique_id("", game, &taken), "7dtd-2");
    }
}
