//! Status posts as Discord Components V2 message bodies.
//!
//! Reference: <https://discord.com/developers/docs/components/reference>. A V2 message cannot also
//! carry `content` or `embeds`, so everything goes into components.
//!
//! Layout, per server:
//!
//! ```text
//! 🟢 **Athens PvE**
//! **4** / 30 playing   ▰▱▱▱▱▱▱▱▱▱   up 2h 14m
//! -# Day 591, 18:03 · Blood moon in 4 days · Join `203.0.113.10:26930`
//! ```

use lighthouse_core::{GameKind, ServerConfig, ServerState, ServerStatus};
use serde_json::{Value, json};

/// Message flag that switches a message to Components V2.
pub const IS_COMPONENTS_V2: u64 = 1 << 15;

const CONTAINER: u8 = 17;
const TEXT_DISPLAY: u8 = 10;
const SEPARATOR: u8 = 14;

/// Discord caps the text across all components of a message at 4000 characters; keep some room.
const TEXT_BUDGET: usize = 3800;
/// Discord allows 40 components per message (containers and their children all count).
const COMPONENT_BUDGET: usize = 38;
/// Room for the player list on a single-server card.
const PLAYER_LIST_BUDGET: usize = 1500;

pub const COLOR_ONLINE: u32 = 0x3C9B6C;
pub const COLOR_STARTING: u32 = 0xD9A441;
pub const COLOR_OFFLINE: u32 = 0xC95B5B;
pub const COLOR_UNKNOWN: u32 = 0x8FA0A2;

/// Board title when a bot doesn't set one.
pub const DEFAULT_TITLE: &str = "Server status";

fn state_color(state: ServerState) -> u32 {
    match state {
        ServerState::Online => COLOR_ONLINE,
        ServerState::Starting => COLOR_STARTING,
        ServerState::Offline => COLOR_OFFLINE,
        ServerState::Unknown => COLOR_UNKNOWN,
    }
}

fn dot(state: ServerState) -> &'static str {
    match state {
        ServerState::Online => "🟢",
        ServerState::Starting => "🟡",
        ServerState::Offline => "🔴",
        ServerState::Unknown => "⚪",
    }
}

/// `3d 4h`, `4h 12m`, `12m`, `~1m`. Short on purpose: it is refreshed on every update.
pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = secs % 86_400 / 3_600;
    let mins = secs % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        "~1m".to_string()
    }
}

/// A ten-step capacity bar, e.g. `▰▰▰▱▱▱▱▱▱▱`. Any players at all show at least one step.
pub fn capacity_bar(players: u32, max: u32) -> String {
    const STEPS: u32 = 10;
    let filled = match (players.min(max) * STEPS + max / 2).checked_div(max) {
        Some(f) if players > 0 => f.max(1),
        _ => 0,
    };
    "▰".repeat(filled as usize) + &"▱".repeat((STEPS - filled) as usize)
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

/// How much detail a server block carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Detail {
    /// Single-server card: everything, including version and player names.
    Full,
    /// Board: game time, blood moon and join address.
    Board,
    /// Board that ran out of room: name, players and uptime only.
    Brief,
}

/// The server's lines, without its name line.
fn server_body(server: &ServerConfig, status: &ServerStatus, now_unix: i64, detail: Detail) -> Vec<String> {
    let mut lines = Vec::new();
    match status.state {
        ServerState::Online | ServerState::Starting => {
            let mut main = Vec::new();
            match (status.players_online, status.players_max) {
                (Some(n), Some(m)) => main.push(format!("**{n}** / {m} playing   {}", capacity_bar(n, m))),
                (Some(n), None) => main.push(format!("**{n}** playing")),
                _ if status.state == ServerState::Starting => main.push("Starting up".to_string()),
                _ => main.push("Online".to_string()),
            }
            if let Some(up) = status.uptime_secs(now_unix) {
                main.push(format!("up {}", format_uptime(up)));
            }
            lines.push(main.join("   "));

            if detail != Detail::Brief {
                let mut sub = Vec::new();
                if detail == Detail::Full
                    && let Some(map) = &status.map
                {
                    sub.push(escape(map));
                }
                if let Some(t) = status.game_time {
                    sub.push(format!("Day {}, {:02}:{:02}", t.day, t.hour, t.minute));
                }
                if let Some(days) = status.blood_moon_in_days {
                    sub.push(match days {
                        0 => "🩸 Blood moon tonight".to_string(),
                        1 => "Blood moon tomorrow".to_string(),
                        n => format!("Blood moon in {n} days"),
                    });
                }
                if detail == Detail::Full
                    && let Some(v) = &status.version
                {
                    sub.push(format!("Version {}", escape(v)));
                }
                if server.show_address {
                    sub.push(format!("Join `{}:{}`", server.host, server.game_port));
                }
                if !sub.is_empty() {
                    lines.push(format!("-# {}", sub.join(" · ")));
                }
                if detail == Detail::Full && server.show_players && !status.players.is_empty() {
                    lines.push(player_list(status));
                }
            }
        }
        ServerState::Offline => {
            lines.push(format!(
                "Offline · not answering since <t:{}:R>",
                status.observed_at_unix
            ));
            if server.show_address && detail != Detail::Brief {
                lines.push(format!("-# Join `{}:{}`", server.host, server.game_port));
            }
        }
        ServerState::Unknown => lines.push("Checking…".to_string()),
    }
    lines
}

fn name_line(server: &ServerConfig, status: &ServerStatus) -> String {
    let lock = if status.password_protected == Some(true) {
        "  🔒"
    } else {
        ""
    };
    format!(
        "{} **{}**{lock}",
        dot(status.state),
        escape(&display_name(server, status))
    )
}

fn player_list(status: &ServerStatus) -> String {
    let mut out = String::from("-# Playing: ");
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

fn spacer() -> Value {
    json!({ "type": SEPARATOR, "divider": false, "spacing": 1 })
}

fn updated_footer(now_unix: i64) -> Value {
    // <t:..:R> renders as "2 minutes ago" in each viewer's own locale.
    text(format!("-# Updated <t:{now_unix}:R>"))
}

/// `3 of 3 online · 4 playing`
fn summary(statuses: &[&ServerStatus]) -> String {
    let up: Vec<&&ServerStatus> = statuses.iter().filter(|s| s.state == ServerState::Online).collect();
    let players: u32 = up.iter().filter_map(|s| s.players_online).sum();
    let playing = match players {
        0 => "nobody playing".to_string(),
        n => format!("{n} playing"),
    };
    format!("{} of {} online · {playing}", up.len(), statuses.len())
}

/// Text that identifies a post when Lighthouse has lost track of it (see `Bot::find_own_post`):
/// a server card carries the server's name in bold; a board starts with a `## ` title.
pub fn card_marker(server: &ServerConfig, status: &ServerStatus) -> String {
    format!("**{}**", escape(&display_name(server, status)))
}

pub const BOARD_MARKER: &str = "## ";

/// One message per server: the one-bot-per-game layout.
pub fn server_card(server: &ServerConfig, status: &ServerStatus, now_unix: i64) -> Value {
    let title = format!("### {}", name_line(server, status));
    let body = server_body(server, status, now_unix, Detail::Full).join("\n");
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

/// One board for many servers, grouped by game: the one-bot-for-all-games layout.
pub fn compact_board(entries: &[(&ServerConfig, &ServerStatus)], now_unix: i64, title: &str) -> Value {
    for detail in [Detail::Board, Detail::Brief] {
        let board = build_board(entries, now_unix, title, detail, 0);
        if text_len(&board) <= TEXT_BUDGET {
            return board;
        }
    }
    // Still too long: show as many servers as fit, and say how many are left out.
    let mut shown = entries.len();
    while shown > 1 {
        shown -= 1;
        let board = build_board(&entries[..shown], now_unix, title, Detail::Brief, entries.len() - shown);
        if text_len(&board) <= TEXT_BUDGET {
            return board;
        }
    }
    build_board(&entries[..1], now_unix, title, Detail::Brief, entries.len() - 1)
}

fn build_board(
    entries: &[(&ServerConfig, &ServerStatus)],
    now_unix: i64,
    title: &str,
    detail: Detail,
    hidden: usize,
) -> Value {
    let mut games: Vec<GameKind> = entries.iter().map(|(s, _)| s.game).collect();
    games.sort();
    games.dedup();

    let all: Vec<&ServerStatus> = entries.iter().map(|(_, st)| *st).collect();
    let mut components = vec![text(format!("## {}\n-# {}", escape(title), summary(&all)))];

    // One text block per server reads best; past the component limit, a game's servers share one.
    let separate = entries.len() * 2 + games.len() * 2 + 2 <= COMPONENT_BUDGET;
    for game in games {
        let in_game: Vec<&(&ServerConfig, &ServerStatus)> = entries.iter().filter(|(s, _)| s.game == game).collect();
        let statuses: Vec<&ServerStatus> = in_game.iter().map(|(_, st)| *st).collect();
        let up = statuses.iter().filter(|s| s.state == ServerState::Online).count();
        let color = if up == statuses.len() {
            COLOR_ONLINE
        } else if up == 0 {
            COLOR_OFFLINE
        } else {
            COLOR_STARTING
        };
        let mut inner = vec![text(format!("### {game}\n-# {}", summary(&statuses)))];
        let blocks: Vec<String> = in_game
            .iter()
            .map(|(s, st)| {
                let mut lines = vec![name_line(s, st)];
                lines.extend(server_body(s, st, now_unix, detail));
                lines.join("\n")
            })
            .collect();
        if separate {
            for b in blocks {
                inner.push(spacer());
                inner.push(text(b));
            }
        } else {
            inner.push(text(blocks.join("\n\n")));
        }
        components.push(json!({ "type": CONTAINER, "accent_color": color, "components": inner }));
    }
    if hidden > 0 {
        components.push(text(format!(
            "-# +{hidden} more servers not shown (Discord allows 4000 characters per post)"
        )));
    }
    components.push(updated_footer(now_unix));
    json!({ "flags": IS_COMPONENTS_V2, "components": components })
}

/// Total characters of text across a message's components (Discord counts characters, not bytes).
fn text_len(v: &Value) -> usize {
    match v {
        Value::Object(map) => {
            map.get("content")
                .and_then(Value::as_str)
                .map_or(0, |t| t.chars().count())
                + map.values().map(text_len).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(text_len).sum(),
        _ => 0,
    }
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
    fn capacity_bars() {
        assert_eq!(capacity_bar(0, 30), "▱▱▱▱▱▱▱▱▱▱");
        assert_eq!(capacity_bar(1, 30), "▰▱▱▱▱▱▱▱▱▱", "any players show one step");
        assert_eq!(capacity_bar(15, 30), "▰▰▰▰▰▱▱▱▱▱");
        assert_eq!(capacity_bar(40, 30), "▰▰▰▰▰▰▰▰▰▰");
        assert_eq!(capacity_bar(3, 0), "▱▱▱▱▱▱▱▱▱▱");
    }

    #[test]
    fn big_boards_stay_inside_discord_limits() {
        let servers: Vec<ServerConfig> = (0..40)
            .map(|i| {
                let mut s = server();
                s.id = format!("s{i}");
                s.name = Some(format!("A rather long server name number {i}"));
                s
            })
            .collect();
        let status = online();
        let entries: Vec<(&ServerConfig, &ServerStatus)> = servers.iter().map(|s| (s, &status)).collect();
        let v = compact_board(&entries, 10_000, DEFAULT_TITLE);
        fn count(v: &Value) -> usize {
            v.get("components")
                .and_then(Value::as_array)
                .map_or(0, |c| c.len() + c.iter().map(count).sum::<usize>())
        }
        assert!(count(&v) <= 40, "{} components", count(&v));
        assert!(text_len(&v) <= 4000, "{} characters", text_len(&v));
    }

    #[test]
    fn uptime_formatting() {
        assert_eq!(format_uptime(59), "~1m");
        assert_eq!(format_uptime(61), "1m");
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
        assert!(body.contains("**2** / 8 playing   ▰▰▰▱▱▱▱▱▱▱"), "{body}");
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
        let v = compact_board(&[(&a, &sa), (&b, &sb)], 10_000, "Our servers");
        let comps = v["components"].as_array().unwrap();
        assert_eq!(comps.len(), 4); // title, two game containers, footer
        assert_eq!(comps[0]["content"], "## Our servers\n-# 1 of 2 online · 2 playing");
        assert_eq!(comps[1]["accent_color"], COLOR_ONLINE);
        assert_eq!(comps[2]["accent_color"], COLOR_OFFLINE);
        let athens = comps[1]["components"][2]["content"].as_str().unwrap();
        assert!(athens.starts_with("🟢 **PvE Main**\n**2** / 8 playing"), "{athens}");
        assert!(
            athens.contains("-# Day 14, 21:05 · 🩸 Blood moon tonight · Join `203.0.113.5:26900`"),
            "{athens}"
        );
        let pz = comps[2]["components"][2]["content"].as_str().unwrap();
        assert!(pz.contains("Offline · not answering since <t:0:R>"), "{pz}");
    }
}
