//! The heartbeat: what game mods and agents push to Lighthouse. See `docs/heartbeat.md`.

use serde::{Deserialize, Serialize};

use crate::game::GameKind;
use crate::status::ServerStatus;

/// Current heartbeat format version. Bump on breaking changes only.
pub const HEARTBEAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Format version, see [`HEARTBEAT_VERSION`].
    pub v: u32,
    /// The `id` of the server in the Lighthouse config.
    pub server_id: String,
    pub game: GameKind,
    pub sent_at_unix: i64,
    pub status: ServerStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_heartbeat() {
        let json = r#"{
            "v": 1,
            "server_id": "pve-main",
            "game": "7dtd",
            "sent_at_unix": 1700000000,
            "status": { "state": "online", "players_online": 2, "observed_at_unix": 1700000000 }
        }"#;
        let hb: Heartbeat = serde_json::from_str(json).unwrap();
        assert!(hb.game.is(GameKind::SEVEN_DAYS_TO_DIE));
        assert_eq!(hb.status.players_online, Some(2));
        assert!(hb.status.players.is_empty());
    }
}
