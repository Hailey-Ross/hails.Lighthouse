//! The heartbeat: what game mods and agents push to Lighthouse. See `docs/heartbeat.md`.

use serde::{Deserialize, Serialize};

use crate::game::GameKind;
use crate::relay::Command;
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
    /// Control actions the mod can carry out: any of `stop`, `restart`, `say`.
    #[serde(default)]
    pub accepts: Vec<String>,
    /// New console lines since the previous heartbeat, oldest first.
    #[serde(default)]
    pub log: Vec<String>,
    /// Things that happened since the previous heartbeat (bans, kicks, restarts...), oldest first.
    #[serde(default)]
    pub events: Vec<ServerEvent>,
    /// For mods that enforce Lighthouse bans: the revision of the ban list they hold. When it
    /// differs from Lighthouse's, the reply carries the full list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bans_rev: Option<String>,
}

/// Something that happened on a game server, as reported by its mod (or done from Lighthouse).
///
/// `kind` is one of: `ban`, `warn`, `kick`, `revoke` (a ban or warn lifted), `announcement`,
/// `restart_scheduled`, `restart_skipped`, `shutdown`, `start`, or `control` (an action taken from
/// Lighthouse). Unknown kinds are kept and shown as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEvent {
    pub kind: String,
    /// When it happened (Unix seconds). Lighthouse uses the arrival time when missing.
    #[serde(default)]
    pub at_unix: Option<i64>,
    /// The player it concerns, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    /// Who did it (admin name, `txAdmin`, a dashboard user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Reason or message text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The game's or tool's own id for it (txAdmin action id), so a later `revoke` can be matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    /// When a ban runs out; `None` on a ban means permanent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_unix: Option<i64>,
    /// The player's identifiers (`license:...`, `discord:...`, Steam id), if the mod sends them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
}

impl ServerEvent {
    /// Moderation events are kept longer than the rest.
    pub fn is_moderation(kind: &str) -> bool {
        matches!(kind, "ban" | "warn" | "kick" | "revoke")
    }
}

/// A ban issued from Lighthouse, as sent to mods that enforce them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ban {
    /// Lighthouse's ban id.
    pub id: i64,
    /// Player name when banned (for messages; matching uses `ids`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    /// Identifiers to refuse (`license:...`, `discord:...`, `steam:...`, `ip:...`).
    pub ids: Vec<String>,
    pub reason: String,
    pub author: String,
    pub created_unix: i64,
    /// `None` = permanent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_unix: Option<i64>,
}

/// Lighthouse's answer to a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatReply {
    pub v: u32,
    /// Commands for the mod to carry out, oldest first. Each is sent once.
    pub commands: Vec<Command>,
    /// The full list of active Lighthouse bans for this server, when the mod's `bans_rev` was out
    /// of date; replace the held list with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bans: Option<Vec<Ban>>,
    /// Revision of `bans`, to send back as `bans_rev`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bans_rev: Option<String>,
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
        assert!(hb.accepts.is_empty() && hb.log.is_empty() && hb.events.is_empty());
    }
}
