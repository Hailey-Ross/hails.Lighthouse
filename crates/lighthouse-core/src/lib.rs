//! Shared types for Lighthouse.
//!
//! Everything that more than one part of Lighthouse needs lives here: the normalised
//! [`ServerStatus`], the [`GameAdapter`] trait each supported game implements, the
//! configuration file format and the heartbeat format that game mods and agents send.

mod catalog;
pub mod config;
pub mod control;
pub mod game;
pub mod heartbeat;
pub mod relay;
pub mod status;

pub use config::{
    BotConfig, BotMode, Config, ConfigError, DEFAULT_UPDATE_REPO, DashboardConfig, IngestConfig, NativeApi,
    ServerConfig, Telnet, UPDATE_CHANNELS, UpdateConfig,
};
pub use control::{ControlAction, LocalControl};
pub use game::{Capabilities, GameAdapter, GameKind, GameProfile, Protocol, QueryError, QueryPort, Support};
pub use heartbeat::{Ban, HEARTBEAT_VERSION, Heartbeat, HeartbeatReply, ServerEvent};
pub use relay::{Command, LogLine, Relay};
pub use status::{GameTime, Player, ServerState, ServerStatus};
