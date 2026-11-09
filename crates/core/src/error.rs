//! Command rejection reasons.
//!
//! The server never panics on bad client input: every command returns
//! `Result<(), CommandError>` and a rejection is simply reported back.
//! This is the anti-cheat / anti-crash boundary.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum CommandError {
    /// Command arrived from a player that is dead or not in the match.
    PlayerNotActive,
    /// The command references an entity that does not exist (or was killed).
    EntityNotFound,
    /// The entity belongs to someone else.
    NotOwner,
    /// The entity cannot accept this order (wrong type, busy, etc).
    InvalidOrder,
    /// Not enough resources.
    InsufficientResources,
    /// Population cap reached.
    PopulationCap,
    /// Target position outside the map.
    OutOfBounds,
    /// Target is not reachable / not visible.
    Unreachable,
    /// The required epoch has not been reached.
    EpochTooLow,
    /// Missing prerequisite building or research.
    PrerequisiteMissing,
    /// Civilization branch choice is not allowed right now.
    InvalidCivChoice,
    /// Cooldown / rate limit still active.
    OnCooldown,
    /// Malformed payload (bad target list size, etc).
    Malformed,
    /// Feature is not part of the core MVP.
    Unsupported,
}

impl CommandError {
    pub fn message(self) -> &'static str {
        match self {
            CommandError::PlayerNotActive => "player is not active",
            CommandError::EntityNotFound => "entity not found",
            CommandError::NotOwner => "entity is not owned by player",
            CommandError::InvalidOrder => "entity cannot accept this order",
            CommandError::InsufficientResources => "insufficient resources",
            CommandError::PopulationCap => "population cap reached",
            CommandError::OutOfBounds => "target out of bounds",
            CommandError::Unreachable => "target unreachable",
            CommandError::EpochTooLow => "epoch too low",
            CommandError::PrerequisiteMissing => "prerequisite missing",
            CommandError::InvalidCivChoice => "invalid civilization choice",
            CommandError::OnCooldown => "on cooldown",
            CommandError::Malformed => "malformed command",
            CommandError::Unsupported => "unsupported in this build",
        }
    }
}

impl core::fmt::Display for CommandError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.message())
    }
}
