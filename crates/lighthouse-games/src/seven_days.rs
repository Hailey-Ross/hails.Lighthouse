use lighthouse_core::{Capabilities, GameAdapter, GameKind, GameTime, QueryError, ServerConfig, ServerStatus};

use crate::a2s::Rules;
use crate::{native, steam};

/// 7 Days to Die. Steam query on the game port (default 26900). Its rules carry the world time,
/// blood moon settings and `ServerVersion`. The Steam player list has entries but no names; the
/// web API (`native = { kind = "seven_days_web_api", ... }`) adds names and pings. Note: the `DayCount` rule is a setting, not the current day.
pub struct SevenDaysToDie;

/// One in-game day is 24000 world-time ticks, one hour 1000.
fn game_time(rules: &Rules) -> Option<GameTime> {
    let ticks: u64 = rules.get("CurrentServerTime")?.trim().parse().ok()?;
    let day = u32::try_from(ticks / 24_000 + 1).ok()?;
    let hour = ((ticks / 1_000) % 24) as u8;
    let minute = ((ticks % 1_000) * 60 / 1_000) as u8;
    Some(GameTime { day, hour, minute })
}

/// Days until the next blood moon, assuming it falls on every multiple of `BloodMoonFrequency`.
/// Servers with a random blood moon range (`BloodMoonRange` > 0) are skipped: the exact day
/// isn't in the rules.
fn blood_moon_in_days(rules: &Rules, day: u32) -> Option<u32> {
    let frequency: u32 = rules.get("BloodMoonFrequency")?.trim().parse().ok()?;
    let range: u32 = rules
        .get("BloodMoonRange")
        .and_then(|r| r.trim().parse().ok())
        .unwrap_or(0);
    if frequency == 0 || range > 0 || day == 0 {
        return None;
    }
    Some(day.div_ceil(frequency) * frequency - day)
}

#[async_trait::async_trait]
impl GameAdapter for SevenDaysToDie {
    fn kind(&self) -> GameKind {
        GameKind::get(GameKind::SEVEN_DAYS_TO_DIE)
    }

    fn capabilities(&self, server: &ServerConfig) -> Capabilities {
        Capabilities {
            player_names: server.native.is_some(),
            uptime: false,
            game_time: true,
            blood_moon: true,
        }
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let data = match steam::query(server).await {
            Ok(data) => data,
            Err(e) => return native::instead_of_query(server, now_unix, e).await,
        };
        let mut status = steam::base_status(&data, now_unix);
        if let Some(t) = game_time(&data.rules) {
            status.blood_moon_in_days = blood_moon_in_days(&data.rules, t.day);
            status.game_time = Some(t);
        }
        if status.map.is_none() {
            status.map = data.rules.get("LevelName").cloned();
        }
        native::extra(server).await.apply(&mut status);
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(pairs: &[(&str, &str)]) -> Rules {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn world_time_to_clock() {
        // Day 14, 21:30 → 13 full days + 21.5 hours.
        let t = game_time(&rules(&[("CurrentServerTime", "333500")])).unwrap();
        assert_eq!(
            t,
            GameTime {
                day: 14,
                hour: 21,
                minute: 30
            }
        );
    }

    #[test]
    fn blood_moon_countdown() {
        let r = rules(&[("BloodMoonFrequency", "7"), ("BloodMoonRange", "0")]);
        assert_eq!(blood_moon_in_days(&r, 14), Some(0));
        assert_eq!(blood_moon_in_days(&r, 15), Some(6));
        assert_eq!(blood_moon_in_days(&r, 20), Some(1));
        let random = rules(&[("BloodMoonFrequency", "7"), ("BloodMoonRange", "2")]);
        assert_eq!(blood_moon_in_days(&random, 15), None);
    }
}
