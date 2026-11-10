//! Game-specific interfaces layered on top of the Steam query (`native` in the server config):
//!
//! - 7 Days to Die web API: player names and pings (`/api/player`), the in-game clock and the next
//!   blood moon (`/api/bloodmoon`), real uptime (every `/api/log` line carries the milliseconds since
//!   the server started), console commands (`/api/command`) and the console log. Checked against a
//!   V 3.4 server; the server describes its API at `/api/openapi`.
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
    pub started_at_unix: Option<i64>,
    pub blood_moon_in_days: Option<u32>,
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
        if self.started_at_unix.is_some() {
            status.started_at_unix = self.started_at_unix;
        }
        if self.blood_moon_in_days.is_some() {
            status.blood_moon_in_days = self.blood_moon_in_days;
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

/// When the Steam query fails but the game's own API answers, the server is up (the Steam query
/// is off on unlisted 7 Days to Die servers, or blocked by a firewall): build the status from the
/// API alone. Without an API, or if it fails too, the query's error stands.
pub(crate) async fn instead_of_query(
    server: &ServerConfig,
    now_unix: i64,
    query_error: lighthouse_core::QueryError,
) -> Result<ServerStatus, lighthouse_core::QueryError> {
    if server.native.is_none() {
        return Err(query_error);
    }
    match try_extra(server).await {
        Ok(extra) => {
            let mut status = ServerStatus::new(lighthouse_core::ServerState::Online, now_unix);
            // 7 Days to Die: what the Steam query would have said, from /api/serverinfo.
            if let Some((base, name, secret)) = sdtd_api(server)
                && let Ok(info) = sdtd_get(&base, "/api/serverinfo", name, secret).await
            {
                sdtd_server_info(&info, &mut status);
            }
            extra.apply(&mut status);
            Ok(status)
        }
        Err(e) => {
            tracing::debug!(server = %server.id, "native API: {e}");
            Err(query_error)
        }
    }
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
    let (players, moon, log) = tokio::join!(
        sdtd_get(&base, "/api/player", name, secret),
        sdtd_get(&base, "/api/bloodmoon", name, secret),
        sdtd_get(&base, "/api/log?count=-1", name, secret)
    );
    if let (Err(e), Err(_), Err(_)) = (&players, &moon, &log) {
        return Err(e.clone());
    }
    // Older servers without /api/bloodmoon still have the clock in /api/serverstats.
    let clock = match moon {
        Ok(m) => Some(m),
        Err(_) => sdtd_get(&base, "/api/serverstats", name, secret).await.ok(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    Ok(Extra {
        players: players.ok().as_ref().and_then(sdtd_players),
        game_time: clock.as_ref().and_then(sdtd_game_time),
        started_at_unix: log.ok().as_ref().and_then(|l| sdtd_started_at(l, now)),
        blood_moon_in_days: clock.as_ref().and_then(sdtd_blood_moon),
    })
}

/// Each log line carries `uptime`: milliseconds since the server started.
fn sdtd_started_at(json: &Value, now: i64) -> Option<i64> {
    let entry = json.pointer("/data/entries")?.as_array()?.last()?;
    let ms: i64 = match entry.get("uptime")? {
        Value::String(s) => s.trim().parse().ok()?,
        Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    Some(now - ms / 1000)
}

/// `/api/bloodmoon`: `{"data":{"gameTime":{"days":1,...},"bloodmoonActive":false,"nextBloodmoon":{"days":7,...}}}`.
/// Works for random blood moon ranges too: the server knows the day it picked.
fn sdtd_blood_moon(json: &Value) -> Option<u32> {
    let data = json.get("data")?;
    if data.get("bloodmoonActive").and_then(Value::as_bool) == Some(true) {
        return Some(0);
    }
    let today = data.pointer("/gameTime/days")?.as_u64()?;
    let next = data.pointer("/nextBloodmoon/days")?.as_u64()?;
    u32::try_from(next.saturating_sub(today)).ok()
}

/// The server's 7 Days to Die web API login, if set: (base URL, token name, secret).
fn sdtd_api(server: &ServerConfig) -> Option<(String, &str, &str)> {
    match &server.native {
        Some(NativeApi::SevenDaysWebApi {
            port,
            token_name,
            token_secret,
        }) => Some((
            format!("http://{}:{port}", server.host),
            token_name.as_str(),
            token_secret.as_str(),
        )),
        _ => None,
    }
}

/// Whether console commands can go through the 7 Days to Die web API.
pub fn has_sdtd_api(server: &ServerConfig) -> bool {
    sdtd_api(server).is_some()
}

/// Run a console command through the 7 Days to Die web API and return its output.
pub async fn sdtd_command(server: &ServerConfig, command: &str) -> Result<String, String> {
    let (base, name, secret) = sdtd_api(server).ok_or("the 7 Days to Die web API is not set up")?;
    let json: Value = http::client()
        .post(format!("{base}/api/command"))
        .header("X-SDTD-API-TOKENNAME", name)
        .header("X-SDTD-API-SECRET", secret)
        .json(&serde_json::json!({ "command": command, "format": "Simple" }))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(json
        .pointer("/data/resultRaw")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// Console lines from a server that serves its log over the network (7 Days to Die web API):
/// the lines after `after` (or the last 200 when `None`) and the number to ask from next time.
/// `None` when the server has no such source.
pub async fn remote_log(server: &ServerConfig, after: Option<u64>) -> Option<Result<(Vec<String>, u64), String>> {
    let (base, name, secret) = sdtd_api(server)?;
    let path = match after {
        Some(first) => format!("/api/log?firstLine={first}&count=500"),
        None => "/api/log?count=-200".to_string(),
    };
    Some(
        sdtd_get(&base, &path, name, secret)
            .await
            .and_then(|json| sdtd_log_lines(&json).ok_or_else(|| "unexpected answer from /api/log".to_string())),
    )
}

fn sdtd_log_lines(json: &Value) -> Option<(Vec<String>, u64)> {
    let data = json.get("data")?;
    let lines = data
        .get("entries")?
        .as_array()?
        .iter()
        .filter_map(|e| {
            let msg = e.get("msg")?.as_str()?;
            let time = e
                .get("isotime")
                .and_then(Value::as_str)
                .and_then(|t| t.get(11..19))
                .unwrap_or("");
            let kind = match e.get("type").and_then(Value::as_str) {
                Some("Error" | "Exception" | "Assert") => "ERR ",
                Some("Warning") => "WRN ",
                _ => "",
            };
            Some(format!("{time} {kind}{msg}"))
        })
        .collect();
    Some((lines, data.get("lastLine")?.as_u64()?))
}

/// `/api/serverinfo`: `{"data":[{"name":"GameHost","type":"string","value":"..."}, ...]}`, the
/// same keys as the Steam rules.
fn sdtd_server_info(json: &Value, status: &mut ServerStatus) {
    let Some(list) = json.get("data").and_then(Value::as_array) else {
        return;
    };
    let get = |key: &str| {
        list.iter()
            .find(|i| i.get("name").and_then(Value::as_str) == Some(key))?
            .get("value")
    };
    let text = |key: &str| {
        get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    status.name = text("GameHost");
    status.map = text("LevelName");
    status.version = text("ServerVersion");
    status.players_max = get("MaxPlayers").and_then(Value::as_u64).map(|n| n as u32);
    status.password_protected = get("IsPasswordProtected").and_then(Value::as_bool);
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
                // Entity id for kicks; platform ids (`Steam_7656...`, `EOS_...`) for bans.
                let id = p.get("entityId").and_then(Value::as_i64).map(|n| n.to_string());
                let ids: Vec<String> = ["platformId", "crossplatformId"]
                    .iter()
                    .filter_map(|k| match p.get(*k)? {
                        Value::String(s) => Some(s.clone()),
                        Value::Object(o) => o.get("combinedString").and_then(Value::as_str).map(str::to_string),
                        _ => None,
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                (!name.is_empty()).then_some(Player { name, ping_ms, id, ids })
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
        ..Default::default()
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
                ..Default::default()
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    Ok(Extra {
        players: vein_players(&json),
        started_at_unix: vein_started_at(&json, now),
        ..Default::default()
    })
}

/// `uptime` is seconds since the server's session began (measured on a live server: it rose 60.6
/// over 61 s, starting a few seconds after the process).
fn vein_started_at(json: &Value, now: i64) -> Option<i64> {
    let secs = json.get("uptime")?.as_f64()?;
    (secs.is_finite() && secs >= 0.0).then(|| now - secs as i64)
}

/// `{"uptime":1234.5,"onlinePlayers":{"7656...":{"name":"Alice","timeConnected":...}}}`.
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
            ..Default::default()
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
    fn seven_days_uptime_blood_moon_and_log() {
        // Shapes as answered by a V 3.4 server.
        let log = json(
            r#"{"data":{"entries":[{"id":21,"msg":"Executing command 'say'","type":"Log","trace":null,"isotime":"2026-11-10T15:55:44.5871994-07:00","uptime":"92314"},{"id":22,"msg":"Oops","type":"Error","trace":null,"isotime":"2026-11-10T15:55:45.5913488-07:00","uptime":"92318"}],"firstLine":21,"lastLine":23},"meta":{}}"#,
        );
        assert_eq!(sdtd_started_at(&log, 1000), Some(1000 - 92));
        let (lines, next) = sdtd_log_lines(&log).unwrap();
        assert_eq!(lines, ["15:55:44 Executing command 'say'", "15:55:45 ERR Oops"]);
        assert_eq!(next, 23);
        let moon = json(
            r#"{"data":{"gameTime":{"days":1,"hours":7,"minutes":0},"bloodmoonActive":false,"nextBloodmoon":{"days":7,"hours":22,"minutes":0}}}"#,
        );
        assert_eq!(sdtd_blood_moon(&moon), Some(6));
        assert_eq!(
            sdtd_game_time(&moon).unwrap(),
            GameTime {
                day: 1,
                hour: 7,
                minute: 0
            }
        );
        let active = json(r#"{"data":{"gameTime":{"days":7},"bloodmoonActive":true,"nextBloodmoon":{"days":14}}}"#);
        assert_eq!(sdtd_blood_moon(&active), Some(0));
    }

    #[test]
    fn seven_days_server_info() {
        let info = json(
            r#"{"data":[{"name":"GameHost","type":"string","value":"Scratch"},{"name":"LevelName","type":"string","value":"Navezgane"},{"name":"MaxPlayers","type":"int","value":8},{"name":"IsPasswordProtected","type":"bool","value":false},{"name":"ServerVersion","type":"string","value":"V 3.4.0"}]}"#,
        );
        let mut s = ServerStatus::new(lighthouse_core::ServerState::Online, 0);
        sdtd_server_info(&info, &mut s);
        assert_eq!(s.name.as_deref(), Some("Scratch"));
        assert_eq!(s.map.as_deref(), Some("Navezgane"));
        assert_eq!((s.players_max, s.password_protected), (Some(8), Some(false)));
        assert_eq!(s.version.as_deref(), Some("V 3.4.0"));
    }

    #[test]
    fn seven_days_player_ids() {
        let p = sdtd_players(&json(
            r#"{"data":{"players":[{"entityId":171,"name":"Alice","ping":12,"platformId":{"combinedString":"Steam_7656","platformId":"Steam"},"crossplatformId":"EOS_00ab"}]}}"#,
        ))
        .unwrap();
        assert_eq!(p[0].id.as_deref(), Some("171"));
        assert_eq!(p[0].ids, ["Steam_7656", "EOS_00ab"]);
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
        assert_eq!(
            vein_started_at(&json(r#"{"uptime":69.37,"onlinePlayers":{}}"#), 1000),
            Some(931)
        );
        assert_eq!(vein_started_at(&json(r#"{"onlinePlayers":{}}"#), 1000), None);
    }

    #[test]
    fn extra_overrides_count() {
        let mut s = ServerStatus::new(lighthouse_core::ServerState::Online, 0);
        s.players_online = Some(5);
        Extra {
            players: Some(vec![Player {
                name: "A".into(),
                ping_ms: None,
                ..Default::default()
            }]),
            ..Default::default()
        }
        .apply(&mut s);
        assert_eq!(s.players_online, Some(1));
    }
}
