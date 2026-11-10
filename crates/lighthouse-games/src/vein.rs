use lighthouse_core::{Capabilities, GameAdapter, GameKind, QueryError, ServerConfig, ServerStatus};

use crate::{native, steam};

/// VEIN. Steam query on `-QueryPort` (default 27015; many hosts use the game port + 1).
/// The Steam player list has entries but no names, and the version is in the `Version` rule.
/// The `HTTPPort` API (`native = { kind = "vein_http", ... }`) adds player names; it has no
/// authentication, so only use it on localhost or a private network.
pub struct Vein;

#[async_trait::async_trait]
impl GameAdapter for Vein {
    fn kind(&self) -> GameKind {
        GameKind::get(GameKind::VEIN)
    }

    fn capabilities(&self, server: &ServerConfig) -> Capabilities {
        Capabilities {
            player_names: server.native.is_some(),
            ..Capabilities::default()
        }
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let data = match steam::query(server).await {
            Ok(data) => data,
            Err(e) => return native::instead_of_query(server, now_unix, e).await,
        };
        let mut status = steam::base_status(&data, now_unix);
        native::extra(server).await.apply(&mut status);
        Ok(status)
    }
}
