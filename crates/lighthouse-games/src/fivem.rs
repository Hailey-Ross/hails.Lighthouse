//! FiveM / RedM (Cfx.re) servers: `dynamic.json` and `players.json` on the game port over HTTP.

use std::time::{Duration, Instant};

use lighthouse_core::{
    Capabilities, GameAdapter, GameKind, Player, QueryError, ServerConfig, ServerState, ServerStatus,
};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(5);

/// Remove `^0`–`^9` colour codes.
fn strip_colours(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '^' && chars.peek().is_some_and(char::is_ascii_digit) {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Numbers in `dynamic.json` are sometimes strings (`"sv_maxclients": "48"`).
fn number(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Number(n) => n.as_u64().map(|n| n as u32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn fivem_status(dynamic: &Value, players: Option<&Value>, now_unix: i64) -> ServerStatus {
    let mut s = ServerStatus::new(ServerState::Online, now_unix);
    s.name = dynamic
        .get("hostname")
        .and_then(Value::as_str)
        .map(strip_colours)
        .filter(|n| !n.is_empty());
    s.map = dynamic
        .get("mapname")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|m| !m.is_empty());
    s.players_online = number(dynamic.get("clients"));
    s.players_max = number(dynamic.get("sv_maxclients"));
    if let Some(list) = players.and_then(Value::as_array) {
        s.players = list
            .iter()
            .filter_map(|p| {
                let name = strip_colours(p.get("name")?.as_str()?);
                let ping_ms = number(p.get("ping"));
                (!name.is_empty()).then_some(Player { name, ping_ms })
            })
            .collect();
    }
    s
}

pub struct FiveM(pub GameKind);

#[async_trait::async_trait]
impl GameAdapter for FiveM {
    fn kind(&self) -> GameKind {
        self.0
    }

    fn capabilities(&self, _server: &ServerConfig) -> Capabilities {
        Capabilities {
            player_names: true,
            ..Capabilities::default()
        }
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let port = server
            .query_port
            .unwrap_or(server.game.query_port_for(server.game_port));
        let base = format!("http://{}:{port}", server.host);
        let http = reqwest::Client::builder()
            .timeout(WAIT)
            .user_agent(concat!("Lighthouse/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| QueryError::Unreachable(e.to_string()))?;
        let started = Instant::now();
        let dynamic: Value = http
            .get(format!("{base}/dynamic.json"))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| QueryError::Unreachable(e.to_string()))?
            .json()
            .await
            .map_err(|e| QueryError::BadResponse(e.to_string()))?;
        let ping = started.elapsed();
        // Some servers hide the player list; that's fine.
        let players: Option<Value> = match http.get(format!("{base}/players.json")).send().await {
            Ok(r) if r.status().is_success() => r.json().await.ok(),
            _ => None,
        };
        let mut status = fivem_status(&dynamic, players.as_ref(), now_unix);
        status.ping_ms = Some(ping.as_millis().min(u128::from(u32::MAX)) as u32);
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dynamic_and_players() {
        let dynamic: Value = serde_json::from_str(
            r#"{"clients":2,"gametype":"Roleplay","hostname":"^1Red ^7City RP","mapname":"Los Santos","sv_maxclients":"48"}"#,
        )
        .unwrap();
        let players: Value = serde_json::from_str(r#"[{"name":"Alice","ping":40},{"name":"","ping":1}]"#).unwrap();
        let s = fivem_status(&dynamic, Some(&players), 0);
        assert_eq!(s.name.as_deref(), Some("Red City RP"));
        assert_eq!((s.players_online, s.players_max), (Some(2), Some(48)));
        assert_eq!(s.players.len(), 1);
        assert_eq!(s.players[0].ping_ms, Some(40));
    }
}
