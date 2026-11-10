//! Shared types for Lighthouse.
//!
//! Everything that more than one part of Lighthouse needs lives here: the normalised
//! [`ServerStatus`], the [`GameAdapter`] trait each supported game implements, the
//! configuration file format and the heartbeat format that game mods and agents send.

mod catalog;
pub mod config;
pub mod game;
pub mod heartbeat;
pub mod status;

pub use config::{
    BotConfig, BotMode, Config, ConfigError, DEFAULT_UPDATE_REPO, DashboardConfig, IngestConfig, NativeApi,
    ServerConfig, UPDATE_CHANNELS, UpdateConfig,
};
pub use game::{Capabilities, GameAdapter, GameKind, GameProfile, Protocol, QueryError, QueryPort, Support};
pub use heartbeat::{HEARTBEAT_VERSION, Heartbeat};
pub use status::{GameTime, Player, ServerState, ServerStatus};
