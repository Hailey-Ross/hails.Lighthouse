//! Population reports: players over time, peaks and averages, player-hours, how often servers are
//! full, and when people play (weekday × hour, in the viewer's time zone). Exportable as CSV.

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use lighthouse_core::{ServerConfig, ServerState};
use lighthouse_store::Sample;
use maud::{Markup, html};
use serde::Deserialize;

use super::server_name;
use crate::auth::CurrentUser;
use crate::charts::{self, Series};
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_utc, now_unix};
use crate::{AppState, layout};

const VIEW: &str = "reports.view";

pub struct Reports;

impl Module for Reports {
    fn id(&self) -> &'static str {
        "reports"
    }
    fn title(&self) -> &'static str {
        "Reports"
    }
    fn icon(&self) -> &'static str {
        "M4 20V11 M10 20V4 M16 20v-7 M21 20H3"
    }
    fn view_permission(&self) -> &'static str {
        VIEW
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: VIEW,
            label: "See population reports and export them",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new().route("/", get(page)).route("/export.csv", get(export))
    }
}

// ---------- ranges ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Range {
    Day,
    Week,
    Month,
}

impl Range {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("24h") => Range::Day,
            Some("30d") => Range::Month,
            _ => Range::Week,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Range::Day => "24h",
            Range::Week => "7d",
            Range::Month => "30d",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Range::Day => "Last 24 hours",
            Range::Week => "Last 7 days",
            Range::Month => "Last 30 days",
        }
    }
    fn secs(self) -> i64 {
        match self {
            Range::Day => 86_400,
            Range::Week => 7 * 86_400,
            Range::Month => 30 * 86_400,
        }
    }
    /// Chart bucket size.
    fn bucket(self) -> i64 {
        match self {
            Range::Day => 900,
            Range::Week => 2 * 3600,
            Range::Month => 6 * 3600,
        }
    }
}

// ---------- calculations ----------

/// One chart bucket: (average players, most players), or None when there is no data.
pub(crate) type Bucket = Option<(f64, u32)>;

/// Per-bucket (average, maximum) players over `[start, start + n*step)`, online samples only.
pub(crate) fn buckets(samples: &[Sample], start: i64, step: i64, n: usize) -> Vec<Bucket> {
    let mut acc = vec![(0u64, 0u32, 0u32); n]; // (sum, count, max)
    for s in samples
        .iter()
        .filter(|s| s.state == ServerState::Online && s.at >= start)
    {
        let i = ((s.at - start) / step) as usize;
        if let (Some(slot), Some(p)) = (acc.get_mut(i), s.players) {
            slot.0 += u64::from(p);
            slot.1 += 1;
            slot.2 = slot.2.max(p);
        }
    }
    acc.into_iter()
        .map(|(sum, c, max)| (c > 0).then(|| (sum as f64 / f64::from(c), max)))
        .collect()
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Stats {
    pub peak: Option<(u32, i64)>,
    pub average: Option<f64>,
    pub player_hours: f64,
    pub full_share: Option<f64>,
    pub uptime: Option<f64>,
}

/// Summary for one server since `since`. Each online sample counts until the next sample, but at
/// most `cap` seconds (so gaps in history don't inflate player-hours).
pub(crate) fn stats(samples: &[Sample], since: i64, now: i64, cap: i64) -> Stats {
    let window: Vec<&Sample> = samples.iter().filter(|s| s.at >= since).collect();
    let mut st = Stats::default();
    let (mut sum, mut count, mut full, mut up, mut known) = (0u64, 0u32, 0u32, 0u32, 0u32);
    for (i, s) in window.iter().enumerate() {
        match s.state {
            ServerState::Online => {
                up += 1;
                known += 1;
                if let Some(p) = s.players {
                    sum += u64::from(p);
                    count += 1;
                    if st.peak.is_none_or(|(best, _)| p > best) {
                        st.peak = Some((p, s.at));
                    }
                    if s.max_players.is_some_and(|m| m > 0 && p >= m) {
                        full += 1;
                    }
                    let next = window.get(i + 1).map_or(now, |n| n.at);
                    st.player_hours += f64::from(p) * ((next - s.at).clamp(0, cap) as f64) / 3600.0;
                }
            }
            ServerState::Offline => known += 1,
            _ => {}
        }
    }
    st.average = (count > 0).then(|| sum as f64 / f64::from(count));
    st.full_share = (count > 0).then(|| f64::from(full) * 100.0 / f64::from(count));
    st.uptime = (known > 0).then(|| f64::from(up) * 100.0 / f64::from(known));
    st
}

/// Average players by weekday × hour (viewer's time zone), summed across servers: each server's
/// hourly average is added to its weekday/hour slot, then each slot is averaged over the weeks seen.
pub(crate) fn weekly_pattern(all: &[&[Sample]], since: i64, tz: i64) -> [[Option<f64>; 24]; 7] {
    use std::collections::BTreeMap;
    // hour start -> total of per-server hourly averages
    let mut hours: BTreeMap<i64, f64> = BTreeMap::new();
    for samples in all {
        let mut per_hour: BTreeMap<i64, (u64, u32)> = BTreeMap::new();
        for s in samples
            .iter()
            .filter(|s| s.at >= since && s.state == ServerState::Online)
        {
            if let Some(p) = s.players {
                let e = per_hour.entry(s.at.div_euclid(3600) * 3600).or_default();
                e.0 += u64::from(p);
                e.1 += 1;
            }
        }
        for (h, (sum, c)) in per_hour {
            *hours.entry(h).or_default() += sum as f64 / f64::from(c);
        }
    }
    let mut acc = [[(0.0f64, 0u32); 24]; 7];
    for (h, v) in hours {
        let (d, hr) = charts::weekday_hour(h, tz);
        acc[d][hr].0 += v;
        acc[d][hr].1 += 1;
    }
    acc.map(|row| row.map(|(sum, c)| (c > 0).then(|| sum / f64::from(c))))
}

// ---------- page ----------

#[derive(Deserialize)]
struct ReportQuery {
    range: Option<String>,
    server: Option<String>,
}

struct Loaded<'a> {
    server: &'a ServerConfig,
    name: String,
    samples: Vec<Sample>,
}

fn load<'a>(state: &'a AppState, user: &lighthouse_store::User, q: &ReportQuery, since: i64) -> Vec<Loaded<'a>> {
    state
        .visible_servers(user)
        .into_iter()
        .filter(|s| q.server.as_deref().is_none_or(|id| id == "all" || id == s.id))
        .map(|server| Loaded {
            server,
            name: server_name(server, state.ctx.monitor.status(&server.id).as_ref()),
            samples: state.ctx.store.history(&server.id, since).unwrap_or_default(),
        })
        .collect()
}

fn fmt_num(v: Option<f64>, digits: usize) -> String {
    v.map_or("—".to_string(), |v| format!("{v:.digits$}"))
}

async fn page(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Query(q): Query<ReportQuery>,
) -> Markup {
    let now = now_unix();
    let tz = charts::tz_offset(&headers);
    let range = Range::parse(q.range.as_deref());
    let since = now - range.secs();
    let step = range.bucket();
    let n = (range.secs() / step) as usize;
    let start = now - n as i64 * step;
    let cap = (state.ctx.config.poll_interval_secs as i64 * 3).max(60);
    let selected = q.server.clone().unwrap_or_else(|| "all".into());
    let visible = state.visible_servers(&user);
    let loaded = load(&state, &user, &q, since);

    let per_server: Vec<(&Loaded, Stats, Vec<Bucket>)> = loaded
        .iter()
        .map(|l| {
            (
                l,
                stats(&l.samples, since, now, cap),
                buckets(&l.samples, start, step, n),
            )
        })
        .collect();

    // Combined: sum of each server's bucket average / bucket max.
    let combined_avg: Vec<Option<f64>> = (0..n)
        .map(|i| {
            let vals: Vec<f64> = per_server.iter().filter_map(|(_, _, b)| b[i].map(|(a, _)| a)).collect();
            (!vals.is_empty()).then(|| vals.iter().sum())
        })
        .collect();
    let combined_peak = (0..n)
        .filter_map(|i| {
            let vals: Vec<u32> = per_server.iter().filter_map(|(_, _, b)| b[i].map(|(_, m)| m)).collect();
            (!vals.is_empty()).then(|| (vals.iter().sum::<u32>(), start + i as i64 * step))
        })
        .max_by_key(|(v, _)| *v);
    let total_hours: f64 = per_server.iter().map(|(_, s, _)| s.player_hours).sum();
    let avg_total = {
        let vals: Vec<f64> = combined_avg.iter().flatten().copied().collect();
        (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64)
    };
    let busiest = per_server
        .iter()
        .filter(|(_, s, _)| s.player_hours > 0.0)
        .max_by(|a, b| a.1.player_hours.total_cmp(&b.1.player_hours))
        .map(|(l, _, _)| l.name.clone());
    let pattern_sources: Vec<&[Sample]> = loaded.iter().map(|l| l.samples.as_slice()).collect();
    let pattern = weekly_pattern(&pattern_sources, since, tz);
    let busiest_hour = pattern
        .iter()
        .enumerate()
        .flat_map(|(d, row)| row.iter().enumerate().filter_map(move |(h, v)| v.map(|v| (v, d, h))))
        .max_by(|a, b| a.0.total_cmp(&b.0));

    let series: Vec<Series> = if per_server.len() == 1 || selected != "all" {
        per_server
            .iter()
            .enumerate()
            .map(|(i, (l, _, b))| Series {
                name: l.name.clone(),
                color: i,
                points: b.iter().map(|x| x.map(|(a, _)| a)).collect(),
            })
            .collect()
    } else {
        let mut v: Vec<Series> = per_server
            .iter()
            .enumerate()
            .map(|(i, (l, _, b))| Series {
                name: l.name.clone(),
                color: i + 1,
                points: b.iter().map(|x| x.map(|(a, _)| a)).collect(),
            })
            .collect();
        v.insert(
            0,
            Series {
                name: "All servers".into(),
                color: 0,
                points: combined_avg.clone(),
            },
        );
        v
    };
    let labels: Vec<String> = (0..5)
        .map(|k| charts::local_label(start + (n as i64 * step) * k / 4, tz, range != Range::Day))
        .collect();
    let has_data = combined_avg.iter().any(Option::is_some);
    let days = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];

    layout::page(
        &state,
        &user,
        "reports",
        "Reports",
        html! {
            form.filters method="get" action="/reports" {
                div.tabs role="group" aria-label="Time range" {
                    @for r in [Range::Day, Range::Week, Range::Month] {
                        a href={ "/reports?range=" (r.key()) "&server=" (selected) }
                          class=[(r == range).then_some("active")] { (r.label()) }
                    }
                }
                input type="hidden" name="range" value=(range.key());
                select name="server" onchange="this.form.submit()" aria-label="Server" {
                    option value="all" selected[selected == "all"] { "All servers" }
                    @for s in &visible {
                        option value=(s.id) selected[selected == s.id] { (server_name(s, state.ctx.monitor.status(&s.id).as_ref())) }
                    }
                }
                noscript { button type="submit" { "Show" } }
                a.export href={ "/reports/export.csv?range=" (range.key()) "&server=" (selected) } { "Download CSV" }
            }

            @if !has_data {
                p.notice { "No player data for this period yet. Lighthouse records every check, so numbers appear within minutes." }
            } @else {
                div.stats.big {
                    div { b { @if let Some((p, _)) = combined_peak { (p) } @else { "—" } }
                          span { "Peak players"
                                 @if let Some((_, at)) = combined_peak { ", " (charts::local_label(at, tz, true)) } } }
                    div { b { (fmt_num(avg_total, 1)) } span { "Average players online" } }
                    div { b { (format!("{total_hours:.0}")) } span { "Player-hours played" } }
                    @if let Some((_, d, h)) = busiest_hour {
                        div { b { (format!("{h:02}:00")) } span { "Busiest hour, usually on " (days[d]) } }
                    }
                    @if selected == "all" { @if let Some(name) = &busiest { div { b.name { (name) } span { "Busiest server" } } } }
                }

                section.report-block {
                    h2 { "Players over time" }
                    (charts::line_chart(&series, &labels, "players"))
                }

                section.report-block {
                    h2 { "When people play" }
                    @if range == Range::Day {
                        p.muted { "Pick 7 or 30 days to see weekly patterns." }
                    } @else {
                        p.muted { "Average players by day and hour, in your time zone. Darker means busier." }
                        (charts::heatmap(&pattern, "players"))
                    }
                }

                section.report-block {
                    h2 { "By server" }
                    div.table-wrap { table {
                        thead { tr {
                            th { "Server" } th.num { "Peak" } th.num { "Average" } th.num { "Player-hours" }
                            th.num { "Time full" } th.num { "Uptime" }
                        } }
                        tbody { @for (l, s, _) in &per_server { tr {
                            td { (l.name) span.sub { (l.server.game) } }
                            td.num { @if let Some((p, at)) = s.peak { (p) span.sub { (charts::local_label(at, tz, true)) } } @else { "—" } }
                            td.num { (fmt_num(s.average, 1)) }
                            td.num { (format!("{:.0}", s.player_hours)) }
                            td.num { (fmt_num(s.full_share, 0)) @if s.full_share.is_some() { "%" } }
                            td.num { (fmt_num(s.uptime, 1)) @if s.uptime.is_some() { "%" } }
                        } } }
                    } }
                }
                p.muted { "Times are shown in your time zone. Player-hours are estimated from Lighthouse's regular checks." }
            }
        },
    )
}

async fn export(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ReportQuery>,
) -> Response {
    let now = now_unix();
    let range = Range::parse(q.range.as_deref());
    let since = now - range.secs();
    let n = (range.secs() / 3600) as usize;
    let start = now - n as i64 * 3600;
    let mut csv = String::from("hour_start_utc,server_id,server_name,game,avg_players,max_players\n");
    for l in load(&state, &user, &q, since) {
        for (i, b) in buckets(&l.samples, start, 3600, n).into_iter().enumerate() {
            if let Some((avg, max)) = b {
                let name = l.name.replace('"', "\"\"");
                csv.push_str(&format!(
                    "{},{},\"{name}\",{},{avg:.2},{max}\n",
                    fmt_utc(start + i as i64 * 3600).trim_end_matches(" UTC"),
                    l.server.id,
                    l.server.game.id()
                ));
            }
        }
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"lighthouse-players-{}.csv\"", range.key()),
            ),
        ],
        csv,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(at: i64, p: u32, max: u32) -> Sample {
        Sample {
            at,
            state: ServerState::Online,
            players: Some(p),
            max_players: Some(max),
        }
    }
    fn off(at: i64) -> Sample {
        Sample {
            at,
            state: ServerState::Offline,
            players: None,
            max_players: None,
        }
    }

    #[test]
    fn bucket_averages_and_peaks() {
        let s = [on(0, 2, 10), on(30, 4, 10), on(70, 6, 10), off(80)];
        let b = buckets(&s, 0, 60, 3);
        assert_eq!(b[0], Some((3.0, 4)));
        assert_eq!(b[1], Some((6.0, 6)));
        assert_eq!(b[2], None);
    }

    #[test]
    fn stats_summary() {
        // 4 players for an hour, then full (10/10) for an hour, then offline.
        let s = [on(0, 4, 10), on(3600, 10, 10), off(7200)];
        let st = stats(&s, 0, 7200, 3600);
        assert_eq!(st.peak, Some((10, 3600)));
        assert_eq!(st.average, Some(7.0));
        assert!((st.player_hours - 14.0).abs() < 1e-9);
        assert_eq!(st.full_share, Some(50.0));
        assert!((st.uptime.unwrap() - 66.666).abs() < 0.01);
    }

    #[test]
    fn player_hours_ignore_long_gaps() {
        let s = [on(0, 10, 10), on(10 * 3600, 10, 10)];
        let st = stats(&s, 0, 10 * 3600, 90);
        assert!(
            (st.player_hours - 10.0 * 90.0 / 3600.0).abs() < 1e-9,
            "{}",
            st.player_hours
        );
    }

    #[test]
    fn weekly_pattern_sums_servers() {
        // Thursday 1970-01-01 01:00 UTC: server A 2 players, server B 3 players.
        let a = [on(3600, 2, 10)];
        let b = [on(3700, 3, 10)];
        let p = weekly_pattern(&[&a, &b], 0, 0);
        assert_eq!(p[3][1], Some(5.0));
        assert_eq!(p[3][2], None);
    }
}
