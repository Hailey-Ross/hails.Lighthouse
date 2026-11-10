//! Games and the adapter trait.
//!
//! Every game Lighthouse knows is a row in the catalogue (`catalog.rs`). Adding a game that speaks
//! an already supported protocol is one new row. A game that needs special handling also gets its
//! own adapter in `lighthouse-games` (registered in `lighthouse_games::adapter_for`).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::catalog::GAMES;
use crate::config::ServerConfig;
use crate::status::ServerStatus;

/// How a game's server status is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Steam server query (A2S) over UDP.
    Steam,
    /// Minecraft Java Edition server list ping over TCP.
    MinecraftJava,
    /// Minecraft Bedrock Edition (RakNet unconnected ping) over UDP.
    MinecraftBedrock,
    /// FiveM / RedM `dynamic.json` and `players.json` over HTTP.
    FiveM,
}

/// Where the status query goes, relative to the game port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryPort {
    /// The game port itself.
    Same,
    /// A fixed port, whatever the game port is.
    Fixed(u16),
    /// Game port plus an offset.
    Offset(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Support {
    /// Own adapter, checked against live servers.
    Full,
    /// Generic adapter for its protocol; untested defaults.
    Experimental,
}

/// One catalogue row.
#[derive(Debug)]
pub struct GameProfile {
    /// Stable id used in config files and heartbeats.
    pub id: &'static str,
    /// Other accepted spellings of the id.
    pub aliases: &'static [&'static str],
    pub name: &'static str,
    /// Upper-case tag for environment variable names.
    pub short: &'static str,
    pub protocol: Protocol,
    /// Default port players connect to.
    pub game_port: u16,
    pub query_port: QueryPort,
    pub support: Support,
}

/// A game from the catalogue. Cheap to copy; compares by id.
#[derive(Clone, Copy)]
pub struct GameKind(&'static GameProfile);

impl GameKind {
    pub const SEVEN_DAYS_TO_DIE: &'static str = "seven_days_to_die";
    pub const PROJECT_ZOMBOID: &'static str = "project_zomboid";
    pub const VEIN: &'static str = "vein";

    /// Look a game up by id or alias, ignoring case and `-`/`_`/space differences.
    pub fn from_id(id: &str) -> Option<GameKind> {
        let want = normalise(id);
        GAMES
            .iter()
            .find(|g| normalise(g.id) == want || g.aliases.iter().any(|a| normalise(a) == want))
            .map(GameKind)
    }

    /// A catalogue game by its exact id. Panics if the id is missing (catalogue bug).
    pub fn get(id: &str) -> GameKind {
        Self::from_id(id).unwrap_or_else(|| panic!("game {id:?} missing from the catalogue"))
    }

    /// All games: fully supported first, then by name.
    pub fn all() -> Vec<GameKind> {
        let mut games: Vec<GameKind> = GAMES.iter().map(GameKind).collect();
        games.sort();
        games
    }

    /// Games whose name or id contains `text` (case-insensitive).
    pub fn search(text: &str) -> Vec<GameKind> {
        let want = normalise(text);
        Self::all()
            .into_iter()
            .filter(|g| {
                normalise(g.0.name).contains(&want)
                    || normalise(g.0.id).contains(&want)
                    || g.0.aliases.iter().any(|a| normalise(a).contains(&want))
            })
            .collect()
    }

    pub fn profile(self) -> &'static GameProfile {
        self.0
    }
    pub fn id(self) -> &'static str {
        self.0.id
    }
    pub fn is(self, id: &str) -> bool {
        self.0.id == id
    }
    pub fn display_name(self) -> &'static str {
        self.0.name
    }
    pub fn short_name(self) -> &'static str {
        self.0.short
    }
    pub fn protocol(self) -> Protocol {
        self.0.protocol
    }
    pub fn is_experimental(self) -> bool {
        self.0.support == Support::Experimental
    }
    pub fn default_game_port(self) -> u16 {
        self.0.game_port
    }
    /// Query port for a server whose game port is `game_port`.
    pub fn query_port_for(self, game_port: u16) -> u16 {
        match self.0.query_port {
            QueryPort::Same => game_port,
            QueryPort::Fixed(p) => p,
            QueryPort::Offset(o) => u16::try_from(i32::from(game_port) + o).unwrap_or(game_port),
        }
    }
    /// Query port for the default game port.
    pub fn default_query_port(self) -> u16 {
        self.query_port_for(self.0.game_port)
    }
}

fn normalise(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl PartialEq for GameKind {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}
impl Eq for GameKind {}
impl Hash for GameKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}
impl Ord for GameKind {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0.support, self.0.name.to_lowercase(), self.0.id).cmp(&(
            other.0.support,
            other.0.name.to_lowercase(),
            other.0.id,
        ))
    }
}
impl PartialOrd for GameKind {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for GameKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name)
    }
}
impl fmt::Debug for GameKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GameKind({})", self.0.id)
    }
}

impl Serialize for GameKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.id)
    }
}
impl<'de> Deserialize<'de> for GameKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let id = String::deserialize(d)?;
        GameKind::from_id(&id).ok_or_else(|| {
            let hint = GameKind::search(&id)
                .into_iter()
                .take(3)
                .map(|g| g.id())
                .collect::<Vec<_>>();
            let hint = if hint.is_empty() {
                String::new()
            } else {
                format!(" (did you mean {})", hint.join(", "))
            };
            serde::de::Error::custom(format!("unknown game {id:?}{hint}; `lighthouse games` lists them all"))
        })
    }
}

/// Which status fields an adapter can provide, so the board can hide what will never be known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub player_names: bool,
    pub uptime: bool,
    pub game_time: bool,
    pub blood_moon: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("server did not answer: {0}")]
    Unreachable(String),
    #[error("unexpected response: {0}")]
    BadResponse(String),
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

/// Polls one game's servers. Push data (mods/agents) arrives separately as heartbeats and is
/// merged on top of what the adapter returns.
#[async_trait::async_trait]
pub trait GameAdapter: Send + Sync {
    fn kind(&self) -> GameKind;

    fn capabilities(&self, server: &ServerConfig) -> Capabilities;

    /// Query the server once. Network failures are `Err`; deciding "offline" is up to the caller.
    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_ids_resolve_and_are_unique() {
        // No lookup key (id or alias, after normalising) may point at two different games.
        let mut owner = std::collections::HashMap::new();
        for g in GAMES {
            for key in std::iter::once(&g.id).chain(g.aliases.iter()) {
                let prev = owner.insert(normalise(key), g.id);
                assert!(
                    prev.is_none_or(|p| p == g.id),
                    "{key:?} is used by {} and {}",
                    prev.unwrap(),
                    g.id
                );
            }
            assert_eq!(GameKind::from_id(g.id).map(|k| k.id()), Some(g.id));
        }
        assert!(GAMES.len() >= 100);
    }

    #[test]
    fn aliases_and_ports() {
        let pz = GameKind::from_id("PZ").unwrap();
        assert!(pz.is(GameKind::PROJECT_ZOMBOID));
        // Build 42 answers the Steam query on its game port (checked on a 42.22 server).
        assert_eq!(pz.query_port_for(16261), 16261);
        let vein = GameKind::get(GameKind::VEIN);
        assert_eq!(vein.query_port_for(27018), 27019);
        assert_eq!(GameKind::from_id("7dtd").unwrap().query_port_for(26930), 26930);
        assert!(GameKind::from_id("no such game").is_none());
    }

    #[test]
    fn full_games_sort_first_and_search_works() {
        let all = GameKind::all();
        assert!(!all[0].is_experimental());
        assert!(GameKind::search("minecraft").len() >= 2);
    }

    #[test]
    fn serde_round_trip() {
        let g: GameKind = serde_json::from_str("\"7dtd\"").unwrap();
        assert_eq!(serde_json::to_string(&g).unwrap(), "\"seven_days_to_die\"");
        assert!(serde_json::from_str::<GameKind>("\"nope\"").is_err());
    }
}
