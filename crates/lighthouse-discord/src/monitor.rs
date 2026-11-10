//! Polls every configured server and keeps the latest status of each.
//!
//! A server only counts as offline after two failed polls in a row, so one lost UDP packet doesn't
//! flip the board. Uptime without a mod is "online since Lighthouse first saw it come up", kept in
//! the state file so it survives a Lighthouse restart.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use lighthouse_core::{ServerConfig, ServerState, ServerStatus};
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
}

impl Monitor {
    pub fn new(servers: &[ServerConfig], recorder: Option<Recorder>) -> Arc<Self> {
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
        })
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
            match adapter.query(&server, now).await {
                Ok(mut status) => {
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
                Err(e) => {
                    failures += 1;
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
