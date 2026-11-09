//! Commands: the *only* way a client can influence the simulation.
//!
//! Everything here is a plain data struct so it can be serialized over the
//! wire, recorded into a replay, and validated server-side.

use serde::{Deserialize, Serialize};

use crate::epoch::{CivSlot, Epoch};
use crate::ids::{BuildingDefId, EntityId, PlayerId, UnitDefId};
use crate::math::TilePos;

/// Player-issued intent. Server validates and applies it atomically.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
    /// Train a unit in a building (appends to the queue).
    Train {
        building: EntityId,
        unit: UnitDefId,
    },
    /// Cancel the last queued item, refunding its cost.
    CancelProduction {
        building: EntityId,
        index: u8,
    },
    /// Place a new building. The origin is the top-left tile.
    PlaceBuilding {
        building: BuildingDefId,
        origin: TilePos,
    },
    /// Cancel a construction site (partial refund).
    CancelBuilding { building: EntityId },
    /// Move a group of units to a point (formation offsets are computed
    /// server-side so clients cannot inject positions).
    Move {
        units: Vec<EntityId>,
        target: TilePos,
    },
    /// Attack-move: engage everything on the way.
    AttackMove {
        units: Vec<EntityId>,
        target: TilePos,
    },
    /// Explicit attack order on an entity.
    Attack {
        units: Vec<EntityId>,
        target: EntityId,
    },
    /// Send workers to a resource node.
    Gather {
        units: Vec<EntityId>,
        node: u32,
    },
    /// Build / repair a friendly building.
    Assist {
        units: Vec<EntityId>,
        target: EntityId,
    },
    /// Stop everything.
    Stop { units: Vec<EntityId> },
    /// Hold position.
    Hold { units: Vec<EntityId> },
    /// Advance to the next epoch (requires the requirements to be met).
    AdvanceEpoch,
    /// Pick culture/branch/country.
    ChooseCiv { slot: CivSlot, value: u16 },
    /// Set the rally point of a production building.
    SetRally {
        building: EntityId,
        target: TilePos,
    },
    /// Give up (FFA / team matches).
    Resign,
    /// Chat has no gameplay effect but is validated and rate limited.
    Chat { text: String },
}

impl Command {
    /// Rough resource cost of the command, used for anti-spam rate limits.
    pub fn weight(&self) -> u32 {
        match self {
            Command::Move { units, .. }
            | Command::AttackMove { units, .. }
            | Command::Attack { units, .. }
            | Command::Gather { units, .. }
            | Command::Assist { units, .. }
            | Command::Stop { units }
            | Command::Hold { units } => units.len().max(1) as u32,
            _ => 1,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Command::Train { .. } => "train",
            Command::CancelProduction { .. } => "cancel_production",
            Command::PlaceBuilding { .. } => "place_building",
            Command::CancelBuilding { .. } => "cancel_building",
            Command::Move { .. } => "move",
            Command::AttackMove { .. } => "attack_move",
            Command::Attack { .. } => "attack",
            Command::Gather { .. } => "gather",
            Command::Assist { .. } => "assist",
            Command::Stop { .. } => "stop",
            Command::Hold { .. } => "hold",
            Command::AdvanceEpoch => "advance_epoch",
            Command::ChooseCiv { .. } => "choose_civ",
            Command::SetRally { .. } => "set_rally",
            Command::Resign => "resign",
            Command::Chat { .. } => "chat",
        }
    }
}

/// How many units one order can address. Bounded to keep packets and
/// server CPU predictable even with 62 players.
pub const MAX_UNITS_PER_ORDER: usize = 200;
pub const MAX_CHAT_LEN: usize = 200;
pub const MAX_QUEUE: usize = 6;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimedCommand {
    /// Tick at which the command was submitted. Used for replays.
    pub tick: u64,
    pub player: PlayerId,
    pub command: Command,
}

/// Result of applying a command, so the server can tell the client why it
/// failed without desynchronising the simulation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub accepted: bool,
    pub reason: Option<crate::error::CommandError>,
    pub epoch: Option<Epoch>,
}

impl CommandResult {
    pub fn ok() -> Self {
        CommandResult {
            accepted: true,
            reason: None,
            epoch: None,
        }
    }

    pub fn rejected(e: crate::error::CommandError) -> Self {
        CommandResult {
            accepted: false,
            reason: Some(e),
            epoch: None,
        }
    }
}
