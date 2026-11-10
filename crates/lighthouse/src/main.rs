mod bots;
mod http;
mod password;
mod setup;
mod update;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::Confirm;
use lighthouse_core::{Config, GameKind, ServerConfig, ServerState, ServerStatus};
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "lighthouse", version = update::version_text(), about = "Game server status boards for Discord")]
struct Cli {
    /// Path to the config file.
    #[arg(short, long, global = true, default_value = "lighthouse.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// First-time setup: asks a few questions and writes the config file.
    Setup,
    /// Start Lighthouse (bots, plus the dashboard and update checks if enabled).
    Run,
    /// Check the config file and exit.
    CheckConfig,
    /// Switch between one bot for all games and one bot per game (walks you through new bots).
    BotMode {
        #[arg(value_enum)]
        mode: ModeArg,
    },
    /// Create or reset the dashboard superadmin login (and enable the dashboard).
    SetPassword {
        /// Superadmin username (asks when not given).
        #[arg(long)]
        username: Option<String>,
        /// Read the password from this environment variable instead of asking (for unattended installs).
        #[arg(long, value_name = "VAR")]
        password_env: Option<String>,
    },
    /// Check for a new version and install it.
    Update {
        /// Only report whether an update is available.
        #[arg(long)]
        check: bool,
        /// Install the channel's build even if it isn't newer (or this is a local build).
        #[arg(long)]
        force: bool,
    },
    /// Print the status card JSON for each configured server, using placeholder data.
    PreviewCards,
    /// Lift a dashboard block on an address (e.g. if you blocked yourself).
    Unblock {
        /// The blocked IP address.
        ip: String,
    },
    /// List the games Lighthouse knows, with their default ports.
    Games {
        /// Only show games whose name contains this text.
        search: Option<String>,
    },
    /// Query one game server right now and print what Lighthouse sees. Needs no config file.
    Query {
        /// Game id, e.g. 7dtd, pz, vein, minecraft (see `lighthouse games`).
        #[arg(value_parser = parse_game)]
        game: GameKind,
        /// Server address.
        host: String,
        /// Query port (defaults to the game's usual one).
        port: Option<u16>,
        /// Also print the Discord card JSON.
        #[arg(long)]
        card: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    /// One bot shows every server of every game.
    Mega,
    /// One bot per game; servers are grouped by game automatically.
    PerGame,
}

fn parse_game(s: &str) -> Result<GameKind, String> {
    GameKind::from_id(s).ok_or_else(|| format!("unknown game {s:?}; `lighthouse games` lists them"))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Decides which status samples are worth writing to the history.
///
/// A change of state is always written at once (so uptime and outages stay exact). Otherwise a
/// server gets at most one row per minute while its player count moves, and one every
/// [`HistoryThrottle::QUIET`] while nothing changes. That keeps the database small without losing
/// anything the dashboard shows.
/// Last written sample per server: (time, state, players).
type LastSample = (i64, lighthouse_core::ServerState, Option<u32>);

#[derive(Default)]
struct HistoryThrottle {
    last: std::sync::Mutex<std::collections::HashMap<String, LastSample>>,
}

impl HistoryThrottle {
    const BUSY: i64 = 60;
    const QUIET: i64 = 120;

    fn should_record(&self, id: &str, s: &lighthouse_core::ServerStatus) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        let now = s.observed_at_unix;
        let write = match last.get(id) {
            None => true,
            Some(&(at, state, players)) => {
                let gap = now - at;
                state != s.state || (players != s.players_online && gap >= Self::BUSY) || gap >= Self::QUIET
            }
        };
        if write {
            last.insert(id.to_string(), (now, s.state, s.players_online));
        }
        write
    }
}

/// Keep 30 days of status history; prune once an hour.
async fn prune_history(store: Arc<lighthouse_store::Store>) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        every.tick().await;
        match store.prune_history(now_unix() - 30 * 86_400) {
            Ok(0) => {}
            Ok(n) => {
                tracing::debug!("pruned {n} old status samples");
                store.compact();
            }
            Err(e) => tracing::warn!("could not prune status history: {e}"),
        }
    }
}

/// `lighthouse.toml` -> `lighthouse.state.json` in the same folder.
fn state_file(config: &Path) -> PathBuf {
    let stem = config
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lighthouse".into());
    config.with_file_name(format!("{stem}.state.json"))
}

fn load(path: &Path) -> anyhow::Result<Config> {
    Config::load(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // RUST_LOG takes a plain level: error, warn, info (default), debug or trace.
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| {
            v.split(',')
                .next()
                .and_then(|l| l.trim().parse::<tracing::Level>().ok())
        })
        .unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt().with_max_level(level).init();

    let cli = Cli::parse();
    match dispatch(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    let path = cli.config;
    match cli.command {
        Command::Setup => setup::run(&path).await,
        Command::BotMode { mode } => {
            let target = match mode {
                ModeArg::Mega => lighthouse_core::BotMode::Mega,
                ModeArg::PerGame => lighthouse_core::BotMode::PerGame,
            };
            bots::switch_mode(&path, target).await
        }
        Command::Run => run(&path).await,
        Command::CheckConfig => {
            let config = load(&path)?;
            println!(
                "{} is valid: {} bot(s), {} server(s), mode {:?}, dashboard {}",
                path.display(),
                config.bots.len(),
                config.servers.len(),
                config.mode,
                if config.dashboard.enabled { "on" } else { "off" },
            );
            Ok(())
        }
        Command::SetPassword { username, password_env } => {
            let mut config = load(&path)?;
            let admin = match password_env {
                Some(var) => setup::Admin {
                    username: username.unwrap_or_else(|| "admin".to_string()),
                    password: std::env::var(&var)
                        .map_err(|_| anyhow::anyhow!("environment variable {var} is not set"))?,
                },
                None => setup::ask_superadmin(&dialoguer::theme::ColorfulTheme::default())?,
            };
            setup::store_superadmin(&config, &path, &admin)?;
            if !config.dashboard.enabled {
                config.dashboard.enabled = true;
                config.save(&path)?;
            }
            println!("Dashboard enabled at http://{}", config.dashboard.bind);
            Ok(())
        }
        Command::Update { check, force } => {
            let config = load(&path).map(|c| c.updates).unwrap_or_default();
            let me = update::this_build();
            let release = update::latest(&config).await?;
            println!("This is Lighthouse {}.", me.describe());
            println!(
                "Latest on {}: {}  ({})",
                config.channel,
                release.describe(),
                release.page_url
            );
            let newer = update::is_newer(&me, &release.info);
            if check {
                return Ok(());
            }
            if newer || force {
                update::install(&release).await?;
                println!("Updated to {}. Restart Lighthouse to use it.", release.describe());
            } else if me.build.is_none() {
                println!(
                    "This is a local build, so it isn't replaced automatically. Use --force to install the release."
                );
            } else {
                println!("Already up to date.");
            }
            Ok(())
        }
        Command::Query { game, host, port, card } => {
            let server = ServerConfig {
                id: "query".to_string(),
                game,
                name: None,
                host,
                game_port: port.unwrap_or(game.default_game_port()),
                query_port: Some(port.unwrap_or(game.default_query_port())),
                show_address: false,
                show_players: true,
                alerts: false,
                native: None,
                log_dir: None,
            };
            let now = now_unix();
            let status = lighthouse_games::adapter_for(game).query(&server, now).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            if card {
                let card = lighthouse_discord::render::server_card(&server, &status, now);
                println!("{}", serde_json::to_string_pretty(&card)?);
            }
            Ok(())
        }
        Command::Unblock { ip } => {
            let config = load(&path)?;
            let store = lighthouse_store::Store::open(&config.database_path(&path))?;
            if store.unblock(ip.trim())? {
                println!("Lifted the block on {ip}.");
            } else {
                println!("{ip} was not blocked.");
            }
            Ok(())
        }
        Command::Games { search } => {
            let games = match search.as_deref() {
                Some(text) => GameKind::search(text),
                None => GameKind::all(),
            };
            println!("{:<28} {:<36} {:>6} {:>6}  support", "id", "name", "port", "query");
            for g in &games {
                println!(
                    "{:<28} {:<36} {:>6} {:>6}  {}",
                    g.id(),
                    g.display_name(),
                    g.default_game_port(),
                    g.default_query_port(),
                    if g.is_experimental() { "experimental" } else { "full" }
                );
            }
            println!("{} game(s)", games.len());
            Ok(())
        }
        Command::PreviewCards => {
            let config = load(&path)?;
            let now = now_unix();
            for server in &config.servers {
                let mut status = ServerStatus::new(ServerState::Online, now);
                status.players_online = Some(0);
                status.players_max = Some(0);
                status.started_at_unix = Some(now - 3600);
                let card = lighthouse_discord::render::server_card(server, &status, now);
                println!("{}", serde_json::to_string_pretty(&card)?);
            }
            Ok(())
        }
    }
}

async fn run(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        let interactive = std::io::stdin().is_terminal();
        if interactive
            && Confirm::new()
                .with_prompt(format!("{} not found. Run first-time setup now?", path.display()))
                .default(true)
                .interact()?
        {
            setup::run(path).await?;
            if !path.exists() {
                return Ok(());
            }
        } else {
            anyhow::bail!("{} not found; run `lighthouse setup` first", path.display());
        }
    }
    let config = load(path)?;

    if config.servers.is_empty() && !config.dashboard.enabled {
        anyhow::bail!(
            "no servers configured; run `lighthouse setup` or add [[servers]] to {}",
            path.display()
        );
    }
    if config.bots.is_empty() {
        info!("no bots configured: polling only");
    }

    // Database: dashboard users and status history.
    let db_path = config.database_path(path);
    let store =
        Arc::new(lighthouse_store::Store::open(&db_path).map_err(|e| anyhow::anyhow!("{}: {e}", db_path.display()))?);
    let recorder: lighthouse_discord::monitor::Recorder = {
        let store = Arc::clone(&store);
        let throttle = HistoryThrottle::default();
        Arc::new(move |id: &str, status: &lighthouse_core::ServerStatus| {
            if !throttle.should_record(id, status) {
                return;
            }
            if let Err(e) = store.record(id, status) {
                tracing::warn!("could not save status history: {e}");
            }
        })
    };
    tokio::spawn(prune_history(Arc::clone(&store)));

    let persist = Arc::new(lighthouse_discord::persist::Persist::load(&state_file(path)));
    let monitor = lighthouse_discord::monitor::Monitor::new(&config.servers, Some(recorder));
    monitor.spawn_pollers(
        config.servers.clone(),
        std::time::Duration::from_secs(config.poll_interval_secs),
        Arc::clone(&persist),
    );
    info!(
        servers = config.servers.len(),
        every = config.poll_interval_secs,
        "polling started"
    );
    lighthouse_discord::start_bots(&config, &monitor, &persist);

    if config.dashboard.enabled {
        if store.has_superadmin()? {
            let bind = config.dashboard.bind.clone();
            let ctx = Arc::new(lighthouse_dashboard::Context {
                config: config.clone(),
                monitor: Arc::clone(&monitor),
                store: Arc::clone(&store),
            });
            tokio::spawn(async move {
                if let Err(e) = lighthouse_dashboard::serve(ctx, &bind).await {
                    error!("dashboard stopped: {e} (is {bind} already in use?)");
                }
            });
        } else {
            error!("the dashboard is enabled but has no superadmin yet; run `lighthouse set-password`");
        }
    }
    if config.servers.is_empty() {
        info!("no servers configured yet; add them in the config file (adding them in the dashboard is coming)");
    }
    tokio::spawn(update::service(config.updates.clone()));

    info!("Lighthouse is running; press Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    info!("stopping");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lighthouse_core::{ServerState, ServerStatus};

    fn st(at: i64, state: ServerState, players: Option<u32>) -> ServerStatus {
        let mut s = ServerStatus::new(state, at);
        s.players_online = players;
        s
    }

    #[test]
    fn history_throttle_keeps_changes_and_skips_repeats() {
        let t = HistoryThrottle::default();
        assert!(t.should_record("a", &st(0, ServerState::Online, Some(3))));
        assert!(
            !t.should_record("a", &st(30, ServerState::Online, Some(3))),
            "nothing changed"
        );
        assert!(
            !t.should_record("a", &st(45, ServerState::Online, Some(4))),
            "players moved, but under a minute"
        );
        assert!(
            t.should_record("a", &st(65, ServerState::Online, Some(5))),
            "players moved, a minute later"
        );
        assert!(
            t.should_record("a", &st(70, ServerState::Offline, None)),
            "state changes are written at once"
        );
        assert!(!t.should_record("a", &st(100, ServerState::Offline, None)));
        assert!(
            t.should_record("a", &st(190, ServerState::Offline, None)),
            "quiet heartbeat every 2 minutes"
        );
        assert!(
            t.should_record("b", &st(100, ServerState::Online, Some(1))),
            "servers are tracked separately"
        );
    }
}
