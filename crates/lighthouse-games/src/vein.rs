use lighthouse_core::{Capabilities, GameAdapter, GameKind, QueryError, ServerConfig, ServerStatus};

use crate::steam;

/// VEIN. Steam query on `-QueryPort` (default 27015; many hosts use the game port + 1).
/// The Steam player list has entries but no names, and the version is in the `Version` rule.
/// The `HTTPPort` API (uptime, names) comes later; it has no authentication, so only use it on
/// localhost or a private network.
pub struct Vein;

#[async_trait::async_trait]
impl GameAdapter for Vein {
    fn kind(&self) -> GameKind {
        GameKind::get(GameKind::VEIN)
    }

    fn capabilities(&self, _server: &ServerConfig) -> Capabilities {
        Capabilities::default()
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let data = steam::query(server).await?;
        Ok(steam::base_status(&data, now_unix))
    }
}
