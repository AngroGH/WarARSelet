//! `warsel-core`: deterministic, authoritative simulation of War Selection.
//!
//! # Architecture
//!
//! ```text
//!            commands (validated)                  events (broadcast)
//!   client ───────────────────►  World::step()  ───────────────────►  client
//!                                  │
//!                            map / entities / players
//! ```
//!
//! Rules that keep the core trustworthy:
//!
//! * **No floating point in gameplay.** All positions and times are integers
//!   ([`math::Fixed`]), so replays and clients never drift.
//! * **Server authority.** Clients only send [`command::Command`]; the world
//!   validates ownership, epoch, resources, population and bounds.
//! * **Bounded work per tick.** Spatial hashing, bounded pathfinding budgets and
//!   capped event buffers keep 62-player matches predictable.
//! * **Generational entity ids.** A stale id can never point at a new unit.
//!
//! # Example
//!
//! ```
//! use warsel_core::{Command, MatchConfig, World, GameMode};
//!
//! let mut world = World::new(
//!     MatchConfig::default().with_seed(7).with_mode(GameMode::Sandbox),
//!     vec![("Alice".into(), 0, false), ("Bob".into(), 0, true)],
//! );
//! for _ in 0..200 {
//!     world.step();
//! }
//! assert_eq!(world.tick().0, 200);
//! ```

#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]

pub mod arena;
pub mod bot;
pub mod checksum;
pub mod command;
pub mod defs;
pub mod entity;
pub mod epoch;
pub mod error;
pub mod fog;
pub mod ids;
pub mod map;
pub mod math;
pub mod pathfinding;
pub mod player;
pub mod replay;
pub mod resource;
pub mod rng;
pub mod spatial;
pub mod time;
pub mod world;

pub use bot::SimpleBot;
pub use command::{Command, CommandResult, TimedCommand};
pub use defs::{default_catalog, Catalog};
pub use entity::{Building, Entity, GameEvent, Unit};
pub use epoch::{Branch, CivPath, Country, Culture, Epoch};
pub use error::CommandError;
pub use ids::{BuildingDefId, EntityId, PlayerId, UnitDefId};
pub use map::{Map, MapSize, Terrain};
pub use math::{Fixed, TilePos, Vec2};
pub use player::{Player, PlayerStatus};
pub use replay::{Replay, ReplayHeader};
pub use resource::{Cost, ResourceKind};
pub use time::Tick;
pub use world::{GameMode, MatchConfig, MatchOutcome, World};

pub mod prelude {
    pub use crate::command::Command;
    pub use crate::entity::Entity;
    pub use crate::epoch::{CivSlot, Epoch};
    pub use crate::ids::{BuildingDefId, EntityId, PlayerId, UnitDefId};
    pub use crate::math::TilePos;
    pub use crate::resource::ResourceKind;
    pub use crate::world::{GameMode, MatchConfig, World};
}
