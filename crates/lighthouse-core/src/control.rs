//! Server controls: what can be asked of a game server, and how a configured server can be reached
//! for it.

use serde::{Deserialize, Serialize};

/// Something to do to a game server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ControlAction {
    /// Save and shut down.
    Stop,
    /// Save, shut down and start again. Through a game API or mod this is a shutdown that relies
    /// on the host (panel auto-restart, txAdmin, a service manager) to start the server again.
    Restart,
    /// Start a stopped server. Needs a start command on this machine.
    Start,
    /// Show a message to everyone on the server.
    Say { message: String },
    /// Remove a player from the server (by name or the game's player id), with a reason.
    Kick { player: String, reason: String },
    /// Ban a player. 7 Days to Die and Project Zomboid get the game's own ban command; FiveM and
    /// RedM get the Lighthouse ban list instead (see `Ban`), so this is only sent to the former.
    Ban {
        player: String,
        reason: String,
        /// `None` = permanent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_secs: Option<i64>,
    },
}

impl ControlAction {
    /// Short key, as used in URLs, the CLI and a mod's `accepts` list.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Start => "start",
            Self::Say { .. } => "say",
            Self::Kick { .. } => "kick",
            Self::Ban { .. } => "ban",
        }
    }

    /// Past tense, for the audit log and confirmations.
    pub fn done_label(&self) -> &'static str {
        match self {
            Self::Stop => "Stopped",
            Self::Restart => "Restarted",
            Self::Start => "Started",
            Self::Say { .. } => "Sent a message",
            Self::Kick { .. } => "Kicked a player",
            Self::Ban { .. } => "Banned a player",
        }
    }

    /// Parse the URL/CLI form. `say` needs `message`; `kick` needs `player` and takes `message` as
    /// the reason. (Bans go through the ban list, not this.)
    pub fn from_key(key: &str, message: Option<String>, player: Option<String>) -> Option<Self> {
        let clean = |s: Option<String>| s.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        Some(match key {
            "stop" => Self::Stop,
            "restart" => Self::Restart,
            "start" => Self::Start,
            "say" => Self::Say {
                message: clean(message)?,
            },
            "kick" => Self::Kick {
                player: clean(player)?,
                reason: clean(message).unwrap_or_else(|| "Kicked by an admin".to_string()),
            },
            _ => return None,
        })
    }
}

/// Commands that run on the machine Lighthouse runs on, for servers it can manage directly
/// (a service, a script, a container). Each is run through the system shell (`sh -c` on Linux,
/// `cmd /C` on Windows). Leave out the ones you don't want offered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<String>,
}

impl LocalControl {
    pub fn command_for(&self, action: &ControlAction) -> Option<&str> {
        match action {
            ControlAction::Start => self.start.as_deref(),
            ControlAction::Stop => self.stop.as_deref(),
            ControlAction::Restart => self.restart.as_deref(),
            ControlAction::Say { .. } | ControlAction::Kick { .. } | ControlAction::Ban { .. } => None,
        }
        .map(str::trim)
        .filter(|c| !c.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_json() {
        assert_eq!(ControlAction::from_key("stop", None, None), Some(ControlAction::Stop));
        assert_eq!(ControlAction::from_key("say", Some("  ".into()), None), None);
        assert_eq!(ControlAction::from_key("kick", None, None), None);
        let kick = ControlAction::from_key("kick", None, Some("Bob".into())).unwrap();
        assert_eq!(
            serde_json::to_string(&kick).unwrap(),
            r#"{"action":"kick","player":"Bob","reason":"Kicked by an admin"}"#
        );
        let say = ControlAction::from_key("say", Some(" hi ".into()), None).unwrap();
        assert_eq!(
            serde_json::to_string(&say).unwrap(),
            r#"{"action":"say","message":"hi"}"#
        );
        assert_eq!(
            serde_json::to_string(&ControlAction::Restart).unwrap(),
            r#"{"action":"restart"}"#
        );
        assert_eq!(ControlAction::from_key("format-disk", None, None), None);
    }

    #[test]
    fn local_commands() {
        let c = LocalControl {
            start: Some("systemctl start x".into()),
            stop: Some(" ".into()),
            restart: None,
        };
        assert_eq!(c.command_for(&ControlAction::Start), Some("systemctl start x"));
        assert_eq!(c.command_for(&ControlAction::Stop), None);
        assert_eq!(c.command_for(&ControlAction::Restart), None);
    }
}
