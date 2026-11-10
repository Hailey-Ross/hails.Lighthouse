//! Steam query shared by every adapter: query once, then map the common fields.

use std::time::Duration;

use lighthouse_core::{Player, QueryError, ServerConfig, ServerState, ServerStatus};

use crate::a2s::{A2sClient, A2sError, A2sPlayer, Info, Rules};

/// Per-request timeout. A server that doesn't answer INFO within this counts as unreachable.
const WAIT: Duration = Duration::from_secs(3);

pub(crate) struct SteamData {
    pub info: Info,
    pub ping: Duration,
    pub players: Vec<A2sPlayer>,
    pub rules: Rules,
}

fn to_query_error(e: A2sError) -> QueryError {
    match e {
        A2sError::Io(_) | A2sError::Timeout(_) | A2sError::Resolve(_) => QueryError::Unreachable(e.to_string()),
        A2sError::Malformed(_) | A2sError::Compressed => QueryError::BadResponse(e.to_string()),
    }
}

/// INFO is required; PLAYER and RULES are best effort (some servers don't answer them).
pub(crate) async fn query(server: &ServerConfig) -> Result<SteamData, QueryError> {
    let port = server
        .query_port
        .unwrap_or(server.game.query_port_for(server.game_port));
    let client = A2sClient::connect(&server.host, port, WAIT)
        .await
        .map_err(to_query_error)?;
    let (info, ping) = client.info().await.map_err(to_query_error)?;
    let players = client.players().await.unwrap_or_default();
    let rules = client.rules().await.unwrap_or_default();
    Ok(SteamData {
        info,
        ping,
        players,
        rules,
    })
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Many games put a placeholder (`1.0.0.0`, `01.03.40`) in the INFO version and the real one in
/// the rules: 7DTD `ServerVersion`, VEIN `Version`, PZ `version`.
const VERSION_RULES: [&str; 3] = ["ServerVersion", "Version", "version"];

/// Fields every game reports the same way.
pub(crate) fn base_status(data: &SteamData, now_unix: i64) -> ServerStatus {
    let info = &data.info;
    let mut s = ServerStatus::new(ServerState::Online, now_unix);
    s.name = non_empty(&info.name);
    s.map = non_empty(&info.map);
    s.version = VERSION_RULES
        .iter()
        .find_map(|k| data.rules.get(*k).and_then(|v| non_empty(v)))
        .or_else(|| non_empty(&info.version));
    s.players_online = Some(u32::from(info.players.saturating_sub(info.bots)));
    s.players_max = Some(u32::from(info.max_players));
    s.password_protected = Some(info.password);
    s.ping_ms = Some(u32::try_from(data.ping.as_millis()).unwrap_or(u32::MAX));
    s.players = data
        .players
        .iter()
        .filter_map(|p| non_empty(&p.name).map(|name| Player { name, ping_ms: None }))
        .collect();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_fields_and_skips_blank_players() {
        let data = SteamData {
            info: Info {
                name: " PvE ".into(),
                map: "".into(),
                folder: "x".into(),
                game: "x".into(),
                app_id: 0,
                players: 3,
                max_players: 10,
                bots: 1,
                server_type: 'd',
                environment: 'l',
                password: true,
                vac: false,
                version: "1.0".into(),
                port: None,
                steam_id: None,
                keywords: None,
                game_id: None,
            },
            ping: Duration::from_millis(42),
            players: vec![
                A2sPlayer {
                    name: "Alice".into(),
                    score: 0,
                    duration: 1.0,
                },
                A2sPlayer {
                    name: "".into(),
                    score: 0,
                    duration: 1.0,
                },
            ],
            rules: [("Version".to_string(), "0.025h3".to_string())].into_iter().collect(),
        };
        let s = base_status(&data, 100);
        assert_eq!(s.version.as_deref(), Some("0.025h3"));
        assert_eq!(s.name.as_deref(), Some("PvE"));
        assert_eq!(s.map, None);
        assert_eq!((s.players_online, s.players_max), (Some(2), Some(10)));
        assert_eq!(s.ping_ms, Some(42));
        assert_eq!(s.players.len(), 1);
        assert_eq!(s.password_protected, Some(true));
    }
}
