//! Polls every configured server and keeps the latest status of each.
//!
//! A server only counts as offline after two failed polls in a row, so one lost UDP packet doesn't
//! flip the board. Uptime without a mod is "online since Lighthouse first saw it come up", kept in
//! the state file so it survives a Lighthouse restart.
//!
//! Each poll also folds in what the server says about itself: a recent heartbeat from its mod
//! (see `Relay`), a status file its mod writes on this machine, and the start time of its process
//! here. Those win over the network query where both have a value, since the mod knows best (real
//! uptime, names); the query fills the gaps. A server whose mod is sending heartbeats counts as
//! online even if its query port is blocked, and one whose mod reports a shutdown goes offline at
//! once.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use lighthouse_core::{QueryError, Relay, ServerConfig, ServerState, ServerStatus};
use tokio::sync::{broadcast, watch};
use tracing::{debug, info, warn};

use crate::now_unix;
use crate::persist::Persist;

/// Failed polls in a row before a server is shown as offline.
const OFFLINE_AFTER: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    WentOffline,
    CameOnline,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub server_id: String,
    pub transition: Transition,
    pub at_unix: i64,
}

/// Called with every status sample (for history). Keep it quick.
pub type Recorder = Arc<dyn Fn(&str, &ServerStatus) + Send + Sync>;

pub struct Monitor {
    statuses: RwLock<HashMap<String, ServerStatus>>,
    generation: watch::Sender<u64>,
    transitions: broadcast::Sender<Change>,
    recorder: Option<Recorder>,
    relay: Arc<Relay>,
}

/// What one poll found, before the offline debounce.
enum Outcome {
    Up(Box<ServerStatus>),
    /// No answer (counts towards [`OFFLINE_AFTER`]).
    Down(String),
    /// The server's mod reported a shutdown: offline straight away.
    ShutDown,
}

/// Combine the network query with what the server reports about itself.
fn combine(query: Result<ServerStatus, QueryError>, own: Option<ServerStatus>, now: i64) -> Outcome {
    match (query, own) {
        (_, Some(own)) if own.state == ServerState::Offline => Outcome::ShutDown,
        // Only a process start time: it adds to a working query, but doesn't prove the server is up.
        (Ok(mut q), Some(own)) if own.state == ServerState::Unknown => {
            if q.started_at_unix.is_none() {
                q.started_at_unix = own.started_at_unix;
            }
            Outcome::Up(Box::new(q))
        }
        (Err(e), Some(own)) if own.state == ServerState::Unknown => Outcome::Down(e.to_string()),
        (Ok(q), Some(mut own)) => {
            own.fill_from(&q);
            own.ping_ms = q.ping_ms;
            own.observed_at_unix = now;
            Outcome::Up(Box::new(own))
        }
        (Err(_), Some(mut own)) => {
            own.observed_at_unix = now;
            Outcome::Up(Box::new(own))
        }
        (Ok(q), None) => Outcome::Up(Box::new(q)),
        (Err(e), None) => Outcome::Down(e.to_string()),
    }
}

impl Monitor {
    pub fn new(servers: &[ServerConfig], recorder: Option<Recorder>, relay: Arc<Relay>) -> Arc<Self> {
        let now = now_unix();
        let statuses = servers
            .iter()
            .map(|s| (s.id.clone(), ServerStatus::new(ServerState::Unknown, now)))
            .collect();
        Arc::new(Self {
            statuses: RwLock::new(statuses),
            generation: watch::channel(0).0,
            transitions: broadcast::channel(64).0,
            recorder,
            relay,
        })
    }

    /// Heartbeats, queued commands and console lines from game mods.
    pub fn relay(&self) -> &Arc<Relay> {
        &self.relay
    }

    /// What the server says about itself: its mod's heartbeat, else its status file; plus the
    /// start time of its process on this machine.
    async fn own_status(&self, server: &ServerConfig, now: i64, every: Duration) -> Option<ServerStatus> {
        // A heartbeat counts while no more than three polls (and at least 90 s) have passed.
        let max_age = (every.as_secs() as i64 * 3).max(90);
        let mut own = self.relay.fresh_status(&server.id, now, max_age);
        if own.is_none()
            && let Some(path) = server.status_file.clone()
        {
            own = tokio::task::spawn_blocking(move || {
                lighthouse_games::local::read_status_file(std::path::Path::new(&path), std::time::SystemTime::now())
            })
            .await
            .ok()
            .flatten();
        }
        if let Some(program) = server.process.clone() {
            let started = tokio::task::spawn_blocking(move || lighthouse_games::local::process_started_at(&program))
                .await
                .ok()
                .flatten();
            if let Some(started) = started {
                match &mut own {
                    Some(s) => {
                        s.started_at_unix.get_or_insert(started);
                    }
                    None => {
                        // Running here, but nothing else is known: `Unknown` marks a start-time-only hint.
                        let mut s = ServerStatus::new(ServerState::Unknown, now);
                        s.started_at_unix = Some(started);
                        own = Some(s);
                    }
                }
            }
        }
        own
    }

    pub fn status(&self, server_id: &str) -> Option<ServerStatus> {
        self.statuses
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(server_id)
            .cloned()
    }

    /// Bumped after every status update.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    pub fn transitions(&self) -> broadcast::Receiver<Change> {
        self.transitions.subscribe()
    }

    fn record(&self, server_id: &str, status: &ServerStatus) {
        if let Some(r) = &self.recorder {
            r(server_id, status);
        }
    }

    fn publish(&self, server_id: &str, status: ServerStatus) {
        self.record(server_id, &status);
        self.statuses
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(server_id.to_string(), status);
        self.generation.send_modify(|g| *g += 1);
    }

    fn transition(&self, server_id: &str, transition: Transition, at_unix: i64) {
        info!(server = server_id, ?transition, "server state changed");
        // No receivers is fine (alerts may be off everywhere).
        let _ = self.transitions.send(Change {
            server_id: server_id.to_string(),
            transition,
            at_unix,
        });
    }

    /// Start one polling task per server.
    pub fn spawn_pollers(self: &Arc<Self>, servers: Vec<ServerConfig>, every: Duration, persist: Arc<Persist>) {
        for (i, server) in servers.into_iter().enumerate() {
            let monitor = Arc::clone(self);
            let persist = Arc::clone(&persist);
            // Spread the first polls out a little so they don't all fire at once.
            let delay = Duration::from_millis(250 * i as u64);
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                monitor.poll_loop(server, every, persist).await;
            });
        }
    }

    async fn poll_loop(&self, server: ServerConfig, every: Duration, persist: Arc<Persist>) {
        let adapter = lighthouse_games::adapter_for(server.game);
        let mut failures = 0u32;
        let mut shown = ServerState::Unknown;
        let mut online_since = persist.online_since(&server.id);
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            let now = now_unix();
            let query = adapter.query(&server, now).await;
            let own = self.own_status(&server, now, every).await;
            let outcome = match combine(query, own, now) {
                Outcome::ShutDown => {
                    failures = OFFLINE_AFTER;
                    Outcome::ShutDown
                }
                other => other,
            };
            match outcome {
                Outcome::Up(status) => {
                    let mut status = *status;
                    failures = 0;
                    if shown != ServerState::Online {
                        // Came up while we watched (or first sighting with nothing remembered).
                        if shown == ServerState::Offline || online_since.is_none() {
                            online_since = Some(status.started_at_unix.unwrap_or(now));
                            persist.set_online_since(&server.id, online_since);
                        }
                        if shown == ServerState::Offline {
                            self.transition(&server.id, Transition::CameOnline, now);
                        }
                    }
                    if status.started_at_unix.is_none() {
                        status.started_at_unix = online_since;
                    }
                    shown = status.state;
                    self.publish(&server.id, status);
                }
                Outcome::Down(_) | Outcome::ShutDown => {
                    let e = match &outcome {
                        Outcome::Down(e) => e.clone(),
                        _ => "its mod reported a shutdown".to_string(),
                    };
                    failures = failures.saturating_add(1);
                    debug!(server = %server.id, failures, "poll failed: {e}");
                    if failures >= OFFLINE_AFTER && shown != ServerState::Offline {
                        if shown == ServerState::Online {
                            self.transition(&server.id, Transition::WentOffline, now);
                        } else {
                            warn!(server = %server.id, "not answering: {e}");
                        }
                        shown = ServerState::Offline;
                        online_since = None;
                        persist.set_online_since(&server.id, None);
                        self.publish(&server.id, ServerStatus::offline(now));
                    } else if shown == ServerState::Offline {
                        // Still down: keep the history complete for uptime figures.
                        self.record(&server.id, &ServerStatus::offline(now));
                    }
                }
            }
        }
    }
}
