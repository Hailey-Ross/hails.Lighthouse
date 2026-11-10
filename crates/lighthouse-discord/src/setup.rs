//! Setting up a bot from just its token: who it is, its invite link, and the channels it can post
//! in. Used by the dashboard's Setup page (and available to the CLI).

use serde::Deserialize;

const API: &str = "https://discord.com/api/v10";

/// View Channels, Send Messages, Embed Links, Attach Files, Read Message History.
pub const BOT_PERMISSIONS: u64 = (1 << 10) | (1 << 11) | (1 << 14) | (1 << 15) | (1 << 16);
pub const PERMISSION_NAMES: &str = "View Channels, Send Messages, Embed Links, Attach Files, Read Message History";
pub const PORTAL: &str = "https://discord.com/developers/applications";

/// The link that adds the bot to a Discord server with the permissions Lighthouse needs.
pub fn invite_url(application_id: u64) -> String {
    format!(
        "https://discord.com/oauth2/authorize?client_id={application_id}&scope=bot%20applications.commands&permissions={BOT_PERMISSIONS}&integration_type=0"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotIdentity {
    pub name: String,
    pub application_id: u64,
}

/// A text channel the bot can see, for picking where its board goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelChoice {
    pub guild: String,
    pub id: u64,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("Discord rejected the token. Copy it again from the Bot page in the Developer Portal (Reset Token).")]
    BadToken,
    #[error("Discord could not be reached: {0}")]
    Unreachable(String),
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

async fn get<T: for<'de> Deserialize<'de>>(token: &str, path: &str) -> Result<T, SetupError> {
    let resp = client()
        .get(format!("{API}{path}"))
        .header("Authorization", format!("Bot {}", token.trim()))
        .send()
        .await
        .map_err(|e| SetupError::Unreachable(e.to_string()))?;
    match resp.status().as_u16() {
        200 => resp.json().await.map_err(|e| SetupError::Unreachable(e.to_string())),
        401 => Err(SetupError::BadToken),
        code => Err(SetupError::Unreachable(format!("HTTP {code} for {path}"))),
    }
}

#[derive(Deserialize)]
struct Application {
    id: String,
    #[serde(default)]
    bot: Option<AppBot>,
    name: String,
}

#[derive(Deserialize)]
struct AppBot {
    username: String,
}

#[derive(Deserialize)]
struct Guild {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Channel {
    id: String,
    #[serde(rename = "type")]
    kind: u8,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: i64,
}

/// Who the token belongs to.
pub async fn identify(token: &str) -> Result<BotIdentity, SetupError> {
    let app: Application = get(token, "/oauth2/applications/@me").await?;
    Ok(BotIdentity {
        name: app.bot.map(|b| b.username).unwrap_or(app.name),
        application_id: app
            .id
            .parse()
            .map_err(|_| SetupError::Unreachable("odd application id".into()))?,
    })
}

/// Text and announcement channels in every Discord server the bot is in, grouped by server and in
/// channel-list order. Empty until the bot has been invited.
pub async fn channels(token: &str) -> Result<Vec<ChannelChoice>, SetupError> {
    let guilds: Vec<Guild> = get(token, "/users/@me/guilds").await?;
    let mut out = Vec::new();
    for g in guilds {
        let Ok(mut list) = get::<Vec<Channel>>(token, &format!("/guilds/{}/channels", g.id)).await else {
            continue;
        };
        list.sort_by_key(|c| c.position);
        out.extend(list.into_iter().filter(|c| matches!(c.kind, 0 | 5)).filter_map(|c| {
            Some(ChannelChoice {
                guild: g.name.clone(),
                id: c.id.parse().ok()?,
                name: c.name.unwrap_or_default(),
            })
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_link_has_scopes_and_permissions() {
        let url = invite_url(42);
        assert!(url.contains("client_id=42"));
        assert!(url.contains("scope=bot%20applications.commands"));
        assert!(url.contains("permissions=117760"));
    }
}
