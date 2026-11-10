//! Game adapters.
//!
//! 7 Days to Die, Project Zomboid and VEIN have their own adapters. Every other catalogue game
//! uses the generic adapter for its protocol: Steam query (A2S), Minecraft Java/Bedrock ping or
//! FiveM HTTP. Game-specific APIs (7DTD web API, PZ RCON, VEIN HTTP) will be layered on top.

pub mod a2s;
mod fivem;
mod generic;
mod minecraft;
mod project_zomboid;
pub mod rcon;
mod seven_days;
mod steam;
mod vein;

use lighthouse_core::{GameAdapter, GameKind, Protocol};

pub use project_zomboid::ProjectZomboid;
pub use seven_days::SevenDaysToDie;
pub use vein::Vein;

/// The adapter for a game.
pub fn adapter_for(game: GameKind) -> Box<dyn GameAdapter> {
    if game.is(GameKind::SEVEN_DAYS_TO_DIE) {
        return Box::new(SevenDaysToDie);
    }
    if game.is(GameKind::PROJECT_ZOMBOID) {
        return Box::new(ProjectZomboid);
    }
    if game.is(GameKind::VEIN) {
        return Box::new(Vein);
    }
    match game.protocol() {
        Protocol::Steam => Box::new(generic::GenericSteam(game)),
        Protocol::MinecraftJava => Box::new(minecraft::MinecraftJava(game)),
        Protocol::MinecraftBedrock => Box::new(minecraft::MinecraftBedrock(game)),
        Protocol::FiveM => Box::new(fivem::FiveM(game)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_has_an_adapter() {
        for g in GameKind::all() {
            assert_eq!(adapter_for(g).kind(), g);
        }
    }
}
