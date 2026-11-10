//! Where game mods send heartbeats (`[ingest]` in the config). See `docs/heartbeat.md`.
//!
//! `POST /v1/heartbeat` with `Authorization: Bearer <secret>`. The reply carries commands waiting
//! for the server's mod and, for mods that enforce Lighthouse bans, the ban list when theirs is out
//! of date. Wrong secrets count towards the same escalating address blocks as guessed invite codes.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lighthouse_core::{Config, HEARTBEAT_VERSION, Heartbeat, HeartbeatReply, Relay};
use lighthouse_store::Store;
use tracing::{debug, info, warn};

/// Heartbeats are small; this leaves room for a burst of console lines.
const MAX_BODY: usize = 512 * 1024;

pub struct Ingest {
    pub config: Config,
    pub relay: Arc<Relay>,
    pub store: Arc<Store>,
}

pub fn router(ingest: Arc<Ingest>) -> Router {
    Router::new()
        .route("/v1/heartbeat", post(heartbeat))
        .route("/v1/health", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(ingest)
}

pub async fn serve(ingest: Arc<Ingest>, bind: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!("listening for game mod heartbeats on http://{}", listener.local_addr()?);
    axum::serve(
        listener,
        router(ingest).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

/// Compare without leaking where the first difference is.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn client_ip(config: &Config, headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
    if config.dashboard.trust_proxy
        && let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok())
    {
        return ip;
    }
    peer.ip()
}

fn reject(status: StatusCode, why: &str) -> Response {
    (status, why.to_string()).into_response()
}

async fn heartbeat(
    State(ingest): State<Arc<Ingest>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let now = crate::now_unix();
    let ip = client_ip(&ingest.config, &headers, peer).to_string();
    if ingest.store.active_block(&ip, now).ok().flatten().is_some() {
        return reject(StatusCode::FORBIDDEN, "blocked");
    }
    let Some(secret) = ingest.config.ingest.as_ref().map(|i| i.secret.as_bytes()) else {
        return reject(StatusCode::NOT_FOUND, "heartbeats are not enabled");
    };
    let given = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !same_secret(given.trim().as_bytes(), secret) {
        warn!(%ip, "heartbeat with a wrong secret");
        if let Ok(Some(block)) = ingest.store.record_bad_code(&ip, now) {
            warn!(%ip, "blocked after repeated wrong secrets (strike {})", block.strikes);
        }
        return reject(StatusCode::UNAUTHORIZED, "wrong secret");
    }

    let hb: Heartbeat = match serde_json::from_slice(&body) {
        Ok(hb) => hb,
        Err(e) => return reject(StatusCode::BAD_REQUEST, &format!("not a heartbeat: {e}")),
    };
    if hb.v != HEARTBEAT_VERSION {
        return reject(
            StatusCode::BAD_REQUEST,
            &format!("unsupported heartbeat version {}", hb.v),
        );
    }
    let Some(server) = ingest.config.servers.iter().find(|s| s.id == hb.server_id) else {
        return reject(
            StatusCode::NOT_FOUND,
            &format!("no server with id {:?} in lighthouse.toml", hb.server_id),
        );
    };
    if hb.game != server.game {
        return reject(
            StatusCode::BAD_REQUEST,
            &format!(
                "server {:?} is {} in lighthouse.toml, not {}",
                server.id, server.game, hb.game
            ),
        );
    }
    debug!(server = %server.id, players = ?hb.status.players_online, "heartbeat");

    if let Err(e) = ingest.store.add_events(&server.id, &hb.events, now) {
        warn!(server = %server.id, "could not save events: {e}");
    }
    for e in &hb.events {
        info!(server = %server.id, kind = %e.kind, player = ?e.player, author = ?e.author, "server event");
    }

    // Only mods that enforce Lighthouse bans send a revision (empty at first).
    let mut reply_bans = None;
    let mut reply_rev = None;
    if let Some(theirs) = hb.bans_rev.as_deref() {
        match ingest.store.active_bans(&server.id, now) {
            Ok((bans, rev)) => {
                if theirs != rev {
                    reply_bans = Some(bans);
                }
                reply_rev = Some(rev);
            }
            Err(e) => warn!(server = %server.id, "could not read bans: {e}"),
        }
    }

    let commands = ingest.relay.heartbeat(hb, now);
    for c in &commands {
        info!(server = %server.id, action = c.action.key(), "handed a command to the server's mod");
    }
    Json(HeartbeatReply {
        v: HEARTBEAT_VERSION,
        commands,
        bans: reply_bans,
        bans_rev: reply_rev,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use lighthouse_core::{ControlAction, IngestConfig, ServerConfig};
    use lighthouse_store::NewBan;
    use tower::ServiceExt;

    fn setup() -> Arc<Ingest> {
        let config = Config {
            ingest: Some(IngestConfig {
                bind: "127.0.0.1:0".into(),
                secret: "s3cret-s3cret-s3cret".into(),
            }),
            servers: vec![ServerConfig::new(
                "rp",
                lighthouse_core::GameKind::get("fivem"),
                "x",
                30120,
            )],
            ..Default::default()
        };
        Arc::new(Ingest {
            config,
            relay: Arc::default(),
            store: Arc::new(Store::open_in_memory().unwrap()),
        })
    }

    async fn send(ingest: &Arc<Ingest>, secret: &str, body: &str) -> (StatusCode, String) {
        let mut req = Request::post("/v1/heartbeat")
            .header("authorization", format!("Bearer {secret}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 5000))));
        let resp = router(Arc::clone(ingest)).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), MAX_BODY).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    const HB: &str = r#"{"v":1,"server_id":"rp","game":"fivem","sent_at_unix":1,"bans_rev":"",
        "status":{"state":"online","players_online":3,"observed_at_unix":1},
        "events":[{"kind":"ban","player":"Bob","author":"Admin","reason":"rdm","action_id":"ABCD-1234"}],
        "log":["[script] hello"]}"#;

    #[tokio::test]
    async fn heartbeat_round_trip() {
        let ingest = setup();
        assert_eq!(send(&ingest, "wrong", HB).await.0, StatusCode::UNAUTHORIZED);

        ingest.relay.queue("rp", ControlAction::Restart, crate::now_unix());
        ingest
            .store
            .add_ban(
                None,
                &NewBan {
                    ids: vec!["license:x".into()],
                    reason: "r".into(),
                    author: "a".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let (status, body) = send(&ingest, "s3cret-s3cret-s3cret", HB).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let reply: HeartbeatReply = serde_json::from_str(&body).unwrap();
        assert_eq!(reply.commands.len(), 1);
        assert_eq!(reply.bans.as_ref().map(Vec::len), Some(1));
        let rev = reply.bans_rev.unwrap();

        // Saved what the mod reported.
        assert_eq!(ingest.store.events(&["rp"], None, 10).unwrap().len(), 1);
        assert_eq!(ingest.relay.log_since("rp", 0).len(), 1);
        assert_eq!(
            ingest
                .relay
                .fresh_status("rp", crate::now_unix(), 60)
                .unwrap()
                .players_online,
            Some(3)
        );

        // Up-to-date ban list: not sent again; commands only once.
        let again = HB
            .replace(r#""bans_rev":"""#, &format!(r#""bans_rev":"{rev}""#))
            .replace(r#""kind":"ban""#, r#""kind":"warn""#);
        let reply: HeartbeatReply =
            serde_json::from_str(&send(&ingest, "s3cret-s3cret-s3cret", &again).await.1).unwrap();
        assert!(reply.commands.is_empty() && reply.bans.is_none());
    }

    #[tokio::test]
    async fn rejects_unknown_servers_and_wrong_games() {
        let ingest = setup();
        let other = HB.replace(r#""server_id":"rp""#, r#""server_id":"nope""#);
        assert_eq!(
            send(&ingest, "s3cret-s3cret-s3cret", &other).await.0,
            StatusCode::NOT_FOUND
        );
        let wrong_game = HB.replace(r#""game":"fivem""#, r#""game":"7dtd""#);
        assert_eq!(
            send(&ingest, "s3cret-s3cret-s3cret", &wrong_game).await.0,
            StatusCode::BAD_REQUEST
        );
        assert!(same_secret(b"abc", b"abc") && !same_secret(b"abc", b"abd") && !same_secret(b"ab", b"abc"));
    }
}
