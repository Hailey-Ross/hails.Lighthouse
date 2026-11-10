//! Generic Steam-query adapter for every catalogue game without its own adapter.

use lighthouse_core::{Capabilities, GameAdapter, GameKind, QueryError, ServerConfig, ServerStatus};

use crate::steam;

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
        Ok(steam::base_status(&data, now_unix))
    }
}
