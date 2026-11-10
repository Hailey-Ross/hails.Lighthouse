//! Discord side of Lighthouse: polling, status posts, presence and alerts.
//!
//! Messages are rendered as raw Discord JSON (Components V2) and sent with twilight's
//! `payload_json`, so the renderer does not depend on the Discord library's types.

mod bot;
pub mod commands;
pub mod monitor;
pub mod persist;
pub mod render;
pub mod setup;

use std::sync::Arc;

use lighthouse_core::{BotMode, Config};
use tracing::error;

pub use bot::Bot;

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resolve each bot's token from the config or its environment variable.
fn token_for(bot: &lighthouse_core::BotConfig) -> Result<String, String> {
    if let Some(t) = &bot.token {
        return Ok(t.trim().to_string());
    }
    let var = bot.token_env.as_deref().unwrap_or_default();
    std::env::var(var)
        .map(|t| t.trim().to_string())
        .map_err(|_| format!("environment variable {var} is not set"))
}

/// Start every configured bot. Polling is started separately (see [`monitor::Monitor::spawn_pollers`])
/// so the dashboard can share the same monitor.
pub fn start_bots(config: &Config, monitor: &Arc<monitor::Monitor>, persist: &Arc<persist::Persist>) {
    for bot_config in &config.bots {
        let key = bot_config
            .game
            .map_or_else(|| "mega".to_string(), |g| g.id().to_string());
        let token = match token_for(bot_config) {
            Ok(t) => t,
            Err(e) => {
                error!(bot = %key, "no token: {e}");
                continue;
            }
        };
        let servers = config.servers_for(bot_config).cloned().collect::<Vec<_>>();
        let mode = if bot_config.game.is_none() {
            BotMode::Mega
        } else {
            config.mode
        };
        let bot = Bot {
            user_id: Default::default(),
            key,
            config: bot_config.clone(),
            token,
            mode,
            servers,
        };
        tokio::spawn(bot.run(Arc::clone(monitor), Arc::clone(persist)));
    }
}
