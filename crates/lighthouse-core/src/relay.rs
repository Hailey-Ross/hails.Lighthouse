//! In-memory meeting point between game mods (heartbeats), the poller, the dashboard and the
//! controls: the latest heartbeat per server, commands waiting for a mod to collect them, and
//! a short buffer of recent console lines per server.
//!
//! Nothing here is saved; after a restart mods simply send their next heartbeat.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::control::ControlAction;
use crate::heartbeat::Heartbeat;
use crate::status::ServerStatus;

/// Console lines kept per server.
pub const LOG_LINES: usize = 1000;
/// Longer lines are cut.
const MAX_LINE: usize = 2000;
/// A queued command that no mod collected within this many seconds is dropped, so a server
/// isn't stopped hours later when its mod reconnects.
pub const COMMAND_TTL: i64 = 120;

/// A command for a mod, as sent in the heartbeat reply: `{"id":3,"action":"say","message":"hi"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub id: u64,
    #[serde(flatten)]
    pub action: ControlAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
    /// Increases by one per line and server; the dashboard asks for lines after the last it has.
    pub seq: u64,
    pub at_unix: i64,
    pub text: String,
}

#[derive(Default)]
struct Entry {
    /// When the last heartbeat arrived, and what it said.
    heartbeat: Option<(i64, Heartbeat)>,
    pending: Vec<(i64, Command)>,
    log: VecDeque<LogLine>,
    next_seq: u64,
}

#[derive(Default)]
pub struct Relay {
    servers: Mutex<HashMap<String, Entry>>,
    next_command: AtomicU64,
}

impl Relay {
    fn with<T>(&self, server_id: &str, f: impl FnOnce(&mut Entry) -> T) -> T {
        let mut servers = self.servers.lock().unwrap_or_else(|p| p.into_inner());
        f(servers.entry(server_id.to_string()).or_default())
    }

    /// Store a heartbeat and hand back the commands waiting for this server (each is handed out
    /// once).
    pub fn heartbeat(&self, mut hb: Heartbeat, now_unix: i64) -> Vec<Command> {
        let lines = std::mem::take(&mut hb.log);
        let id = hb.server_id.clone();
        self.push_log(&id, now_unix, lines);
        self.with(&id, |e| {
            e.heartbeat = Some((now_unix, hb));
            let pending = std::mem::take(&mut e.pending);
            pending
                .into_iter()
                .filter(|(at, _)| now_unix - at <= COMMAND_TTL)
                .map(|(_, c)| c)
                .collect()
        })
    }

    /// The status from the last heartbeat, if it arrived within `max_age` seconds.
    pub fn fresh_status(&self, server_id: &str, now_unix: i64, max_age: i64) -> Option<ServerStatus> {
        self.with(server_id, |e| {
            e.heartbeat
                .as_ref()
                .filter(|(at, _)| now_unix - at <= max_age)
                .map(|(_, hb)| hb.status.clone())
        })
    }

    /// Actions the server's mod said it can carry out, if it is connected (heartbeat within
    /// `max_age` seconds).
    pub fn mod_accepts(&self, server_id: &str, now_unix: i64, max_age: i64) -> Vec<String> {
        self.with(server_id, |e| {
            e.heartbeat
                .as_ref()
                .filter(|(at, _)| now_unix - at <= max_age)
                .map(|(_, hb)| hb.accepts.clone())
                .unwrap_or_default()
        })
    }

    /// Queue a command for the server's mod; it goes out with the reply to its next heartbeat.
    pub fn queue(&self, server_id: &str, action: ControlAction, now_unix: i64) -> u64 {
        let id = self.next_command.fetch_add(1, Ordering::Relaxed) + 1;
        self.with(server_id, |e| e.pending.push((now_unix, Command { id, action })));
        id
    }

    /// Add console lines (from a mod or another source).
    pub fn push_log(&self, server_id: &str, now_unix: i64, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        self.with(server_id, |e| {
            for mut text in lines {
                if text.len() > MAX_LINE {
                    let mut cut = MAX_LINE;
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    text.truncate(cut);
                }
                e.next_seq += 1;
                e.log.push_back(LogLine {
                    seq: e.next_seq,
                    at_unix: now_unix,
                    text,
                });
                if e.log.len() > LOG_LINES {
                    e.log.pop_front();
                }
            }
        });
    }

    /// Console lines after `after_seq` (0 = everything kept).
    pub fn log_since(&self, server_id: &str, after_seq: u64) -> Vec<LogLine> {
        self.with(server_id, |e| {
            e.log.iter().filter(|l| l.seq > after_seq).cloned().collect()
        })
    }

    /// Whether any console lines were ever received for this server.
    pub fn has_log(&self, server_id: &str) -> bool {
        self.with(server_id, |e| e.next_seq > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameKind;
    use crate::status::ServerState;

    fn hb(id: &str, log: &[&str]) -> Heartbeat {
        Heartbeat {
            v: 1,
            server_id: id.into(),
            game: GameKind::get(GameKind::SEVEN_DAYS_TO_DIE),
            sent_at_unix: 0,
            status: ServerStatus::new(ServerState::Online, 0),
            accepts: vec!["stop".into(), "say".into()],
            log: log.iter().map(|s| s.to_string()).collect(),
            events: Vec::new(),
            bans_rev: None,
        }
    }

    #[test]
    fn commands_are_handed_out_once_and_expire() {
        let r = Relay::default();
        r.queue("a", ControlAction::Stop, 100);
        r.queue("a", ControlAction::Say { message: "hi".into() }, 100 - COMMAND_TTL - 1);
        let got = r.heartbeat(hb("a", &[]), 100);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].action, ControlAction::Stop);
        assert!(r.heartbeat(hb("a", &[]), 101).is_empty());
        assert_eq!(
            serde_json::to_string(&got[0]).unwrap(),
            format!(r#"{{"id":{},"action":"stop"}}"#, got[0].id)
        );
    }

    #[test]
    fn freshness_and_logs() {
        let r = Relay::default();
        r.heartbeat(hb("a", &["one", "two"]), 100);
        assert!(r.fresh_status("a", 130, 60).is_some());
        assert!(r.fresh_status("a", 200, 60).is_none());
        assert_eq!(r.mod_accepts("a", 130, 60), ["stop", "say"]);
        assert!(r.mod_accepts("b", 130, 60).is_empty());
        let lines = r.log_since("a", 0);
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(r.log_since("a", lines[0].seq).len(), 1);
        r.push_log("a", 1, (0..LOG_LINES + 5).map(|i| i.to_string()).collect());
        assert_eq!(r.log_since("a", 0).len(), LOG_LINES);
    }
}
