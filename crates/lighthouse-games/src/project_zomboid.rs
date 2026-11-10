use lighthouse_core::{Capabilities, GameAdapter, GameKind, QueryError, ServerConfig, ServerStatus};

use crate::a2s::Rules;
use crate::steam;

/// Project Zomboid (Build 42). Steam query on `SteamPort1` (default 8766); RCON support comes later.
pub struct ProjectZomboid;

/// B42 reports `1.0.0.0` as the INFO version; the real one is in the rules or the keywords.
fn real_version(rules: &Rules, keywords: Option<&str>) -> Option<String> {
    rules.get("version").filter(|v| !v.is_empty()).cloned().or_else(|| {
        keywords?
            .split(';')
            .find_map(|k| k.strip_prefix("VERSION:"))
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    })
}

#[async_trait::async_trait]
impl GameAdapter for ProjectZomboid {
    fn kind(&self) -> GameKind {
        GameKind::get(GameKind::PROJECT_ZOMBOID)
    }

    fn capabilities(&self, _server: &ServerConfig) -> Capabilities {
        // The Steam player list carries names; RCON (later) makes it more reliable on B42.
        Capabilities {
            player_names: true,
            ..Capabilities::default()
        }
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let data = steam::query(server).await?;
        let mut status = steam::base_status(&data, now_unix);
        if let Some(v) = real_version(&data.rules, data.info.keywords.as_deref()) {
            status.version = Some(v);
        }
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_rules_then_keywords() {
        let mut rules = Rules::new();
        assert_eq!(
            real_version(&rules, Some(";vanilla;VERSION:42.21")).as_deref(),
            Some("42.21")
        );
        rules.insert("version".into(), "42.22".into());
        assert_eq!(
            real_version(&rules, Some(";vanilla;VERSION:42.21")).as_deref(),
            Some("42.22")
        );
        assert_eq!(real_version(&Rules::new(), None), None);
    }
}
