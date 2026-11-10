//! Servers at a glance: one card per server, grouped by game, refreshed every 30 seconds.

use std::collections::BTreeMap;

use axum::Router;
use axum::extract::State;
use axum::routing::get;
use lighthouse_core::{GameKind, ServerConfig, ServerState, ServerStatus};
use maud::{Markup, html};

use super::{lamp, server_name, state_class, state_label};
use crate::auth::CurrentUser;
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_ago, fmt_duration, now_unix};
use crate::{AppState, layout};

const VIEW: &str = "servers.view";

pub struct Overview;

impl Module for Overview {
    fn id(&self) -> &'static str {
        "overview"
    }
    fn title(&self) -> &'static str {
        "Servers"
    }
    fn icon(&self) -> &'static str {
        "M4 5h16v5H4z M4 14h16v5H4z M7.5 7.5h.01 M7.5 16.5h.01"
    }
    fn view_permission(&self) -> &'static str {
        VIEW
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: VIEW,
            label: "See the server overview",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new().route("/", get(page))
    }
}

fn card(server: &ServerConfig, status: &ServerStatus, now: i64, link_uptime: bool, spark: &[Option<f64>]) -> Markup {
    let online = status.state == ServerState::Online;
    let name = server_name(server, Some(status));
    html! {
        article class={ "server " (state_class(status.state)) } {
            header {
                (lamp(status.state))
                h3 {
                    @if link_uptime { a href={ "/uptime/" (server.id) } { (name) } } @else { (name) }
                }
            }
            @if online {
                @if let (Some(n), Some(m)) = (status.players_online, status.players_max) {
                    p.players { (n) small { " of " (m) " playing" } }
                    @let pct = if m > 0 { (f64::from(n) / f64::from(m) * 100.0).min(100.0) } else { 0.0 };
                    div.capacity role="meter" aria-valuemin="0" aria-valuemax=(m) aria-valuenow=(n)
                        aria-label="Players compared to server capacity" {
                        span style={ "width:" (format!("{pct:.0}")) "%" } {}
                    }
                }
            } @else {
                p.muted { (state_label(status.state)) }
            }
            dl {
                @if online {
                    @if let Some(up) = status.uptime_secs(now) { dt { "Up for" } dd { (fmt_duration(up as i64)) } }
                    @if let Some(map) = &status.map { dt { "Map" } dd { (map) } }
                    @if let Some(t) = status.game_time {
                        dt { "In game" } dd { "Day " (t.day) ", " (format!("{:02}:{:02}", t.hour, t.minute)) }
                    }
                    @if let Some(days) = status.blood_moon_in_days {
                        dt { "Blood moon" }
                        dd { @match days { 0 => "Tonight", 1 => "Tomorrow", n => { "In " (n) " days" } } }
                    }
                    @if let Some(v) = &status.version { dt { "Version" } dd { (v) } }
                }
                dt { "Address" } dd { (server.host) ":" (server.game_port) }
            }
            @if spark.iter().filter(|p| p.is_some()).count() > 1 {
                div.spark-wrap title="Players over the last 24 hours" {
                    (crate::charts::sparkline(spark, f64::from(status.players_max.unwrap_or(0)).max(1.0)))
                }
            }
            footer {
                "Checked " (fmt_ago(now, status.observed_at_unix))
                @if let (true, Some(p)) = (online, status.ping_ms) { ", " (p) " ms away" }
            }
        }
    }
}

/// "4 of 5 servers are up" / "All 3 servers are up" / "Every server is down".
fn headline(online: usize, total: usize) -> String {
    match (online, total) {
        (_, 1) if online == 1 => "Your server is up".to_string(),
        (0, 1) => "Your server is down".to_string(),
        (0, _) => "Every server is down".to_string(),
        (o, t) if o == t => format!("All {t} servers are up"),
        (o, t) => format!("{o} of {t} servers are up"),
    }
}

/// A greeting for the viewer's local hour.
fn greeting(hour: usize) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=21 => "Good evening",
        _ => "Quiet night",
    }
}

async fn page(State(state): State<AppState>, CurrentUser(user): CurrentUser, headers: axum::http::HeaderMap) -> Markup {
    let (_, local_hour) = crate::charts::weekday_hour(now_unix(), crate::charts::tz_offset(&headers));
    let now = now_unix();
    let servers = state.visible_servers(&user);
    let statuses: Vec<(&ServerConfig, ServerStatus)> = servers
        .iter()
        .map(|s| {
            (
                *s,
                state
                    .ctx
                    .monitor
                    .status(&s.id)
                    .unwrap_or_else(|| ServerStatus::offline(now)),
            )
        })
        .collect();
    let online = statuses
        .iter()
        .filter(|(_, st)| st.state == ServerState::Online)
        .count();
    let players: u32 = statuses
        .iter()
        .filter(|(_, st)| st.state == ServerState::Online)
        .filter_map(|(_, st)| st.players_online)
        .sum();
    let mut by_game: BTreeMap<GameKind, Vec<&(&ServerConfig, ServerStatus)>> = BTreeMap::new();
    for entry in &statuses {
        by_game.entry(entry.0.game).or_default().push(entry);
    }
    let link_uptime = user.can("uptime.view");
    let sparks: std::collections::HashMap<&str, Vec<Option<f64>>> = servers
        .iter()
        .map(|s| {
            let history = state.ctx.store.history(&s.id, now - 86_400).unwrap_or_default();
            let points = super::reports::buckets(&history, now - 86_400, 1800, 48)
                .into_iter()
                .map(|b| b.map(|(avg, _)| avg))
                .collect();
            (s.id.as_str(), points)
        })
        .collect();

    layout::page_full(
        &state,
        &user,
        "overview",
        "Servers",
        html! { meta http-equiv="refresh" content="30"; },
        html! {
            @if servers.is_empty() {
                h1 { "Servers" }
                p.notice { "There are no servers to show yet. Add them with " code { "lighthouse setup" } " or in the config file." }
            } @else {
                section.hero {
                    div.stars aria-hidden="true" { @for _ in 0..12 { i {} } }
                    div.beam aria-hidden="true" {}
                    p.greeting { (greeting(local_hour)) ", " (user.username) }
                h1.lede { (headline(online, statuses.len())) }
                p.hero-sub {
                    @match players { 0 => "Nobody is playing right now.", 1 => "1 person is playing right now.", n => { (n) " people are playing right now." } }
                }
                }
                @for (game, entries) in &by_game {
                    section.game-group {
                        h2 { (game) small { (entries.len()) @if entries.len() == 1 { " server" } @else { " servers" } } }
                        div.grid {
                            @for (server, status) in entries.iter().map(|e| (e.0, &e.1)) {
                                (card(server, status, now, link_uptime, sparks.get(server.id.as_str()).map_or(&[][..], Vec::as_slice)))
                            }
                        }
                    }
                }
                p.muted.refresh-note { "This page refreshes every 30 seconds." }
            }
        },
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::headline;

    #[test]
    fn greetings_follow_the_clock() {
        assert_eq!(super::greeting(7), "Good morning");
        assert_eq!(super::greeting(13), "Good afternoon");
        assert_eq!(super::greeting(20), "Good evening");
        assert_eq!(super::greeting(2), "Quiet night");
    }

    #[test]
    fn friendly_headlines() {
        assert_eq!(headline(1, 1), "Your server is up");
        assert_eq!(headline(0, 1), "Your server is down");
        assert_eq!(headline(3, 3), "All 3 servers are up");
        assert_eq!(headline(4, 5), "4 of 5 servers are up");
        assert_eq!(headline(0, 5), "Every server is down");
    }
}
