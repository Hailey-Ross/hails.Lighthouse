//! Status cards as Discord Components V2 message bodies.
//!
//! Reference: <https://discord.com/developers/docs/components/reference>. A V2 message cannot also
//! carry `content` or `embeds`, so everything goes into components.

use lighthouse_core::{GameKind, ServerConfig, ServerState, ServerStatus};
use serde_json::{Value, json};

/// Message flag that switches a message to Components V2.
pub const IS_COMPONENTS_V2: u64 = 1 << 15;

const CONTAINER: u8 = 17;
const TEXT_DISPLAY: u8 = 10;
const SEPARATOR: u8 = 14;

/// Discord caps the text across all components of a message at 4000 characters.
const MAX_TEXT: usize = 4000;
/// Room kept for everything on a card except the player list.
const PLAYER_LIST_BUDGET: usize = 1500;

pub const COLOR_ONLINE: u32 = 0x2ECC71;
pub const COLOR_STARTING: u32 = 0xF1C40F;
pub const COLOR_OFFLINE: u32 = 0xE74C3C;
pub const COLOR_UNKNOWN: u32 = 0x95A5A6;

fn state_color(state: ServerState) -> u32 {
    match state {
        ServerState::Online => COLOR_ONLINE,
        ServerState::Starting => COLOR_STARTING,
        ServerState::Offline => COLOR_OFFLINE,
        ServerState::Unknown => COLOR_UNKNOWN,
    }
}

fn state_label(state: ServerState) -> &'static str {
    match state {
        ServerState::Online => "🟢 **Online**",
        ServerState::Starting => "🟡 **Starting**",
        ServerState::Offline => "🔴 **Offline**",
        ServerState::Unknown => "⚪ **Unknown**",
    }
}

/// `3d 4h`, `4h 12m`, `12m`. Short on purpose: it is refreshed on every update.
pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = secs % 86_400 / 3_600;
    let mins = secs % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

/// Strip characters that would break Discord markdown in names coming from game servers.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '*' | '_' | '`' | '~' | '|' | '>' | '#' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn display_name(server: &ServerConfig, status: &ServerStatus) -> String {
    server
        .name
        .clone()
        .or_else(|| status.name.clone())
        .unwrap_or_else(|| server.id.clone())
}

fn players_text(status: &ServerStatus) -> Option<String> {
    match (status.players_online, status.players_max) {
        (Some(n), Some(max)) => Some(format!("👥 {n}/{max} players")),
        (Some(n), None) => Some(format!("👥 {n} players")),
        _ => None,
    }
}

/// The body lines of one server card (without the title).
fn card_lines(server: &ServerConfig, status: &ServerStatus, now_unix: i64) -> Vec<String> {
    let mut lines = Vec::new();

    let mut head = vec![state_label(status.state).to_string()];
    if status.state == ServerState::Online {
        head.extend(players_text(status));
        if let Some(up) = status.uptime_secs(now_unix) {
            head.push(format!("⏱️ up {}", format_uptime(up)));
        }
        if let Some(ping) = status.ping_ms {
            head.push(format!("{ping} ms"));
        }
    }
    lines.push(head.join(" · "));

    if status.state == ServerState::Online {
        let mut info = Vec::new();
        if let Some(map) = &status.map {
            info.push(format!("🗺️ {}", escape(map)));
        }
        if let Some(t) = status.game_time {
            info.push(format!("Day {}, {:02}:{:02}", t.day, t.hour, t.minute));
        }
        if let Some(days) = status.blood_moon_in_days {
            info.push(match days {
                0 => "🩸 Blood moon tonight".to_string(),
                1 => "🩸 Blood moon tomorrow".to_string(),
                n => format!("🩸 Blood moon in {n} days"),
            });
        }
        if status.password_protected == Some(true) {
            info.push("🔒 Password".to_string());
        }
        if !info.is_empty() {
            lines.push(info.join(" · "));
        }
        if let Some(v) = &status.version {
            lines.push(format!("-# Version {}", escape(v)));
        }
        if server.show_players && !status.players.is_empty() {
            lines.push(player_list(status));
        }
    }

    if server.show_address {
        lines.push(format!("Join: `{}:{}`", server.host, server.game_port));
    }
    lines
}

fn player_list(status: &ServerStatus) -> String {
    let mut out = String::from("**Players:** ");
    let total = status.players.len();
    for (i, p) in status.players.iter().enumerate() {
        let name = escape(&p.name);
        if out.len() + name.len() + 2 > PLAYER_LIST_BUDGET {
            out.push_str(&format!("… and {} more", total - i));
            return out;
        }
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&name);
    }
    out
}

fn text(content: String) -> Value {
    json!({ "type": TEXT_DISPLAY, "content": content })
}

fn updated_footer(now_unix: i64) -> Value {
    // <t:..:R> renders as "2 minutes ago" in each viewer's own locale.
    text(format!("-# Updated <t:{now_unix}:R>"))
}

/// One message per server: the per-game bot layout.
pub fn server_card(server: &ServerConfig, status: &ServerStatus, now_unix: i64) -> Value {
    let title = format!("### {}", escape(&display_name(server, status)));
    let body = card_lines(server, status, now_unix).join("\n");
    json!({
        "flags": IS_COMPONENTS_V2,
        "components": [{
            "type": CONTAINER,
            "accent_color": state_color(status.state),
            "components": [
                text(title),
                text(body),
                { "type": SEPARATOR, "divider": true, "spacing": 1 },
                updated_footer(now_unix),
            ],
        }],
    })
}

/// One compact line per server, e.g. `🟢 **PvE Main** · 👥 12/40 · ⏱️ up 3h 2m`.
fn compact_line(server: &ServerConfig, status: &ServerStatus, now_unix: i64) -> String {
    let mut parts = vec![format!(
        "{} **{}**",
        state_label(status.state).split(' ').next().unwrap_or(""),
        escape(&display_name(server, status))
    )];
    if status.state == ServerState::Online {
        if let (Some(n), Some(max)) = (status.players_online, status.players_max) {
            parts.push(format!("👥 {n}/{max}"));
        }
        if let Some(up) = status.uptime_secs(now_unix) {
            parts.push(format!("⏱️ up {}", format_uptime(up)));
        }
    }
    if server.show_address {
        parts.push(format!("`{}:{}`", server.host, server.game_port));
    }
    parts.join(" · ")
}

/// A single compact board grouped by game: the megabot layout, also usable as an overview message.
pub fn compact_board(entries: &[(&ServerConfig, &ServerStatus)], now_unix: i64) -> Value {
    let mut games: Vec<GameKind> = entries.iter().map(|(s, _)| s.game).collect();
    games.sort();
    games.dedup();

    let mut containers = Vec::new();
    for game in games {
        let in_game: Vec<_> = entries.iter().filter(|(s, _)| s.game == game).collect();
        let up = in_game.iter().filter(|(_, st)| st.state == ServerState::Online).count();
        let color = if up == in_game.len() {
            COLOR_ONLINE
        } else if up == 0 {
            COLOR_OFFLINE
        } else {
            COLOR_STARTING
        };
        let lines: Vec<String> = in_game.iter().map(|(s, st)| compact_line(s, st, now_unix)).collect();
        containers.push(json!({
            "type": CONTAINER,
            "accent_color": color,
            "components": [
                text(format!("### {game} · {up}/{} up", in_game.len())),
                text(truncate(&lines.join("\n"), MAX_TEXT / 4)),
            ],
        }));
    }
    containers.push(updated_footer(now_unix));
    json!({ "flags": IS_COMPONENTS_V2, "components": containers })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub(1);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// A copy of a rendered message without its "Updated …" footer, for deciding whether an edit is
/// needed (the footer changes on every render).
pub fn comparable(message: &Value) -> Value {
    fn strip(v: &Value) -> Value {
        match v {
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .filter(|c| {
                        !(c["type"] == TEXT_DISPLAY
                            && c["content"].as_str().is_some_and(|t| t.starts_with("-# Updated")))
                    })
                    .map(strip)
                    .collect(),
            ),
            Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), strip(v))).collect()),
            other => other.clone(),
        }
    }
    strip(message)
}

/// Plain alert message for a state change. Only the given role may be pinged.
pub fn alert_message(server_name: &str, online: bool, role_id: Option<u64>, at_unix: i64) -> Value {
    let what = if online {
        "🟢 **{name}** is back online"
    } else {
        "🔴 **{name}** is offline"
    };
    let mut content = what.replace("{name}", &escape(server_name));
    content.push_str(&format!(" · <t:{at_unix}:R>"));
    if let Some(role) = role_id {
        content = format!("<@&{role}> {content}");
    }
    json!({
        "content": content,
        "allowed_mentions": { "parse": [], "roles": role_id.map(|r| vec![r.to_string()]).unwrap_or_default() },
    })
}

/// One status-line entry per server, grouped by game (catalogue order) and then config order:
/// `7 Days to Die - Athens PvE: 6/30 up` or `VEIN - Harbour Town: down`.
pub fn presence_lines(entries: &[(&ServerConfig, ServerStatus)]) -> Vec<String> {
    let mut sorted: Vec<&(&ServerConfig, ServerStatus)> = entries.iter().collect();
    sorted.sort_by_key(|(server, _)| server.game); // stable: keeps config order within a game
    sorted
        .into_iter()
        .map(|(server, status)| {
            let name = display_name(server, status);
            let state = match status.state {
                ServerState::Online => match (status.players_online, status.players_max) {
                    (Some(n), Some(m)) => format!("{n}/{m} up"),
                    (Some(n), None) => format!("{n} up"),
                    _ => "up".to_string(),
                },
                ServerState::Starting => "starting".to_string(),
                ServerState::Offline => "down".to_string(),
                ServerState::Unknown => "checking…".to_string(),
            };
            format!("{} - {name}: {state}", server.game)
        })
        .collect()
}

/// Text for the bot's custom status, e.g. `4/5 servers online · 37 players`.
pub fn presence_text(statuses: &[ServerStatus]) -> String {
    let total = statuses.len();
    let up: Vec<&ServerStatus> = statuses.iter().filter(|s| s.state == ServerState::Online).collect();
    let players: u32 = up.iter().filter_map(|s| s.players_online).sum();
    if total == 1 {
        return match up.first() {
            Some(s) => match (s.players_online, s.players_max) {
                (Some(n), Some(m)) => format!("Online · {n}/{m} players"),
                _ => "Online".to_string(),
            },
            None => "Offline".to_string(),
        };
    }
    format!("{}/{total} servers online · {players} players", up.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lighthouse_core::{GameTime, Player};

    fn server() -> ServerConfig {
        ServerConfig {
            id: "pve".into(),
            game: GameKind::get(GameKind::SEVEN_DAYS_TO_DIE),
            name: Some("PvE Main".into()),
            host: "203.0.113.5".into(),
            game_port: 26900,
            query_port: None,
            show_address: true,
            show_players: true,
            alerts: false,
            native: None,
            log_dir: None,
        }
    }

    fn online() -> ServerStatus {
        let mut s = ServerStatus::new(ServerState::Online, 10_000);
        s.players_online = Some(2);
        s.players_max = Some(8);
        s.started_at_unix = Some(10_000 - 3 * 3600 - 120);
        s.game_time = Some(GameTime {
            day: 14,
            hour: 21,
            minute: 5,
        });
        s.blood_moon_in_days = Some(0);
        s.players = vec![
            Player {
                name: "a_b".into(),
                ping_ms: None,
            },
            Player {
                name: "c".into(),
                ping_ms: None,
            },
        ];
        s
    }

    #[test]
    fn comparable_ignores_footer_only() {
        let a = server_card(&server(), &online(), 10_000);
        let b = server_card(&server(), &online(), 10_030);
        assert_ne!(a, b);
        assert_eq!(comparable(&a), comparable(&b));
        let mut other = online();
        other.players_online = Some(5);
        assert_ne!(comparable(&a), comparable(&server_card(&server(), &other, 10_000)));
    }

    #[test]
    fn alert_and_presence_text() {
        let v = alert_message("PvE *Main*", false, Some(42), 100);
        let content = v["content"].as_str().unwrap();
        assert!(content.starts_with(r"<@&42> 🔴 **PvE \*Main\***"), "{content}");
        assert_eq!(v["allowed_mentions"]["roles"][0], "42");
        assert_eq!(presence_text(&[online()]), "Online · 2/8 players");
        assert_eq!(
            presence_text(&[online(), ServerStatus::offline(0)]),
            "1/2 servers online · 2 players"
        );
    }

    #[test]
    fn presence_lines_grouped_by_game() {
        let a = server(); // 7DTD "PvE Main"
        let mut pz = server();
        pz.id = "pz".into();
        pz.name = Some("Safehouse".into());
        pz.game = GameKind::get(GameKind::PROJECT_ZOMBOID);
        let mut b = server();
        b.id = "b".into();
        b.name = Some("Athens".into());
        let lines = presence_lines(&[(&pz, ServerStatus::offline(0)), (&a, online()), (&b, online())]);
        assert_eq!(
            lines,
            [
                "7 Days to Die - PvE Main: 2/8 up",
                "7 Days to Die - Athens: 2/8 up",
                "Project Zomboid - Safehouse: down"
            ]
        );
    }

    #[test]
    fn uptime_formatting() {
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(3 * 3600 + 120), "3h 2m");
        assert_eq!(format_uptime(2 * 86_400 + 5 * 3600), "2d 5h");
    }

    #[test]
    fn card_has_v2_flag_and_fields() {
        let v = server_card(&server(), &online(), 10_000);
        assert_eq!(v["flags"], IS_COMPONENTS_V2);
        let container = &v["components"][0];
        assert_eq!(container["accent_color"], COLOR_ONLINE);
        let body = container["components"][1]["content"].as_str().unwrap();
        assert!(body.contains("2/8 players"), "{body}");
        assert!(body.contains("up 3h 2m"), "{body}");
        assert!(body.contains("Blood moon tonight"), "{body}");
        assert!(body.contains(r"a\_b, c"), "{body}");
        assert!(body.contains("`203.0.113.5:26900`"), "{body}");
    }

    #[test]
    fn hidden_address_and_offline() {
        let mut cfg = server();
        cfg.show_address = false;
        let v = server_card(&cfg, &ServerStatus::offline(0), 0);
        let body = v["components"][0]["components"][1]["content"].as_str().unwrap();
        assert!(body.contains("Offline"));
        assert!(!body.contains("203.0.113.5"));
    }

    #[test]
    fn compact_board_groups_by_game() {
        let a = server();
        let mut b = server();
        b.id = "pz".into();
        b.game = GameKind::get(GameKind::PROJECT_ZOMBOID);
        let (sa, sb) = (online(), ServerStatus::offline(0));
        let v = compact_board(&[(&a, &sa), (&b, &sb)], 10_000);
        let comps = v["components"].as_array().unwrap();
        assert_eq!(comps.len(), 3); // two game containers + footer
        assert_eq!(comps[0]["accent_color"], COLOR_ONLINE);
        assert_eq!(comps[1]["accent_color"], COLOR_OFFLINE);
    }
}
