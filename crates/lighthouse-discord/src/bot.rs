//! One Discord bot: keeps its status posts up to date, shows a summary as its status line and
//! posts alerts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lighthouse_core::{BotConfig, BotMode, ServerConfig};
use serde_json::Value;
use tracing::{error, info, warn};
use twilight_gateway::{
    CloseFrame, ConfigBuilder, Event, EventTypeFlags, Intents, MessageSender, Shard, ShardId, StreamExt as _,
};
use twilight_http::Client;
use twilight_http::api_error::ApiError;
use twilight_http::error::ErrorType;
use twilight_model::gateway::payload::outgoing::update_presence::{UpdatePresence, UpdatePresencePayload};
use twilight_model::gateway::presence::{Activity, ActivityType, MinimalActivity, Status};
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, MessageMarker};

use crate::monitor::{Monitor, Transition};
use crate::now_unix;
use crate::persist::{MessageRef, Persist};
use crate::render;

/// Re-render at least this often so the "Updated …" line and uptimes stay fresh.
const REFRESH_EVERY: Duration = Duration::from_secs(60);
/// Collect status changes for this long before editing, so several servers update in one pass.
const SETTLE: Duration = Duration::from_secs(2);
/// How long each server stays in the bot's status line before the next one.
const PRESENCE_ROTATE: Duration = Duration::from_secs(20);
/// Edit a post whose content didn't change only after this long (keeps "Updated" honest).
const FORCE_EDIT_AFTER: Duration = Duration::from_secs(300);

/// Discord error codes we handle specially.
const UNKNOWN_MESSAGE: u64 = 10008;
const UNKNOWN_CHANNEL: u64 = 10003;
const MISSING_ACCESS: u64 = 50001;
const MISSING_PERMISSIONS: u64 = 50013;

pub struct Bot {
    /// Short name for logs and state keys: the game id, or `mega`.
    pub key: String,
    pub config: BotConfig,
    pub token: String,
    pub mode: BotMode,
    pub servers: Vec<ServerConfig>,
    /// The bot's own user id, learned at start-up (used to recognise its earlier posts).
    pub user_id: std::sync::OnceLock<u64>,
}

#[derive(Debug, thiserror::Error)]
enum SendError {
    #[error("{0}")]
    Http(#[from] twilight_http::Error),
    #[error("unexpected reply from Discord: {0}")]
    Body(#[from] twilight_http::response::DeserializeBodyError),
}

fn api_code(e: &twilight_http::Error) -> Option<u64> {
    match e.kind() {
        ErrorType::Response {
            error: ApiError::General(g),
            ..
        } => Some(g.code),
        _ => None,
    }
}

fn send_error_code(e: &SendError) -> Option<u64> {
    match e {
        SendError::Http(e) => api_code(e),
        SendError::Body(_) => None,
    }
}

fn custom_status(text: &str) -> Option<UpdatePresencePayload> {
    let mut activity: Activity = MinimalActivity {
        kind: ActivityType::Custom,
        name: "Custom Status".to_string(),
        url: None,
    }
    .into();
    activity.state = Some(text.chars().take(128).collect());
    UpdatePresencePayload::new(vec![activity], false, None, Status::Online).ok()
}

impl Bot {
    pub async fn run(self, monitor: Arc<Monitor>, persist: Arc<Persist>) {
        let http = Arc::new(Client::new(self.token.clone()));

        match http.current_user().await {
            Ok(resp) => match resp.model().await {
                Ok(user) => {
                    info!(bot = %self.key, "logged in as {}", user.name);
                    let _ = self.user_id.set(user.id.get());
                }
                Err(e) => warn!(bot = %self.key, "could not read bot user: {e}"),
            },
            Err(e) if matches!(e.kind(), ErrorType::Unauthorized) => {
                error!(bot = %self.key, "Discord rejected the token; this bot will not run");
                return;
            }
            Err(e) => warn!(bot = %self.key, "could not reach Discord yet: {e}"),
        }
        let Some(status_channel) = Id::<ChannelMarker>::new_checked(self.config.status_channel_id) else {
            error!(bot = %self.key, "status_channel_id is 0; this bot will not run");
            return;
        };

        // Gateway connection: only needed for the status line, so no intents are requested.
        let mut gateway = ConfigBuilder::new(self.token.clone(), Intents::empty());
        if let Some(p) = custom_status("Starting…") {
            gateway = gateway.presence(p);
        }
        let shard = Shard::with_config(ShardId::ONE, gateway.build());
        let sender = shard.sender();
        tokio::spawn(gateway_loop(shard, self.key.clone()));

        let bot = Arc::new(self);
        tokio::spawn(Arc::clone(&bot).alert_loop(Arc::clone(&http), Arc::clone(&monitor)));
        tokio::spawn(Arc::clone(&bot).presence_loop(sender, Arc::clone(&monitor)));
        bot.board_loop(http, status_channel, monitor, persist).await;
    }

    async fn board_loop(
        &self,
        http: Arc<Client>,
        channel: Id<ChannelMarker>,
        monitor: Arc<Monitor>,
        persist: Arc<Persist>,
    ) {
        let mut changes = monitor.changes();
        // Last content posted per state key, and when it was last edited.
        let mut posted: HashMap<String, (Value, Instant)> = HashMap::new();

        loop {
            let now = now_unix();
            let statuses: Vec<_> = self
                .servers
                .iter()
                .map(|s| {
                    (
                        s,
                        monitor
                            .status(&s.id)
                            .unwrap_or_else(|| lighthouse_core::ServerStatus::offline(now)),
                    )
                })
                .collect();

            // Build the messages this bot owns: one per server, or one compact board.
            // (state key, message body, text that identifies the post if its id was lost)
            let mut messages: Vec<(String, Value, String)> = Vec::new();
            match self.mode {
                BotMode::PerGame => {
                    for (server, status) in &statuses {
                        messages.push((
                            format!("{}/{}", self.key, server.id),
                            render::server_card(server, status, now),
                            render::card_marker(server, status),
                        ));
                    }
                }
                BotMode::Mega => {
                    let entries: Vec<_> = statuses.iter().map(|(s, st)| (*s, st)).collect();
                    messages.push((
                        format!("{}/board", self.key),
                        render::compact_board(
                            &entries,
                            now,
                            self.config.title.as_deref().unwrap_or(render::DEFAULT_TITLE),
                        ),
                        render::BOARD_MARKER.to_string(),
                    ));
                }
            }

            for (key, body, marker) in messages {
                let cmp = render::comparable(&body);
                let fresh = posted
                    .get(&key)
                    .is_some_and(|(prev, at)| *prev == cmp && at.elapsed() < FORCE_EDIT_AFTER);
                if fresh {
                    continue;
                }
                match self.upsert(&http, &persist, &key, channel, &body, &marker).await {
                    Ok(()) => {
                        posted.insert(key, (cmp, Instant::now()));
                    }
                    Err(e) => match send_error_code(&e) {
                        Some(MISSING_ACCESS | MISSING_PERMISSIONS) => error!(
                            bot = %self.key,
                            "cannot post in channel {channel}: give the bot View Channel, Send Messages and Read Message History there"
                        ),
                        Some(UNKNOWN_CHANNEL) => error!(bot = %self.key, "status channel {channel} does not exist"),
                        _ => warn!(bot = %self.key, "could not update {key}: {e}"),
                    },
                }
            }

            // Wait for a status change (then let others settle) or the periodic refresh.
            tokio::select! {
                res = changes.changed() => {
                    if res.is_err() {
                        return;
                    }
                    tokio::time::sleep(SETTLE).await;
                    changes.borrow_and_update();
                }
                () = tokio::time::sleep(REFRESH_EVERY) => {}
            }
        }
    }

    /// Rotate the bot's status line through its servers, grouped by game:
    /// `7 Days to Die - Athens PvE: 6/30 up`, then the next server, and so on.
    async fn presence_loop(self: Arc<Self>, sender: MessageSender, monitor: Arc<Monitor>) {
        let mut index = 0usize;
        let mut last = String::new();
        let mut ticker = tokio::time::interval(PRESENCE_ROTATE);
        loop {
            ticker.tick().await;
            let now = now_unix();
            let entries: Vec<_> = self
                .servers
                .iter()
                .map(|s| {
                    (
                        s,
                        monitor
                            .status(&s.id)
                            .unwrap_or_else(|| lighthouse_core::ServerStatus::offline(now)),
                    )
                })
                .collect();
            let lines = render::presence_lines(&entries);
            if lines.is_empty() {
                continue;
            }
            let text = lines[index % lines.len()].clone();
            index = index.wrapping_add(1);
            if text == last {
                continue;
            }
            if let Some(p) = custom_status(&text) {
                let cmd = UpdatePresence {
                    d: p,
                    op: twilight_model::gateway::OpCode::PresenceUpdate,
                };
                if sender.command(&cmd).is_ok() {
                    last = text;
                }
            }
        }
    }

    /// Look through the channel's recent messages for this bot's own earlier post carrying
    /// `marker`, for when its id was lost (e.g. the state file was deleted or moved).
    async fn find_own_post(
        &self,
        http: &Client,
        persist: &Persist,
        channel: Id<ChannelMarker>,
        marker: &str,
    ) -> Option<Id<MessageMarker>> {
        let me = *self.user_id.get()?;
        let recent = http
            .channel_messages(channel)
            .limit(50)
            .await
            .ok()?
            .models()
            .await
            .ok()?;
        let claimed = persist.message_ids();
        // Match inside the JSON of the post's components, so encode the marker the same way.
        let needle = serde_json::to_string(marker).ok()?;
        let needle = needle.trim_matches('"');
        let needle = if marker == render::BOARD_MARKER {
            format!("\"content\":\"{needle}")
        } else {
            needle.to_string()
        };
        // Newest first: if there are several, the latest is the one people see.
        recent
            .into_iter()
            .find(|m| {
                m.author.id.get() == me
                    && !claimed.contains(&m.id.get())
                    && serde_json::to_string(&m.components).is_ok_and(|c| c.contains(&needle))
            })
            .map(|m| m.id)
    }

    /// Edit this post's message: the remembered one, or the bot's own earlier post found in the
    /// channel; post a new one only if there is neither (or it was deleted).
    async fn upsert(
        &self,
        http: &Client,
        persist: &Persist,
        key: &str,
        channel: Id<ChannelMarker>,
        body: &Value,
        marker: &str,
    ) -> Result<(), SendError> {
        let payload = serde_json::to_vec(body).expect("JSON values always serialise");
        if persist.message(key).filter(|m| m.channel_id == channel.get()).is_none()
            && let Some(found) = self.find_own_post(http, persist, channel, marker).await
        {
            info!(bot = %self.key, "{key}: found its earlier post {found}, updating it");
            persist.set_message(
                key,
                MessageRef {
                    channel_id: channel.get(),
                    message_id: found.get(),
                },
            );
        }
        if let Some(existing) = persist.message(key).filter(|m| m.channel_id == channel.get())
            && let Some(message_id) = Id::new_checked(existing.message_id)
        {
            match http.update_message(channel, message_id).payload_json(&payload).await {
                Ok(_) => return Ok(()),
                Err(e) if api_code(&e) == Some(UNKNOWN_MESSAGE) => {
                    info!(bot = %self.key, "{key}: post was deleted, creating a new one");
                    persist.remove_message(key);
                }
                Err(e) => return Err(e.into()),
            }
        }
        let message = http
            .create_message(channel)
            .payload_json(&payload)
            .await?
            .model()
            .await?;
        persist.set_message(
            key,
            MessageRef {
                channel_id: channel.get(),
                message_id: message.id.get(),
            },
        );
        info!(bot = %self.key, "{key}: posted status message {}", message.id);
        Ok(())
    }

    async fn alert_loop(self: Arc<Self>, http: Arc<Client>, monitor: Arc<Monitor>) {
        let Some(channel) = self.config.alert_channel_id.and_then(Id::<ChannelMarker>::new_checked) else {
            return;
        };
        let mut transitions = monitor.transitions();
        loop {
            let change = match transitions.recv().await {
                Ok(c) => c,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(bot = %self.key, "skipped {n} alerts");
                    continue;
                }
                Err(_) => return,
            };
            let Some(server) = self.servers.iter().find(|s| s.id == change.server_id && s.alerts) else {
                continue;
            };
            let name = server.name.clone().unwrap_or_else(|| server.id.clone());
            let online = change.transition == Transition::CameOnline;
            let body = render::alert_message(&name, online, self.config.alert_role_id, change.at_unix);
            let payload = serde_json::to_vec(&body).expect("JSON values always serialise");
            if let Err(e) = http.create_message(channel).payload_json(&payload).await {
                warn!(bot = %self.key, "could not post alert for {}: {e}", server.id);
            }
        }
    }
}

/// Keep the gateway connection alive. twilight reconnects by itself; we only log.
async fn gateway_loop(mut shard: Shard, key: String) {
    while let Some(item) = shard.next_event(EventTypeFlags::READY).await {
        match item {
            Ok(Event::Ready(ready)) => info!(bot = %key, "connected to the Discord gateway as {}", ready.user.name),
            Ok(Event::GatewayClose(Some(CloseFrame { code, reason, .. }))) => {
                // 4004: bad token, 4013/4014: intents. Those won't fix themselves.
                if matches!(code, 4004 | 4013 | 4014) {
                    error!(bot = %key, "gateway refused the connection ({code}: {reason}); status line disabled");
                    return;
                }
                warn!(bot = %key, "gateway closed ({code}: {reason}), reconnecting");
            }
            Ok(_) => {}
            Err(e) => warn!(bot = %key, "gateway error: {e}"),
        }
    }
}
