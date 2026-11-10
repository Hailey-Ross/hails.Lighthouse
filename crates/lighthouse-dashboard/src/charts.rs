//! Small server-rendered charts (SVG + HTML). No JavaScript.
//!
//! Lines are drawn in a 1000 × H box that stretches to the page width; labels are HTML so they
//! never stretch with it.

use axum::http::HeaderMap;
use maud::{Markup, html};

/// Number of colours in the chart palette (`--series-1` … in the stylesheet).
pub const PALETTE: usize = 6;

pub struct Series {
    pub name: String,
    /// Index into the palette (wraps around).
    pub color: usize,
    /// One value per bucket; `None` = no data (the line breaks).
    pub points: Vec<Option<f64>>,
}

/// A round number at or above `v`, for the top of an axis.
pub fn nice_max(v: f64) -> f64 {
    if v <= 0.0 {
        return 1.0;
    }
    let mag = 10f64.powf(v.log10().floor());
    for step in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if step * mag >= v {
            return step * mag;
        }
    }
    10.0 * mag
}

fn segments(points: &[Option<f64>], top: f64, height: f64) -> Vec<String> {
    let n = points.len().max(2) - 1;
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        match p {
            Some(v) => {
                let x = i as f64 / n as f64 * 1000.0;
                let y = height - (v / top).clamp(0.0, 1.0) * (height - 6.0) - 3.0;
                cur.push(format!("{x:.1},{y:.1}"));
            }
            None if !cur.is_empty() => out.push(std::mem::take(&mut cur).join(" ")),
            None => {}
        }
    }
    if !cur.is_empty() {
        out.push(cur.join(" "));
    }
    out
}

/// Multi-series line chart with a legend, y-axis scale and x labels.
pub fn line_chart(series: &[Series], x_labels: &[String], unit: &str) -> Markup {
    const H: f64 = 220.0;
    let max = series
        .iter()
        .flat_map(|s| s.points.iter().flatten())
        .fold(0.0f64, |a, b| a.max(*b));
    let top = nice_max(max);
    html! {
        figure.chart {
            div.chart-plot {
                div.y-axis { span { (top) } span { (top / 2.0) } span { "0" } }
                svg viewBox={ "0 0 1000 " (H) } preserveAspectRatio="none" role="img" aria-label={ "Chart of " (unit) } {
                    @for f in [0.0, 0.5, 1.0] {
                        line.grid x1="0" x2="1000" y1=(H - f * (H - 6.0) - 3.0) y2=(H - f * (H - 6.0) - 3.0) {}
                    }
                    @for s in series {
                        @for seg in segments(&s.points, top, H) {
                            @if seg.contains(' ') {
                                polyline class={ "s" (s.color % PALETTE + 1) } points=(seg) {}
                            } @else {
                                // A single point: draw a short dash so it's visible.
                                @let (x, y) = seg.split_once(',').unwrap_or(("0", "0"));
                                line class={ "s" (s.color % PALETTE + 1) } x1=(x) x2=(x) y1=(y) y2=(y) {}
                            }
                        }
                    }
                }
            }
            div.x-axis { @for l in x_labels { span { (l) } } }
            @if series.len() > 1 {
                figcaption.legend {
                    @for s in series { span { i class={ "swatch s" (s.color % PALETTE + 1) } {} (s.name) } }
                }
            }
        }
    }
}

/// Tiny line without labels, for cards. `max` is the top of the scale.
pub fn sparkline(points: &[Option<f64>], max: f64) -> Markup {
    let top = if max > 0.0 { max } else { 1.0 };
    html! {
        svg.spark viewBox="0 0 1000 60" preserveAspectRatio="none" aria-hidden="true" {
            @for seg in segments(points, top, 60.0) { polyline points=(seg) {} }
        }
    }
}

/// The viewer's UTC offset in seconds, from the `lighthouse_tz` cookie (minutes) the page sets.
pub fn tz_offset(headers: &HeaderMap) -> i64 {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            kv.trim()
                .strip_prefix("lighthouse_tz=")
                .and_then(|m| m.parse::<i64>().ok())
        })
        .map_or(0, |m| m.clamp(-14 * 60, 14 * 60) * 60)
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Weekday (0 = Monday) and hour for a unix time shifted by `tz` seconds.
pub fn weekday_hour(unix: i64, tz: i64) -> (usize, usize) {
    let t = unix + tz;
    let days = t.div_euclid(86_400);
    (
        ((days + 3).rem_euclid(7)) as usize,
        (t.rem_euclid(86_400) / 3600) as usize,
    )
}

/// `Tue 20:00` or `Nov 10 20:00` style label in the viewer's time zone.
pub fn local_label(unix: i64, tz: i64, with_day: bool) -> String {
    let (wd, h) = weekday_hour(unix, tz);
    let m = (unix + tz).rem_euclid(3600) / 60;
    if with_day {
        format!("{} {h:02}:{m:02}", WEEKDAYS[wd])
    } else {
        format!("{h:02}:{m:02}")
    }
}

/// Weekday × hour grid. `cells[d][h]` = average value, `None` = no data.
pub fn heatmap(cells: &[[Option<f64>; 24]; 7], unit: &str) -> Markup {
    let max = cells.iter().flatten().flatten().fold(0.0f64, |a, b| a.max(*b));
    html! {
        div.heat-wrap {
            table.heat {
                thead { tr { th {} @for h in 0..24 { th { @if h % 3 == 0 { (format!("{h:02}")) } } } } }
                tbody {
                    @for (d, row) in cells.iter().enumerate() {
                        tr {
                            th { (WEEKDAYS[d]) }
                            @for v in row {
                                @match v {
                                    Some(v) => {
                                        @let pct = if max > 0.0 { (v / max * 85.0 + 8.0).round() } else { 8.0 };
                                        td style={ "--v:" (pct) "%" } title={ (format!("{v:.1}")) " " (unit) } {}
                                    }
                                    None => td.empty title="No data" {},
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_numbers() {
        assert_eq!(nice_max(0.0), 1.0);
        assert_eq!(nice_max(7.0), 10.0);
        assert_eq!(nice_max(23.0), 25.0);
        assert_eq!(nice_max(30.0), 50.0);
    }

    #[test]
    fn line_segments_break_on_gaps() {
        let s = segments(&[Some(1.0), Some(2.0), None, Some(2.0)], 2.0, 100.0);
        assert_eq!(s.len(), 2);
        assert!(s[0].contains(' ') && !s[1].contains(' '));
    }

    #[test]
    fn weekdays_and_hours() {
        // 1970-01-01 was a Thursday.
        assert_eq!(weekday_hour(0, 0), (3, 0));
        assert_eq!(weekday_hour(1_794_339_508, 0), (1, 19)); // Tue 2026-11-10 19:38 UTC
        assert_eq!(weekday_hour(1_794_339_508, 10 * 3600), (2, 5)); // Wed 05:38 in UTC+10
        assert_eq!(local_label(1_794_339_508, 0, true), "Tue 19:38");
    }

    #[test]
    fn tz_cookie() {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::COOKIE, "a=1; lighthouse_tz=600".parse().unwrap());
        assert_eq!(tz_offset(&h), 36_000);
    }
}
