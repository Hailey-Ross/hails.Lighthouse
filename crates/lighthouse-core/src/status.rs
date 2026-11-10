//! The normalised status of one game server, whatever source it came from.

use serde::{Deserialize, Serialize};

/// Whether a server is reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    Online,
    /// The process is up but not accepting players yet (reported by mods/agents only).
    Starting,
    Offline,
    /// Never queried yet.
    Unknown,
}

/// A connected player.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_ms: Option<u32>,
}

/// In-game clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameTime {
    /// In-game day, or 0 when the game only reports the time of day.
    pub day: u32,
    pub hour: u8,
    pub minute: u8,
}

/// Status of one game server. Every field except `state` and `observed_at_unix` is optional
/// because each source (Steam query, native API, mod) knows a different subset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub state: ServerState,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub players_online: Option<u32>,
    #[serde(default)]
    pub players_max: Option<u32>,
    #[serde(default)]
    pub players: Vec<Player>,
    /// Unix time (seconds) the server process last started. Uptime is measured from here.
    #[serde(default)]
    pub started_at_unix: Option<i64>,
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub game_time: Option<GameTime>,
    /// 7 Days to Die only: days until the next blood moon (0 = tonight).
    #[serde(default)]
    pub blood_moon_in_days: Option<u32>,
    #[serde(default)]
    pub password_protected: Option<bool>,
    /// Round-trip time of the query that produced this status.
    #[serde(default)]
    pub ping_ms: Option<u32>,
    /// A short description the server advertises (FiveM project description, Minecraft MOTD).
    #[serde(default)]
    pub description: Option<String>,
    /// Game mode or type (FiveM gametype, Minecraft Bedrock game mode).
    #[serde(default)]
    pub mode: Option<String>,
    /// Unix time (seconds) of the last wipe, for games that report it (Rust).
    #[serde(default)]
    pub wiped_at_unix: Option<i64>,
    /// Unix time (seconds) this status was observed.
    pub observed_at_unix: i64,
}

impl GameTime {
    /// `Day 14, 21:05`, or just `21:05` when the day isn't known.
    pub fn label(&self) -> String {
        if self.day == 0 {
            format!("{:02}:{:02}", self.hour, self.minute)
        } else {
            format!("Day {}, {:02}:{:02}", self.day, self.hour, self.minute)
        }
    }
}

impl ServerStatus {
    /// An empty status in the given state.
    pub fn new(state: ServerState, observed_at_unix: i64) -> Self {
        Self {
            state,
            name: None,
            players_online: None,
            players_max: None,
            players: Vec::new(),
            started_at_unix: None,
            map: None,
            version: None,
            game_time: None,
            blood_moon_in_days: None,
            password_protected: None,
            ping_ms: None,
            description: None,
            mode: None,
            wiped_at_unix: None,
            observed_at_unix,
        }
    }

    pub fn offline(observed_at_unix: i64) -> Self {
        Self::new(ServerState::Offline, observed_at_unix)
    }

    /// Seconds since the last restart, if known and the server is up.
    pub fn uptime_secs(&self, now_unix: i64) -> Option<u64> {
        match self.state {
            ServerState::Online | ServerState::Starting => self
                .started_at_unix
                .map(|start| now_unix.saturating_sub(start).max(0) as u64),
            _ => None,
        }
    }

    /// Fill gaps in `self` with values from `other` (a lower-priority source).
    pub fn fill_from(&mut self, other: &ServerStatus) {
        fn fill<T: Clone>(a: &mut Option<T>, b: &Option<T>) {
            if a.is_none() {
                a.clone_from(b);
            }
        }
        fill(&mut self.name, &other.name);
        fill(&mut self.players_online, &other.players_online);
        fill(&mut self.players_max, &other.players_max);
        fill(&mut self.started_at_unix, &other.started_at_unix);
        fill(&mut self.map, &other.map);
        fill(&mut self.version, &other.version);
        fill(&mut self.game_time, &other.game_time);
        fill(&mut self.blood_moon_in_days, &other.blood_moon_in_days);
        fill(&mut self.password_protected, &other.password_protected);
        fill(&mut self.ping_ms, &other.ping_ms);
        fill(&mut self.description, &other.description);
        fill(&mut self.mode, &other.mode);
        fill(&mut self.wiped_at_unix, &other.wiped_at_unix);
        if self.players.is_empty() {
            self.players.clone_from(&other.players);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_only_when_up() {
        let mut s = ServerStatus::new(ServerState::Online, 1_000);
        s.started_at_unix = Some(400);
        assert_eq!(s.uptime_secs(1_000), Some(600));
        s.state = ServerState::Offline;
        assert_eq!(s.uptime_secs(1_000), None);
    }

    #[test]
    fn fill_keeps_higher_priority_values() {
        let mut a = ServerStatus::new(ServerState::Online, 0);
        a.players_online = Some(3);
        let mut b = ServerStatus::new(ServerState::Online, 0);
        b.players_online = Some(5);
        b.players_max = Some(10);
        a.fill_from(&b);
        assert_eq!(a.players_online, Some(3));
        assert_eq!(a.players_max, Some(10));
    }
}
