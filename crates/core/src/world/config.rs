//! Match configuration.

use serde::{Deserialize, Serialize};

use crate::epoch::Epoch;
use crate::map::MapSize;
use crate::resource::Cost;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum GameMode {    /// Free for all.
    Ffa,
    /// Team match, elimination is per team.
    Team,
    /// Royal-battle variant: meteors hit the weakest player.
    Armageddon,
    /// Co-op defence against waves.
    Survival,
    /// No victory conditions.
    Sandbox,
}

impl GameMode {
    pub fn name(self) -> &'static str {
        match self {
            GameMode::Ffa => "FFA",
            GameMode::Team => "Team",
            GameMode::Armageddon => "Armageddon",
            GameMode::Survival => "Survival",
            GameMode::Sandbox => "Sandbox",
        }
    }

    pub fn has_meteors(self) -> bool {
        self == GameMode::Armageddon
    }

    pub fn has_victory(self) -> bool {
        self != GameMode::Sandbox
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchConfig {
    pub seed: u64,
    pub map_size: MapSize,
    pub mode: GameMode,
    /// Population ceiling regardless of houses.
    pub hard_pop_cap: i32,
    pub starting_resources: Cost,
    pub starting_workers: u16,
    pub starting_warriors: u16,
    /// Ticks between meteor strikes in Armageddon.
    pub meteor_period: u32,
    /// Include the "wonder" victory.
    pub wonder_victory: bool,
    /// Max players supported (62 in the reference game).
    pub max_players: u16,
}

impl Default for MatchConfig {
    fn default() -> Self {
        MatchConfig {
            seed: 12345,
            map_size: MapSize::Small,
            mode: GameMode::Ffa,
            hard_pop_cap: 200,
            starting_resources: Cost::new(200, 200, 100).with_gold(50),
            starting_workers: 8,
            starting_warriors: 3,
            meteor_period: 600, // 30 seconds
            wonder_victory: true,
            max_players: 62,
        }
    }
}

impl MatchConfig {
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn with_mode(mut self, mode: GameMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_map(mut self, size: MapSize) -> Self {
        self.map_size = size;
        self
    }

    /// Starting epoch (always Stone in the MVP).
    pub fn start_epoch(&self) -> Epoch {
        Epoch::Stone
    }

    /// Whether this configuration uses the standard victory conditions.
    pub fn has_victory(&self) -> bool {
        self.mode.has_victory()
    }
}
