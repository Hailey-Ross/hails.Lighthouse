//! Generic Steam-query adapter for every catalogue game without its own adapter.

use lighthouse_core::{Capabilities, GameAdapter, GameKind, GameTime, QueryError, ServerConfig, ServerStatus};

use crate::steam;

/// Rust lists `born<unix time>` (the last wipe) among its comma-separated keywords.
fn rust_wipe(keywords: &str) -> Option<i64> {
    keywords
        .split(',')
        .find_map(|k| k.trim().strip_prefix("born")?.parse().ok())
        .filter(|&t| t > 0)
}

/// DayZ puts the in-game time of day (`HH:MM`) last in its keywords.
fn dayz_time(keywords: &str) -> Option<GameTime> {
    let (h, m) = keywords.rsplit(',').next()?.trim().split_once(':')?;
    let (hour, minute) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (hour < 24 && minute < 60).then_some(GameTime { day: 0, hour, minute })
}

pub struct GenericSteam(pub GameKind);

#[async_trait::async_trait]
impl GameAdapter for GenericSteam {
    fn kind(&self) -> GameKind {
        self.0
    }

    fn capabilities(&self, _server: &ServerConfig) -> Capabilities {
        // Many Steam games list player names; some (7DTD, VEIN) send blank ones.
        Capabilities {
            player_names: true,
            ..Capabilities::default()
        }
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let data = steam::query(server).await?;
        let mut status = steam::base_status(&data, now_unix);
        if let Some(k) = data.info.keywords.as_deref() {
            if self.0.is("rust") {
                status.wiped_at_unix = rust_wipe(k);
            } else if self.0.is("dayz") {
                status.game_time = dayz_time(k);
            }
        }
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_and_dayz_keywords() {
        assert_eq!(
            rust_wipe("mp100,cp3,ptrak,qp0,v2563,born1730000000,gmrust,cs12345"),
            Some(1730000000)
        );
        assert_eq!(rust_wipe("mp100,cp3"), None);
        let t = dayz_time("battleye,no3rd,external,privHive,shard000,lqs0,etm4.000000,entm4.000000,15:24").unwrap();
        assert_eq!((t.day, t.hour, t.minute), (0, 15, 24));
        assert_eq!(dayz_time("battleye,no3rd"), None);
    }
}
