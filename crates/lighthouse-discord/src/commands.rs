//! Slash commands: `/status [server]`, answered only to the person who asked.
//!
//! Commands are registered globally when a bot starts. They arrive over the bot's gateway
//! connection (no privileged intents needed) and are answered through the interaction callback,
//! which needs no bot token. Replies reuse the status post layouts.

use std::sync::Arc;

use lighthouse_core::{ServerConfig, ServerStatus};
use serde_json::{Value, json};
use tracing::{info, warn};
use twilight_model::application::interaction::application_command::{CommandData, CommandOptionValue};
use twilight_model::application::interaction::{Interaction, InteractionData, InteractionType};

use crate::monitor::Monitor;
use crate::{now_unix, render};

const API: &str = "https://discord.com/api/v10";
/// Interaction response types.
const CHANNEL_MESSAGE: u8 = 4;
const AUTOCOMPLETE_RESULT: u8 = 8;
/// Message flags.
const EPHEMERAL: u64 = 1 << 6;
/// Discord shows at most 25 autocomplete choices.
const MAX_CHOICES: usize = 25;

/// The commands every Lighthouse bot offers.
pub fn definitions() -> Value {
    json!([{
        "name": "status",
        "type": 1,
        "description": "Show the status of the game servers (only you see the answer)",
        "contexts": [0],
        "options": [{
            "type": 3,
            "name": "server",
            "description": "One server (leave empty for all)",
            "required": false,
            "autocomplete": true,
        }],
    }])
}

fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("Lighthouse/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("TLS backend is compiled in")
    })
}

/// Replace the bot's global commands with Lighthouse's.
pub async fn register(token: &str, application_id: u64, key: &str) {
    let resp = client()
        .put(format!("{API}/applications/{application_id}/commands"))
        .header("Authorization", format!("Bot {token}"))
        .json(&definitions())
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => info!(bot = %key, "slash commands registered (/status)"),
        Ok(r) => warn!(bot = %key, "could not register slash commands: {}", r.status()),
        Err(e) => warn!(bot = %key, "could not register slash commands: {e}"),
    }
}

/// Name shown for a server in replies and autocomplete.
fn label(server: &ServerConfig, status: Option<&ServerStatus>) -> String {
    server
        .name
        .clone()
        .or_else(|| status.and_then(|s| s.name.clone()))
        .unwrap_or_else(|| server.id.clone())
}

/// Servers whose id or name contains `typed` (case-insensitive), as autocomplete choices.
pub(crate) fn choices(servers: &[ServerConfig], monitor: &Monitor, typed: &str) -> Value {
    let typed = typed.trim().to_lowercase();
    let list: Vec<Value> = servers
        .iter()
        .map(|s| (s, label(s, monitor.status(&s.id).as_ref())))
        .filter(|(s, name)| typed.is_empty() || name.to_lowercase().contains(&typed) || s.id.contains(&typed))
        .take(MAX_CHOICES)
        .map(|(s, name)| json!({ "name": name.chars().take(100).collect::<String>(), "value": s.id }))
        .collect();
    Value::Array(list)
}

/// The reply to `/status`: one server's card, or the board of all of them.
pub(crate) fn status_reply(servers: &[ServerConfig], monitor: &Monitor, wanted: Option<&str>, title: &str) -> Value {
    let now = now_unix();
    let wanted = wanted.map(str::trim).filter(|w| !w.is_empty());
    let mut message = match wanted {
        Some(w) => {
            let lower = w.to_lowercase();
            let found = servers.iter().find(|s| s.id == w).or_else(|| {
                servers
                    .iter()
                    .find(|s| label(s, monitor.status(&s.id).as_ref()).to_lowercase() == lower)
            });
            match found {
                Some(server) => {
                    let status = monitor.status(&server.id).unwrap_or_else(|| ServerStatus::offline(now));
                    render::server_card(server, &status, now)
                }
                None => {
                    json!({ "content": format!("No server called \"{w}\". Start typing to pick one from the list.") })
                }
            }
        }
        None => {
            let statuses: Vec<ServerStatus> = servers
                .iter()
                .map(|s| monitor.status(&s.id).unwrap_or_else(|| ServerStatus::offline(now)))
                .collect();
            let entries: Vec<(&ServerConfig, &ServerStatus)> = servers.iter().zip(statuses.iter()).collect();
            render::compact_board(&entries, now, title)
        }
    };
    let flags = message.get("flags").and_then(Value::as_u64).unwrap_or(0);
    message["flags"] = json!(flags | EPHEMERAL);
    message
}

fn option_string(data: &CommandData, name: &str) -> Option<String> {
    data.options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            CommandOptionValue::String(s) => Some(s.clone()),
            CommandOptionValue::Focused(s, _) => Some(s.clone()),
            _ => None,
        })
}

/// Answer one interaction.
pub async fn handle(
    interaction: Interaction,
    servers: Arc<Vec<ServerConfig>>,
    monitor: Arc<Monitor>,
    title: String,
    key: String,
) {
    let Some(InteractionData::ApplicationCommand(data)) = &interaction.data else {
        return;
    };
    if data.name != "status" {
        return;
    }
    let body = match interaction.kind {
        InteractionType::ApplicationCommandAutocomplete => json!({
            "type": AUTOCOMPLETE_RESULT,
            "data": { "choices": choices(&servers, &monitor, &option_string(data, "server").unwrap_or_default()) },
        }),
        InteractionType::ApplicationCommand => json!({
            "type": CHANNEL_MESSAGE,
            "data": status_reply(&servers, &monitor, option_string(data, "server").as_deref(), &title),
        }),
        _ => return,
    };
    let url = format!("{API}/interactions/{}/{}/callback", interaction.id, interaction.token);
    match client().post(url).json(&body).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            let status = r.status();
            warn!(bot = %key, "could not answer /status: {status} {}", r.text().await.unwrap_or_default())
        }
        Err(e) => warn!(bot = %key, "could not answer /status: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lighthouse_core::{GameKind, Relay};

    fn servers() -> Vec<ServerConfig> {
        let mut a = ServerConfig::new("athens", GameKind::get("7dtd"), "203.0.113.10", 26900);
        a.name = Some("Nightshade Athens".into());
        let b = ServerConfig::new("pz-main", GameKind::get("pz"), "203.0.113.10", 8766);
        vec![a, b]
    }

    #[test]
    fn autocomplete_filters_by_name_and_id() {
        let servers = servers();
        let monitor = Monitor::new(&servers, None, Arc::new(Relay::default()));
        let all = choices(&servers, &monitor, "");
        assert_eq!(all.as_array().unwrap().len(), 2);
        let athens = choices(&servers, &monitor, "ATH");
        assert_eq!(athens, json!([{ "name": "Nightshade Athens", "value": "athens" }]));
        assert_eq!(choices(&servers, &monitor, "pz").as_array().unwrap().len(), 1);
    }

    #[test]
    fn replies_are_ephemeral_cards_or_boards() {
        let servers = servers();
        let monitor = Monitor::new(&servers, None, Arc::new(Relay::default()));
        let one = status_reply(&servers, &monitor, Some("athens"), "Servers");
        assert_eq!(one["flags"].as_u64().unwrap() & EPHEMERAL, EPHEMERAL);
        assert!(one.to_string().contains("Nightshade Athens"));
        let by_name = status_reply(&servers, &monitor, Some("nightshade athens"), "Servers");
        assert!(by_name.to_string().contains("Nightshade Athens"));
        let board = status_reply(&servers, &monitor, None, "Servers");
        assert!(board.to_string().contains("## Servers"));
        let missing = status_reply(&servers, &monitor, Some("nope"), "Servers");
        assert!(missing["content"].as_str().unwrap().contains("No server called"));
    }

    #[test]
    fn definition_shape() {
        let d = definitions();
        assert_eq!(d[0]["name"], "status");
        assert_eq!(d[0]["options"][0]["autocomplete"], true);
    }
}
