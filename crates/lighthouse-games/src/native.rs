//! Game-specific interfaces layered on top of the Steam query (`native` in the server config):
//!
//! - 7 Days to Die web API: player names and pings (`/api/player`), the in-game clock (`/api/serverstats`).
//! - Project Zomboid RCON: the `players` command, which lists names and gives a reliable count on
//!   Build 42 (its Steam count can lag).
//! - VEIN HTTP API: `/status` lists online players by name.
//!
//! These only add detail. If one fails, the Steam result is used as it is and the error is logged.

use std::time::Duration;

use lighthouse_core::{GameTime, NativeApi, Player, ServerConfig, ServerStatus};
use serde_json::Value;

use crate::http;
use crate::rcon::Rcon;

/// What a native API added. Every field is optional; `None` keeps the Steam value.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Extra {
    pub players: Option<Vec<Player>>,
    pub game_time: Option<GameTime>,
}

impl Extra {
    pub fn apply(self, status: &mut ServerStatus) {
        if let Some(players) = self.players {
            status.players_online = Some(players.len() as u32);
            status.players = players;
        }
        if let Some(t) = self.game_time {
            status.game_time = Some(t);
        }
    }
}

/// Query the server's native API, if one is configured. Errors are logged and swallowed.
pub(crate) async fn extra(server: &ServerConfig) -> Extra {
    try_extra(server).await.unwrap_or_else(|e| {
        tracing::debug!(server = %server.id, "native API: {e}");
        Extra::default()
    })
}

/// Like [`extra`], but returns the error (for setup checks and `lighthouse query`).
pub(crate) async fn try_extra(server: &ServerConfig) -> Result<Extra, String> {
    let Some(api) = &server.native else {
        return Ok(Extra::default());
    };
    match api {
        NativeApi::SevenDaysWebApi {
            port,
            token_name,
            token_secret,
        } => seven_days(&server.host, *port, token_name, token_secret).await,
        NativeApi::Rcon { port, password } => zomboid_rcon(&server.host, *port, password).await,
        NativeApi::VeinHttp { port } => vein_http(&server.host, *port).await,
    }
}

// ---------- 7 Days to Die web API ----------

async fn sdtd_get(base: &str, path: &str, name: &str, secret: &str) -> Result<Value, String> {
    http::client()
        .get(format!("{base}{path}"))
        .header("X-SDTD-API-TOKENNAME", name)
        .header("X-SDTD-API-SECRET", secret)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

async fn seven_days(host: &str, port: u16, name: &str, secret: &str) -> Result<Extra, String> {
    let base = format!("http://{host}:{port}");
    let (players, stats) = tokio::join!(
        sdtd_get(&base, "/api/player", name, secret),
        sdtd_get(&base, "/api/serverstats", name, secret)
    );
    if let (Err(e), Err(_)) = (&players, &stats) {
        return Err(e.clone());
    }
    Ok(Extra {
        players: players.ok().as_ref().and_then(sdtd_players),
        game_time: stats.ok().as_ref().and_then(sdtd_game_time),
    })
}

/// `{"data":{"players":[{"name":"Alice","ping":40,"online":true}, ...]}}`. Offline players are
/// listed too on some versions, so keep only those marked online (or not marked at all).
fn sdtd_players(json: &Value) -> Option<Vec<Player>> {
    let list = json
        .pointer("/data/players")
        .or_else(|| json.get("players"))?
        .as_array()?;
    Some(
        list.iter()
            .filter(|p| p.get("online").and_then(Value::as_bool).unwrap_or(true))
            .filter_map(|p| {
                let name = p.get("name")?.as_str()?.trim().to_string();
                let ping_ms = p
                    .get("ping")
                    .and_then(Value::as_u64)
                    .map(|n| n.min(u64::from(u32::MAX)) as u32);
                (!name.is_empty()).then_some(Player { name, ping_ms })
            })
            .collect(),
    )
}

/// `{"data":{"gameTime":{"days":14,"hours":21,"minutes":30}}}`
fn sdtd_game_time(json: &Value) -> Option<GameTime> {
    let t = json.pointer("/data/gameTime").or_else(|| json.get("gameTime"))?;
    let n = |k: &str| t.get(k).and_then(Value::as_u64);
    Some(GameTime {
        day: u32::try_from(n("days")?).ok()?,
        hour: u8::try_from(n("hours")?).ok()?,
        minute: u8::try_from(n("minutes")?).ok()?,
    })
}

// ---------- Project Zomboid RCON ----------

async fn zomboid_rcon(host: &str, port: u16, password: &str) -> Result<Extra, String> {
    let mut rcon = Rcon::connect(&format!("{host}:{port}"), password, Duration::from_secs(4))
        .await
        .map_err(|e| e.to_string())?;
    // PZ ignores the terminator packet, so wait for a short silence instead.
    rcon.use_terminator = false;
    rcon.quiet = Duration::from_millis(300);
    let reply = rcon.exec("players").await.map_err(|e| e.to_string())?;
    let players = zomboid_players(&reply).ok_or_else(|| format!("unexpected reply: {reply:.80}"))?;
    Ok(Extra {
        players: Some(players),
        game_time: None,
    })
}

/// `Players connected (2):\n-Alice\n-Bob\n`
fn zomboid_players(reply: &str) -> Option<Vec<Player>> {
    let mut lines = reply.lines();
    lines.find(|l| l.trim_start().starts_with("Players connected"))?;
    Some(
        lines
            .filter_map(|l| l.trim().strip_prefix('-'))
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|name| Player {
                name: name.to_string(),
                ping_ms: None,
            })
            .collect(),
    )
}

// ---------- VEIN HTTP API ----------

async fn vein_http(host: &str, port: u16) -> Result<Extra, String> {
    let json: Value = http::client()
        .get(format!("http://{host}:{port}/status"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Extra {
        players: vein_players(&json),
        game_time: None,
    })
}

/// `{"uptime":1234.5,"onlinePlayers":{"7656...":{"name":"Alice","timeConnected":...}}}`.
/// The `uptime` unit isn't confirmed yet, so it isn't used.
fn vein_players(json: &Value) -> Option<Vec<Player>> {
    let online = json.get("onlinePlayers")?;
    let entries: Vec<&Value> = match online {
        Value::Object(map) => map.values().collect(),
        Value::Array(list) => list.iter().collect(),
        _ => return None,
    };
    let mut players: Vec<Player> = entries
        .into_iter()
        .filter_map(|p| p.get("name").or_else(|| p.get("Name"))?.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|name| Player {
            name: name.to_string(),
            ping_ms: None,
        })
        .collect();
    players.sort_by_key(|p| p.name.to_lowercase());
    Some(players)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn seven_days_players_and_time() {
        let p = sdtd_players(&json(
            r#"{"data":{"players":[{"name":"Alice","ping":40,"online":true},{"name":"Bob","online":false},{"name":"Cy"}]},"meta":{}}"#,
        ))
        .unwrap();
        assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Alice", "Cy"]);
        assert_eq!(p[0].ping_ms, Some(40));
        let t = sdtd_game_time(&json(r#"{"data":{"gameTime":{"days":14,"hours":21,"minutes":30}}}"#)).unwrap();
        assert_eq!(
            t,
            GameTime {
                day: 14,
                hour: 21,
                minute: 30
            }
        );
    }

    #[test]
    fn zomboid_player_list() {
        let p = zomboid_players("Players connected (2): \n-Alice\n-Bob the Builder\n").unwrap();
        assert_eq!(
            p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["Alice", "Bob the Builder"]
        );
        assert_eq!(zomboid_players("Players connected (0): \n").unwrap(), vec![]);
        assert_eq!(zomboid_players("Unknown command"), None);
    }

    #[test]
    fn vein_status() {
        let p = vein_players(&json(
            r#"{"uptime":12.5,"onlinePlayers":{"1":{"name":"zed","characterId":1},"2":{"name":"Amy"}}}"#,
        ))
        .unwrap();
        assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Amy", "zed"]);
        assert_eq!(vein_players(&json(r#"{"uptime":1}"#)), None);
    }

    #[test]
    fn extra_overrides_count() {
        let mut s = ServerStatus::new(lighthouse_core::ServerState::Online, 0);
        s.players_online = Some(5);
        Extra {
            players: Some(vec![Player {
                name: "A".into(),
                ping_ms: None,
            }]),
            game_time: None,
        }
        .apply(&mut s);
        assert_eq!(s.players_online, Some(1));
    }
}
