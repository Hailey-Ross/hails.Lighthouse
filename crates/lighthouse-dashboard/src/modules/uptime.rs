//! Uptime history: availability over 24 h / 7 d / 30 d, a 24-hour timeline, outages and a
//! player-count chart, built from the status samples Lighthouse stores on every poll.

use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use lighthouse_core::{ServerState, ServerStatus};
use lighthouse_store::Sample;
use maud::{Markup, html};

use super::{server_name, state_badge};
use crate::auth::{CurrentUser, forbidden};
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_duration, fmt_utc, now_unix};
use crate::{AppState, layout};

const VIEW: &str = "uptime.view";
const DAY: i64 = 86_400;

pub struct Uptime;

impl Module for Uptime {
    fn id(&self) -> &'static str {
        "uptime"
    }
    fn title(&self) -> &'static str {
        "Uptime"
    }
    fn icon(&self) -> &'static str {
        "M3 12h4l2-6 4 12 2-6h6"
    }
    fn view_permission(&self) -> &'static str {
        VIEW
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: VIEW,
            label: "See uptime history and outages",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new().route("/", get(list)).route("/{id}", get(detail))
    }
}

// ---------- calculations ----------

/// Share of samples since `since` that were online. `None` when there are no samples.
pub(crate) fn availability(samples: &[Sample], since: i64) -> Option<f64> {
    let (mut up, mut total) = (0u32, 0u32);
    for s in samples.iter().filter(|s| s.at >= since) {
        match s.state {
            ServerState::Online => {
                up += 1;
                total += 1;
            }
            ServerState::Offline => total += 1,
            _ => {}
        }
    }
    (total > 0).then(|| f64::from(up) * 100.0 / f64::from(total))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    NoData,
    Up,
    Down,
    Mixed,
}

/// Split `[start, end)` into `n` equal slots and say what happened in each.
pub(crate) fn timeline(samples: &[Sample], start: i64, end: i64, n: usize) -> Vec<Slot> {
    let width = ((end - start) as f64 / n as f64).max(1.0);
    let mut slots = vec![(false, false); n];
    for s in samples.iter().filter(|s| s.at >= start && s.at < end) {
        let i = (((s.at - start) as f64) / width) as usize;
        if let Some(slot) = slots.get_mut(i.min(n - 1)) {
            match s.state {
                ServerState::Online => slot.0 = true,
                ServerState::Offline => slot.1 = true,
                _ => {}
            }
        }
    }
    slots
        .into_iter()
        .map(|(up, down)| match (up, down) {
            (false, false) => Slot::NoData,
            (true, false) => Slot::Up,
            (false, true) => Slot::Down,
            (true, true) => Slot::Mixed,
        })
        .collect()
}

/// Periods the server was offline: (first offline sample, first online sample after it).
pub(crate) fn outages(samples: &[Sample]) -> Vec<(i64, Option<i64>)> {
    let mut out = Vec::new();
    let mut down_since: Option<i64> = None;
    for s in samples {
        match (s.state, down_since) {
            (ServerState::Offline, None) => down_since = Some(s.at),
            (ServerState::Online, Some(start)) => {
                out.push((start, Some(s.at)));
                down_since = None;
            }
            _ => {}
        }
    }
    if let Some(start) = down_since {
        out.push((start, None));
    }
    out
}

/// SVG polyline points for players over `[start, end)`, in a 0..1000 × 0..100 box.
fn player_points(samples: &[Sample], start: i64, end: i64) -> (String, u32) {
    let peak = samples
        .iter()
        .filter_map(|s| s.max_players.or(s.players))
        .max()
        .unwrap_or(1)
        .max(1);
    let span = (end - start).max(1) as f64;
    let points = samples
        .iter()
        .filter(|s| s.at >= start && s.state == ServerState::Online)
        .filter_map(|s| s.players.map(|p| (s.at, p)))
        .map(|(at, p)| {
            let x = (at - start) as f64 / span * 1000.0;
            let y = 100.0 - f64::from(p) / f64::from(peak) * 95.0;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    (points, peak)
}

fn pct(v: Option<f64>) -> String {
    v.map_or("—".to_string(), |p| format!("{p:.2}%"))
}

fn timeline_bar(slots: &[Slot]) -> Markup {
    html! {
        div.bar title="Last 24 hours, oldest on the left" {
            @for s in slots {
                span class=(match s { Slot::Up => "up", Slot::Down => "down", Slot::Mixed => "mixed", Slot::NoData => "" }) {}
            }
        }
    }
}

// ---------- pages ----------

async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Markup {
    let now = now_unix();
    let rows: Vec<_> = state
        .visible_servers(&user)
        .into_iter()
        .map(|server| {
            let status = state
                .ctx
                .monitor
                .status(&server.id)
                .unwrap_or_else(|| ServerStatus::offline(now));
            let history = state.ctx.store.history(&server.id, now - 30 * DAY).unwrap_or_default();
            (server, status, history)
        })
        .collect();

    layout::page(
        &state,
        &user,
        "uptime",
        "Uptime",
        html! {
            @if rows.is_empty() {
                p.notice { "No servers to show." }
            } @else {
                div.table-wrap { table {
                    thead { tr {
                        th { "Server" } th { "Now" } th { "Up for" }
                        th.num { "24 h" } th.num { "7 days" } th.num { "30 days" } th { "Last 24 hours" }
                    } }
                    tbody {
                        @for (server, status, history) in &rows {
                            tr {
                                td { a href={ "/uptime/" (server.id) } { (server_name(server, Some(status))) }
                                     span.sub { (server.game) } }
                                td { (state_badge(status.state)) }
                                td { (status.uptime_secs(now).map_or("—".to_string(), |s| fmt_duration(s as i64))) }
                                td.num { (pct(availability(history, now - DAY))) }
                                td.num { (pct(availability(history, now - 7 * DAY))) }
                                td.num { (pct(availability(history, now - 30 * DAY))) }
                                td style="min-width:180px" { (timeline_bar(&timeline(history, now - DAY, now, 24))) }
                            }
                        }
                    }
                } }
                p.muted { "Uptime counts from when Lighthouse first saw a server online. History is kept for 30 days." }
            }
        },
    )
}

async fn detail(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<String>) -> Response {
    let Some(server) = state
        .ctx
        .config
        .servers
        .iter()
        .find(|s| s.id == id && user.sees_server(&s.id))
    else {
        return forbidden(&state, &user);
    };
    let now = now_unix();
    let status = state
        .ctx
        .monitor
        .status(&server.id)
        .unwrap_or_else(|| ServerStatus::offline(now));
    let history = state.ctx.store.history(&server.id, now - 30 * DAY).unwrap_or_default();
    let mut downs = outages(&history);
    downs.reverse();
    let (points, peak) = player_points(&history, now - DAY, now);
    let name = server_name(server, Some(&status));

    layout::page(
        &state,
        &user,
        "uptime",
        &name,
        html! {
            p.inline { (state_badge(status.state)) span.muted { (server.game) }
                @if let Some(up) = status.uptime_secs(now) { span.muted { "up for " (fmt_duration(up as i64)) } } }
            div.stats {
                div { b { (pct(availability(&history, now - DAY))) } span.muted { "last 24 hours" } }
                div { b { (pct(availability(&history, now - 7 * DAY))) } span.muted { "last 7 days" } }
                div { b { (pct(availability(&history, now - 30 * DAY))) } span.muted { "last 30 days" } }
                div { b { (downs.len()) } span.muted { "outages (30 days)" } }
            }
            h2 { "Last 24 hours" }
            (timeline_bar(&timeline(&history, now - DAY, now, 96)))
            div.bar-legend { span { "24 hours ago" } span { "Now" } }
            h2 { "Players, last 24 hours" span.muted { " (scale 0–" (peak) ")" } }
            @if points.is_empty() {
                p.muted { "No player data yet." }
            } @else {
                svg.chart viewBox="0 0 1000 100" preserveAspectRatio="none" { polyline points=(points) {} }
            }
            h2 { "Outages" }
            @if downs.is_empty() {
                p.muted { "No outages recorded in the last 30 days." }
            } @else {
                div.table-wrap { table {
                    thead { tr { th { "Went down" } th { "Came back" } th { "Lasted" } } }
                    tbody { @for (start, end) in downs.iter().take(50) { tr {
                        td { (fmt_utc(*start)) }
                        td { (end.map_or("still down".to_string(), fmt_utc)) }
                        td { (fmt_duration(end.unwrap_or(now) - start)) }
                    } } }
                } }
            }
            a.back href="/uptime" { "Back to all servers" }
        },
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at: i64, state: ServerState, players: Option<u32>) -> Sample {
        Sample {
            at,
            state,
            players,
            max_players: players.map(|_| 10),
        }
    }

    #[test]
    fn availability_counts_online_share() {
        let h = [
            s(0, ServerState::Online, Some(1)),
            s(10, ServerState::Offline, None),
            s(20, ServerState::Online, Some(2)),
            s(30, ServerState::Online, Some(2)),
        ];
        assert_eq!(availability(&h, 0), Some(75.0));
        assert_eq!(availability(&h, 15), Some(100.0));
        assert_eq!(availability(&[], 0), None);
    }

    #[test]
    fn timeline_slots() {
        let h = [
            s(0, ServerState::Online, None),
            s(5, ServerState::Offline, None),
            s(15, ServerState::Online, None),
        ];
        assert_eq!(timeline(&h, 0, 30, 3), [Slot::Mixed, Slot::Up, Slot::NoData]);
    }

    #[test]
    fn outage_periods() {
        let h = [
            s(0, ServerState::Online, None),
            s(10, ServerState::Offline, None),
            s(20, ServerState::Offline, None),
            s(30, ServerState::Online, None),
            s(40, ServerState::Offline, None),
        ];
        assert_eq!(outages(&h), [(10, Some(30)), (40, None)]);
    }
}
