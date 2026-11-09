//! Entities: units and buildings.

use serde::{Deserialize, Serialize};

use crate::ids::{BuildingDefId, EntityId, PlayerId, UnitDefId};
use crate::math::{TilePos, Vec2};
use crate::resource::{Cargo, ResourceKind};

/// Maximum waypoints kept per order (bounded memory, bounded pathfinding).
pub const MAX_PATH: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum UnitState {
    #[default]
    Idle,
    Moving,
    Attacking,
    Gathering,
    Returning,
    Building,
    Dying,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Order {
    /// Stand still, auto-acquire targets in range.
    Idle,
    Move {
        target: Vec2,
    },
    AttackMove {
        target: Vec2,
    },
    Attack {
        target: EntityId,
    },
    /// Harvest from a map node, then deliver to the nearest drop-off.
    Gather {
        node: usize,
    },
    /// Continue an existing `Gather` order after a deposit.
    GatherNode {
        node: usize,
    },
    /// Add construction progress to a friendly building site.
    Build {
        target: EntityId,
    },
    Repair {
        target: EntityId,
    },
    /// Holds position, does not chase.
    Hold,
}

impl Default for Order {
    fn default() -> Self {
        Order::Idle
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unit {
    pub def: UnitDefId,
    pub owner: PlayerId,
    pub pos: Vec2,
    pub hp: i32,
    pub state: UnitState,
    pub order: Order,
    /// Path in tile space, computed by the A* navigator.
    pub path: Vec<TilePos>,
    pub path_cursor: usize,
    /// Current attack target, if any.
    pub target: Option<EntityId>,
    /// Ticks until the next attack is allowed.
    pub attack_cooldown: u32,
    pub cargo: Cargo,
    /// The node this worker is harvesting.
    pub gather_node: Option<usize>,
    /// Idle timer used for deterministic target acquisition staggering.
    pub idle_ticks: u32,
    /// Temporary combat bonuses from research (milli).
    pub damage_bonus_milli: i32,
    pub armor_bonus_milli: i32,
    /// Sub-unit accumulator for gathering (milli-resource units).
    pub work_accum: i32,
    /// Tick of the last path recomputation (throttles repathing).
    pub last_repath_tick: u64,
}

impl Unit {
    pub fn new(def: UnitDefId, owner: PlayerId, pos: Vec2, max_hp: i32) -> Self {
        Unit {
            def,
            owner,
            pos,
            hp: max_hp,
            state: UnitState::Idle,
            order: Order::Idle,
            path: Vec::new(),
            path_cursor: 0,
            target: None,
            attack_cooldown: 0,
            cargo: Cargo::default(),
            gather_node: None,
            idle_ticks: 0,
            damage_bonus_milli: 0,
            armor_bonus_milli: 0,
            work_accum: 0,
            last_repath_tick: 0,
        }
    }

    pub fn clear_path(&mut self) {
        self.path.clear();
        self.path_cursor = 0;
    }

    pub fn has_path(&self) -> bool {
        self.path_cursor < self.path.len()
    }

    pub fn next_waypoint(&mut self) -> Option<Vec2> {
        if self.path_cursor < self.path.len() {
            let t = self.path[self.path_cursor];
            self.path_cursor += 1;
            Some(t.center())
        } else {
            None
        }
    }
}

/// A building under construction or finished.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Building {
    pub def: BuildingDefId,
    pub owner: PlayerId,
    /// Top-left tile of the footprint.
    pub origin: TilePos,
    pub pos: Vec2,
    pub hp: i32,
    pub max_hp: i32,
    /// Accumulated work ticks; equals `def.build_ticks` when finished.
    pub work_done: u32,
    pub finished: bool,
    /// Units queued for training.
    pub queue: Vec<ProductionItem>,
    /// Ticks spent on the wonder (for the wonder victory timer).
    pub wonder_progress: u32,
    pub attack_cooldown: u32,
    /// Rally point for newly trained units.
    pub rally: Option<Vec2>,
    /// Ticks left until the next production tick (deterministic).
    pub produce_timer: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ProductionItem {
    pub unit: UnitDefId,
    pub ticks_left: u32,
    /// Resources already paid at queue time.
    pub paid: bool,
}

impl Building {
    pub fn new(
        def: BuildingDefId,
        owner: PlayerId,
        origin: TilePos,
        pos: Vec2,
        max_hp: i32,
        footprint: i32,
    ) -> Self {
        Building {
            def,
            owner,
            origin,
            pos,
            hp: max_hp / 12, // construction sites are fragile
            max_hp,
            work_done: 0,
            finished: false,
            queue: Vec::new(),
            wonder_progress: 0,
            attack_cooldown: 0,
            rally: None,
            produce_timer: 0,
            // silence unused warning for footprint on definition change
        }
        .with_footprint(footprint)
    }

    fn with_footprint(self, _footprint: i32) -> Self {
        self
    }

    pub fn tiles(&self, footprint: i32) -> impl Iterator<Item = TilePos> {
        let origin = self.origin;
        (0..footprint).flat_map(move |dy| {
            (0..footprint).map(move |dx| TilePos::new(origin.x + dx, origin.y + dy))
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Entity {
    Unit(Unit),
    Building(Building),
}

impl Entity {
    pub fn owner(&self) -> PlayerId {
        match self {
            Entity::Unit(u) => u.owner,
            Entity::Building(b) => b.owner,
        }
    }

    pub fn pos(&self) -> Vec2 {
        match self {
            Entity::Unit(u) => u.pos,
            Entity::Building(b) => b.pos,
        }
    }

    pub fn hp(&self) -> i32 {
        match self {
            Entity::Unit(u) => u.hp,
            Entity::Building(b) => b.hp,
        }
    }

    pub fn is_unit(&self) -> bool {
        matches!(self, Entity::Unit(_))
    }

    pub fn is_building(&self) -> bool {
        matches!(self, Entity::Building(_))
    }

    pub fn as_unit(&self) -> Option<&Unit> {
        match self {
            Entity::Unit(u) => Some(u),
            _ => None,
        }
    }

    pub fn as_unit_mut(&mut self) -> Option<&mut Unit> {
        match self {
            Entity::Unit(u) => Some(u),
            _ => None,
        }
    }

    pub fn as_building(&self) -> Option<&Building> {
        match self {
            Entity::Building(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_building_mut(&mut self) -> Option<&mut Building> {
        match self {
            Entity::Building(b) => Some(b),
            _ => None,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.hp() > 0
    }
}

/// A short-lived visual/audio event emitted by the simulation.
/// Clients replay these; they are never trusted as input.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum GameEvent {
    UnitTrained {
        owner: PlayerId,
        entity: EntityId,
    },
    BuildingCompleted {
        owner: PlayerId,
        entity: EntityId,
    },
    BuildingPlaced {
        owner: PlayerId,
        entity: EntityId,
    },
    Damage {
        target: EntityId,
        amount: i32,
    },
    EntityKilled {
        entity: EntityId,
        killer: PlayerId,
    },
    ResourceGathered {
        owner: PlayerId,
        kind: ResourceKind,
        amount: i32,
    },
    EpochAdvanced {
        owner: PlayerId,
        epoch: u8,
    },
    CivChosen {
        owner: PlayerId,
        slot: u8,
        value: u16,
    },
    WonderCompleted {
        owner: PlayerId,
    },
    PlayerEliminated {
        owner: PlayerId,
    },
    MeteorStrike {
        pos: Vec2,
        radius_milli: i32,
    },
    CommandRejected {
        owner: PlayerId,
        reason: u8,
    },
}
