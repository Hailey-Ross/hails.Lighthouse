//! Small formatting helpers shared by modules.

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `2026-11-10 19:44 UTC`
pub fn fmt_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, secs % 3600 / 60)
}

/// `3d 4h`, `4h 12m`, `12m`, `40s`
pub fn fmt_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{secs}s")
    }
}

/// `5m ago`
pub fn fmt_ago(now: i64, then: i64) -> String {
    format!("{} ago", fmt_duration(now - then))
}

pub fn fmt_bytes(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates() {
        assert_eq!(fmt_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(fmt_utc(1_794_339_508), "2026-11-10 19:38 UTC");
        assert_eq!(fmt_utc(951_782_400), "2000-02-29 00:00 UTC");
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(40), "40s");
        assert_eq!(fmt_duration(3 * 3600 + 120), "3h 2m");
        assert_eq!(fmt_duration(2 * 86_400 + 5 * 3600), "2d 5h");
        assert_eq!(fmt_bytes(2048), "2.0 KB");
    }
}
