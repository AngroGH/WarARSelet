//! Typed identifiers. Newtypes prevent mixing up players, entities and defs.

use serde::{Deserialize, Serialize};

/// Index into the player table. Player 0 is reserved for neutral/gaia.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PlayerId(pub u16);

impl PlayerId {
    pub const GAIA: PlayerId = PlayerId(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub fn is_gaia(self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "P{}", self.0)
    }
}

/// Slot index inside the entity arena.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct EntityIndex(pub u32);

/// Generational handle. `generation` invalidates handles to recycled slots,
/// which kills the classic "stale unit id" bug class in RTS code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct EntityId {
    pub index: EntityIndex,
    pub generation: u16,
}

impl EntityId {
    pub const NONE: EntityId = EntityId {
        index: EntityIndex(u32::MAX),
        generation: 0,
    };

    pub fn is_none(self) -> bool {
        self.index.0 == u32::MAX
    }
}

impl core::fmt::Display for EntityId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "E{}.{}", self.index.0, self.generation)
    }
}

/// Stable definition id for a unit type, e.g. `UNIT_SPEARMAN`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct UnitDefId(pub u16);

/// Stable definition id for a building type, e.g. `BUILDING_BARRACKS`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct BuildingDefId(pub u16);
