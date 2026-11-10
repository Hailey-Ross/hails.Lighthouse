//! The `lighthouse.toml` configuration file. See `lighthouse.example.toml` for a commented example.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::game::GameKind;

/// How bots are assigned to games.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotMode {
    /// One bot per game (default). Each `[[bots]]` entry has a `game`.
    #[default]
    PerGame,
    /// One bot shows every server of every game. Exactly one `[[bots]]` entry, without `game`.
    Mega,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub mode: BotMode,
    /// Seconds between polls of each server.
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// SQLite database file.
    #[serde(default = "default_database")]
    pub database: String,
    #[serde(default)]
    pub bots: Vec<BotConfig>,
    #[serde(default)]
    pub ingest: Option<IngestConfig>,
    /// Local web dashboard. Off unless `enabled = true`.
    #[serde(default)]
    pub dashboard: DashboardConfig,
    /// Automatic updates from GitHub releases.
    #[serde(default)]
    pub updates: UpdateConfig,
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
}

impl Default for Config {
    /// The same values an empty config file gives.
    fn default() -> Self {
        Self {
            mode: BotMode::default(),
            poll_interval_secs: default_poll_interval(),
            database: default_database(),
            bots: Vec::new(),
            ingest: None,
            dashboard: DashboardConfig::default(),
            updates: UpdateConfig::default(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotConfig {
    /// The game this bot broadcasts. Leave out in `mega` mode.
    #[serde(default)]
    pub game: Option<GameKind>,
    /// Discord bot token. Prefer `token_env` so the token stays out of the file.
    #[serde(default)]
    pub token: Option<String>,
    /// Name of an environment variable that holds the token.
    #[serde(default)]
    pub token_env: Option<String>,
    /// Channel where the status board lives.
    pub status_channel_id: u64,
    /// Heading of the combined board (one-bot-for-all-games mode), e.g. your community's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Channel for up/down alerts. Alerts are only sent for servers with `alerts = true`.
    #[serde(default)]
    pub alert_channel_id: Option<u64>,
    /// Role pinged with alerts.
    #[serde(default)]
    pub alert_role_id: Option<u64>,
}

/// Where mods and agents push heartbeats to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestConfig {
    #[serde(default = "default_ingest_bind")]
    pub bind: String,
    /// Shared secret every heartbeat must carry.
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    /// The dashboard only starts when this is `true`. Off by default.
    #[serde(default)]
    pub enabled: bool,
    /// Defaults to localhost only. Put it behind a reverse proxy with TLS before exposing it.
    #[serde(default = "default_dashboard_bind")]
    pub bind: String,
    /// Behind a reverse proxy, take the visitor's address from `X-Forwarded-For`. Only turn this on
    /// when the dashboard is reachable through the proxy alone, or visitors could fake addresses.
    #[serde(default)]
    pub trust_proxy: bool,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_dashboard_bind(),
            trust_proxy: false,
        }
    }
}

/// GitHub repository updates come from by default.
pub const DEFAULT_UPDATE_REPO: &str = "Hailey-Ross/hails.Lighthouse";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    /// Check for new releases.
    #[serde(default = "yes")]
    pub check: bool,
    /// Download and install new releases automatically, then restart.
    #[serde(default)]
    pub auto_install: bool,
    /// `owner/name` of the GitHub repository whose releases are used.
    #[serde(default = "default_update_repo")]
    pub repo: String,
    /// Hours between checks.
    #[serde(default = "default_update_interval")]
    pub check_interval_hours: u64,
    /// Release channel: `stable` (builds from `main`) or `bleeding-edge` (every commit to `dev`).
    #[serde(default = "default_update_channel")]
    pub channel: String,
}

/// Update channels, which are also the rolling release tags.
pub const UPDATE_CHANNELS: [&str; 2] = ["stable", "bleeding-edge"];

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check: true,
            auto_install: false,
            repo: default_update_repo(),
            check_interval_hours: default_update_interval(),
            channel: default_update_channel(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Stable id, used in heartbeats and the database. Letters, digits, `-` and `_`.
    pub id: String,
    pub game: GameKind,
    /// Display name. Falls back to the name the server reports.
    #[serde(default)]
    pub name: Option<String>,
    pub host: String,
    /// Port players connect to.
    pub game_port: u16,
    /// Steam query port, if different from the game's default for `game_port`.
    #[serde(default)]
    pub query_port: Option<u16>,
    /// Show `host:game_port` on the status card.
    #[serde(default = "yes")]
    pub show_address: bool,
    /// Show connected player names.
    #[serde(default = "yes")]
    pub show_players: bool,
    /// Post up/down alerts for this server.
    #[serde(default)]
    pub alerts: bool,
    /// Optional game-specific API for richer data than the Steam query.
    #[serde(default)]
    pub native: Option<NativeApi>,
    /// The server's install (or data) folder, if it is on this machine. The dashboard reads its logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<String>,
    /// Start/stop/restart commands, if the server runs on this machine (a service, a script).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<crate::control::LocalControl>,
    /// Name of the server's program (e.g. `VeinServer`), if it runs on this machine: its start
    /// time gives the real uptime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    /// A status file a game mod writes on this machine (Project Zomboid: `Zomboid/Lua/lighthouse_status.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_file: Option<String>,
    /// 7 Days to Die telnet console, for server controls (stop, messages, kicks, bans).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telnet: Option<Telnet>,
}

/// Login for 7 Days to Die's telnet console (`TelnetPort` and `TelnetPassword` in serverconfig.xml).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Telnet {
    pub port: u16,
    pub password: String,
}

impl ServerConfig {
    /// A server with only the essentials set; everything else at its default.
    pub fn new(id: impl Into<String>, game: GameKind, host: impl Into<String>, game_port: u16) -> Self {
        Self {
            id: id.into(),
            game,
            name: None,
            host: host.into(),
            game_port,
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
}

/// Game-specific interfaces Lighthouse can poll directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeApi {
    /// 7 Days to Die built-in web API (tokens made with the `webtokens` console command).
    SevenDaysWebApi {
        port: u16,
        token_name: String,
        token_secret: String,
    },
    /// Source RCON (Project Zomboid).
    Rcon { port: u16, password: String },
    /// VEIN `HTTPPort` API. It has no authentication: only use it on localhost or a private network.
    VeinHttp { port: u16 },
}

fn default_poll_interval() -> u64 {
    30
}
fn default_database() -> String {
    "lighthouse.db".into()
}
fn default_ingest_bind() -> String {
    "0.0.0.0:8651".into()
}
fn default_update_repo() -> String {
    DEFAULT_UPDATE_REPO.into()
}
fn default_update_channel() -> String {
    "stable".into()
}
fn default_update_interval() -> u64 {
    24
}
fn default_dashboard_bind() -> String {
    "127.0.0.1:8650".into()
}
fn yes() -> bool {
    true
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("config file is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config problems:\n  - {}", .0.join("\n  - "))]
    Invalid(Vec<String>),
    #[error("could not write config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let config = Self::parse(&text)?;
        Ok(config)
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// The database file, relative paths taken from the config file's folder.
    pub fn database_path(&self, config_path: &Path) -> std::path::PathBuf {
        let db = Path::new(&self.database);
        if db.is_absolute() {
            db.to_path_buf()
        } else {
            config_path.parent().unwrap_or(Path::new(".")).join(db)
        }
    }

    /// Validate, then write the config as TOML. Comments in an existing file are not kept.
    /// Games whose servers no bot shows (per-game mode with a bot still to add, or no bot at all).
    pub fn games_without_bot(&self) -> Vec<GameKind> {
        let mut games: Vec<GameKind> = self
            .servers
            .iter()
            .map(|s| s.game)
            .filter(|g| match self.mode {
                BotMode::Mega => self.bots.is_empty(),
                BotMode::PerGame => !self.bots.iter().any(|b| b.game == Some(*g)),
            })
            .collect();
        games.sort();
        games.dedup();
        games
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        let text = format!(
            "# Lighthouse configuration. Reference: lighthouse.example.toml\n\n{}",
            toml::to_string_pretty(self)?
        );
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Checks the rules serde cannot express. Reports every problem at once.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut problems = Vec::new();

        if self.poll_interval_secs < 10 {
            problems.push("poll_interval_secs must be at least 10".to_string());
        }

        let repo_ok = self
            .updates
            .repo
            .split_once('/')
            .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'));
        if !repo_ok {
            problems.push(format!(
                "updates.repo {:?} must look like \"owner/name\"",
                self.updates.repo
            ));
        }
        if !UPDATE_CHANNELS.contains(&self.updates.channel.as_str()) {
            problems.push(format!(
                "updates.channel {:?} must be one of: {}",
                self.updates.channel,
                UPDATE_CHANNELS.join(", ")
            ));
        }
        if self.updates.check_interval_hours == 0 {
            problems.push("updates.check_interval_hours must be at least 1".to_string());
        }
        if let Some(ingest) = &self.ingest
            && ingest.secret.trim().len() < 16
        {
            problems.push("ingest.secret must be at least 16 characters (`lighthouse setup` makes one)".to_string());
        }

        let mut ids = HashSet::new();
        for s in &self.servers {
            if s.id.is_empty() || !s.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                problems.push(format!(
                    "server id {:?} may only use letters, digits, '-' and '_'",
                    s.id
                ));
            }
            if !ids.insert(s.id.as_str()) {
                problems.push(format!("server id {:?} is used more than once", s.id));
            }
            if let Some(native) = &s.native {
                let fits = match native {
                    NativeApi::SevenDaysWebApi { .. } => s.game.is(GameKind::SEVEN_DAYS_TO_DIE),
                    NativeApi::Rcon { .. } => s.game.is(GameKind::PROJECT_ZOMBOID),
                    NativeApi::VeinHttp { .. } => s.game.is(GameKind::VEIN),
                };
                if !fits {
                    problems.push(format!(
                        "server {:?}: that native API does not belong to {}",
                        s.id, s.game
                    ));
                }
            }
        }

        for s in &self.servers {
            if s.telnet.is_some() && !s.game.is(GameKind::SEVEN_DAYS_TO_DIE) {
                problems.push(format!("server {:?}: telnet is only for 7 Days to Die", s.id));
            }
        }

        for (i, b) in self.bots.iter().enumerate() {
            if b.token.is_none() == b.token_env.is_none() {
                problems.push(format!("bot #{}: set exactly one of `token` or `token_env`", i + 1));
            }
            if b.status_channel_id == 0 {
                problems.push(format!("bot #{}: status_channel_id is not set", i + 1));
            }
        }

        match self.mode {
            BotMode::PerGame => {
                let mut seen = HashSet::new();
                for (i, b) in self.bots.iter().enumerate() {
                    match b.game {
                        None => problems.push(format!("bot #{}: `game` is required in per_game mode", i + 1)),
                        Some(g) if !seen.insert(g) => problems.push(format!("more than one bot for {g}")),
                        Some(_) => {}
                    }
                }
                // Games without a bot are allowed (bots can be added one at a time from the
                // dashboard); their servers are only polled. See `games_without_bot`.
            }
            BotMode::Mega => {
                // No bot yet is fine (polling only, e.g. until one is added on the dashboard).
                if self.bots.len() > 1 {
                    problems.push("mega mode takes one [[bots]] entry".to_string());
                }
                if self.bots.iter().any(|b| b.game.is_some()) {
                    problems.push("in mega mode the bot must not set `game`".to_string());
                }
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(problems))
        }
    }

    /// Servers a given bot is responsible for.
    pub fn servers_for<'a>(&'a self, bot: &'a BotConfig) -> impl Iterator<Item = &'a ServerConfig> + 'a {
        self.servers
            .iter()
            .filter(move |s| bot.game.is_none_or(|g| g == s.game))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../lighthouse.example.toml");

    #[test]
    fn example_config_is_valid() {
        let c = Config::parse(EXAMPLE).unwrap();
        assert_eq!(c.mode, BotMode::PerGame);
        assert!(!c.servers.is_empty());
    }

    #[test]
    fn default_matches_empty_file() {
        let empty = Config::parse("").unwrap();
        let d = Config::default();
        assert_eq!(
            (d.poll_interval_secs, &d.database, &d.updates, &d.dashboard),
            (
                empty.poll_interval_secs,
                &empty.database,
                &empty.updates,
                &empty.dashboard
            )
        );
    }

    #[test]
    fn dashboard_off_and_updates_default_repo() {
        let c = Config::parse("").unwrap();
        assert!(!c.dashboard.enabled);
        assert!(c.updates.check);
        assert!(!c.updates.auto_install);
        assert_eq!(c.updates.repo, DEFAULT_UPDATE_REPO);
    }

    #[test]
    fn round_trips_through_toml() {
        let c = Config::parse(EXAMPLE).unwrap();
        let text = toml::to_string_pretty(&c).unwrap();
        let back = Config::parse(&text).unwrap();
        assert_eq!(back.servers.len(), c.servers.len());
        assert_eq!(back.updates, c.updates);
    }

    #[test]
    fn games_without_a_bot_are_only_polled() {
        let text = r#"
            [[bots]]
            game = "pz"
            token = "x"
            status_channel_id = 1
            [[servers]]
            id = "a"
            game = "vein"
            host = "127.0.0.1"
            game_port = 7777
        "#;
        // Allowed (bots can be added one at a time), and reported.
        let c = Config::parse(text).unwrap();
        assert_eq!(c.games_without_bot(), [GameKind::get("vein")]);
        // One-bot mode with no bot yet: everything is only polled.
        let none = Config::parse(
            "mode = \"mega\"
[[servers]]
id = \"a\"
game = \"pz\"
host = \"127.0.0.1\"
game_port = 16261
",
        )
        .unwrap();
        assert_eq!(none.games_without_bot(), [GameKind::get("pz")]);
    }

    #[test]
    fn mega_mode_bot_covers_every_game() {
        let text = r#"
            mode = "mega"
            [[bots]]
            token_env = "LIGHTHOUSE_TOKEN"
            status_channel_id = 1
            [[servers]]
            id = "a"
            game = "pz"
            host = "127.0.0.1"
            game_port = 16261
            [[servers]]
            id = "b"
            game = "7dtd"
            host = "127.0.0.1"
            game_port = 26900
        "#;
        let c = Config::parse(text).unwrap();
        assert_eq!(c.servers_for(&c.bots[0]).count(), 2);
        assert!(c.games_without_bot().is_empty());
    }

    #[test]
    fn rejects_mismatched_native_api() {
        let text = r#"
            mode = "mega"
            [[bots]]
            token = "x"
            status_channel_id = 1
            [[servers]]
            id = "a"
            game = "vein"
            host = "127.0.0.1"
            game_port = 7777
            native = { kind = "rcon", port = 1, password = "p" }
        "#;
        assert!(Config::parse(text).is_err());
    }
}
