//! FiveM and RedM (Cfx.re) servers, over HTTP on the game port:
//!
//! - `dynamic.json`: host name, player count and limit, game type and map. Always answered.
//! - `info.json`: project name and description, tags, server version. Usually answered.
//! - `players.json`: player names and pings. Often hidden by the server owner; then only the count.

use std::time::Instant;

use lighthouse_core::{
    Capabilities, GameAdapter, GameKind, Player, QueryError, ServerConfig, ServerState, ServerStatus,
};
use serde_json::Value;

use crate::http;

/// Remove Cfx colour codes: `^0`–`^9` and the style codes `^*`, `^_`, `^~`, `^=`, `^r`.
pub(crate) fn strip_colours(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '^' && chars.peek().is_some_and(|n| n.is_ascii_digit() || "*_~=r".contains(*n)) {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Numbers in `dynamic.json` are sometimes strings (`"sv_maxclients": "48"`).
fn number(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Number(n) => n.as_u64().map(|n| n as u32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(strip_colours).filter(|s| !s.is_empty())
}

/// `FXServer-master SERVER v1.0.0.7290 win32` -> `v1.0.0.7290`
fn server_version(server: &str) -> Option<String> {
    server
        .split_whitespace()
        .find(|w| w.starts_with('v') && w[1..].starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

fn fivem_status(dynamic: &Value, info: Option<&Value>, players: Option<&Value>, now_unix: i64) -> ServerStatus {
    let mut s = ServerStatus::new(ServerState::Online, now_unix);
    let vars = info.and_then(|i| i.get("vars"));
    // The project name is the short name owners pick; the host name is often a long banner.
    s.name = text(vars.and_then(|v| v.get("sv_projectName"))).or_else(|| text(dynamic.get("hostname")));
    s.description = text(vars.and_then(|v| v.get("sv_projectDesc")));
    s.mode = text(dynamic.get("gametype"));
    s.map = text(dynamic.get("mapname"));
    s.version = info
        .and_then(|i| i.get("server"))
        .and_then(Value::as_str)
        .and_then(server_version);
    s.players_online = number(dynamic.get("clients"));
    s.players_max = number(dynamic.get("sv_maxclients")).or_else(|| number(vars.and_then(|v| v.get("sv_maxClients"))));
    if let Some(list) = players.and_then(Value::as_array) {
        s.players = list
            .iter()
            .filter_map(|p| {
                let name = strip_colours(p.get("name")?.as_str()?);
                let ping_ms = number(p.get("ping"));
                (!name.is_empty()).then_some(Player { name, ping_ms })
            })
            .collect();
        if s.players_online.is_none() {
            s.players_online = Some(list.len() as u32);
        }
    }
    s
}

async fn get_json(url: String) -> Option<Value> {
    let resp = http::client().get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
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
        let started = Instant::now();
        let dynamic: Value = http::client()
            .get(format!("{base}/dynamic.json"))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| QueryError::Unreachable(e.to_string()))?
            .json()
            .await
            .map_err(|e| QueryError::BadResponse(e.to_string()))?;
        let ping = started.elapsed();
        // The other two are extras: a server that hides them is still online.
        let (info, players) = tokio::join!(
            get_json(format!("{base}/info.json")),
            get_json(format!("{base}/players.json"))
        );
        let mut status = fivem_status(&dynamic, info.as_ref(), players.as_ref(), now_unix);
        status.ping_ms = Some(ping.as_millis().min(u128::from(u32::MAX)) as u32);
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dynamic() -> Value {
        serde_json::from_str(
            r#"{"clients":2,"gametype":"Roleplay","hostname":"^1Red ^7City ^*RP | Whitelisted | Discord: x","mapname":"Los Santos","sv_maxclients":"48"}"#,
        )
        .unwrap()
    }

    #[test]
    fn colour_codes_and_versions() {
        assert_eq!(strip_colours("^1Red ^7City  ^*RP"), "Red City RP");
        assert_eq!(
            server_version("FXServer-master SERVER v1.0.0.7290 win32").as_deref(),
            Some("v1.0.0.7290")
        );
        assert_eq!(server_version("something else"), None);
    }

    #[test]
    fn uses_info_and_players() {
        let info: Value = serde_json::from_str(
            r#"{"server":"FXServer-master SERVER v1.0.0.7290 linux","vars":{"sv_projectName":"^2Red City","sv_projectDesc":"Serious roleplay since 2020","tags":"rp,economy"}}"#,
        )
        .unwrap();
        let players: Value = serde_json::from_str(r#"[{"name":"Alice","ping":40},{"name":"","ping":1}]"#).unwrap();
        let s = fivem_status(&dynamic(), Some(&info), Some(&players), 0);
        assert_eq!(s.name.as_deref(), Some("Red City"));
        assert_eq!(s.description.as_deref(), Some("Serious roleplay since 2020"));
        assert_eq!(s.mode.as_deref(), Some("Roleplay"));
        assert_eq!(s.version.as_deref(), Some("v1.0.0.7290"));
        assert_eq!((s.players_online, s.players_max), (Some(2), Some(48)));
        assert_eq!(s.players.len(), 1);
        assert_eq!(s.players[0].ping_ms, Some(40));
    }

    #[test]
    fn hidden_info_and_players_still_work() {
        let s = fivem_status(&dynamic(), None, None, 0);
        assert_eq!(s.name.as_deref(), Some("Red City RP | Whitelisted | Discord: x"));
        assert_eq!(s.players_online, Some(2));
        assert!(s.players.is_empty());
    }
}
