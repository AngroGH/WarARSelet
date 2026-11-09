//! The authoritative world: owns the map, entities, players and clock.
//!
//! Nothing outside this module mutates gameplay state. Commands go in via
//! [`World::apply_command`], events come out via [`World::drain_events`].
//! The whole struct is `Serialize`, which gives replays and network snapshots
//! for free.

mod commands;
mod config;
mod sim;

pub use config::{GameMode, MatchConfig};

use serde::{Deserialize, Serialize};

use crate::arena::Arena;
use crate::defs::{default_catalog, Catalog, UNIT_CLUBMAN, UNIT_WORKER};
use crate::ids::{BuildingDefId, UnitDefId};
use crate::entity::{Building, Entity, GameEvent, ProductionItem, Unit};
use crate::epoch::Epoch;
use crate::error::CommandError;
use crate::fog::{FogGrid, FOG_REFRESH_TICKS};
use crate::ids::{EntityId, PlayerId};
use crate::map::{Map, MapSize};
use crate::math::{TilePos, Vec2};
use crate::pathfinding::Pathfinder;
use crate::player::{Player, PlayerStatus};
use crate::resource::{Cost, ResourceKind};
use crate::rng::Rng;
use crate::spatial::{SpatialGrid, DEFAULT_CELL_TILES};
use crate::time::Tick;

/// Upper bound on buffered events before the oldest are dropped. Clients that
/// fall behind must resync with a snapshot instead of growing server memory.
pub const EVENT_BUFFER_LIMIT: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum MatchOutcome {
    /// Match is still running.
    Ongoing,
    /// A player (or team) won.
    Victory(PlayerId),
    /// Sandbox or a drawn match.
    Finished,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct World {
    pub config: MatchConfig,
    #[serde(skip)]
    pub catalog: Catalog,
    pub map: Map,
    pub players: Vec<Player>,
    pub entities: Arena<Entity>,
    pub tick: Tick,
    pub outcome: MatchOutcome,
    /// Deterministic RNG for gameplay randomness.
    pub rng: Rng,
    #[serde(skip)]
    pub events: Vec<GameEvent>,
    /// Per-player fog grids. Index matches `players`.
    pub fog: Vec<FogGrid>,
    #[serde(skip)]
    pub grid: SpatialGrid,
    #[serde(skip)]
    pub pathfinder: Pathfinder,
    /// Meteors thrown in Armageddon so far (also drives the defeat timer).
    pub meteor_count: u32,
    /// Per-player count of finished production buildings, cached for the
    /// elimination check and the epoch requirements.
    pub production_buildings: Vec<u16>,
    /// Cached number of workers per player.
    pub worker_count: Vec<u16>,

    #[serde(skip)]
    scratch_ids: Vec<EntityId>,
}

impl World {
    /// Create a match. `players` is a list of `(name, team, is_ai)`.
    pub fn new(config: MatchConfig, players: Vec<(String, u8, bool)>) -> World {
        assert!(!players.is_empty(), "a match needs at least one player");
        assert!(
            players.len() <= config.max_players as usize,
            "too many players for this configuration"
        );

        let map = Map::generate(config.seed, config.map_size, players.len() as u16);
        let catalog = default_catalog();
        debug_assert!(catalog.validate().is_ok());

        let mut player_states = Vec::with_capacity(players.len() + 1);
        // Slot 0 is gaia so `PlayerId(0)` is always the neutral side.
        player_states.push(Player::new(PlayerId::GAIA, "Gaia".into(), 0));
        for (i, (name, team, is_ai)) in players.iter().enumerate() {
            let mut p = Player::new(PlayerId((i + 1) as u16), name.clone(), *team);
            p.is_ai = *is_ai;
            p.resources = config.starting_resources;
            p.epoch = config.start_epoch();
            player_states.push(p);
        }
        let n = player_states.len();

        let mut world = World {
            fog: Vec::new(),
            grid: SpatialGrid::new(&map, DEFAULT_CELL_TILES),
            pathfinder: Pathfinder::new(&map),
            catalog,
            players: player_states,
            entities: Arena::with_capacity(1024),
            tick: Tick::ZERO,
            outcome: MatchOutcome::Ongoing,
            rng: Rng::seeded(config.seed ^ 0xA5A5_5A5A_1234_5678),
            events: Vec::new(),
            meteor_count: 0,
            production_buildings: vec![0; n],
            worker_count: vec![0; n],
            scratch_ids: Vec::new(),
            map,
            config,
        };
        world.fog = (0..n).map(|_| FogGrid::new(&world.map)).collect();
        world.spawn_starting_bases();
        world.recompute_player_caches();
        world.rebuild_fog();
        world
    }

    /// Convenience constructor for a quick skirmish.
    pub fn skirmish(seed: u64, player_count: u16) -> World {
        let names = (1..=player_count)
            .map(|i| (format!("Player {i}"), 0, i != 1))
            .collect();
        World::new(
            MatchConfig {
                seed,
                map_size: if player_count <= 2 {
                    MapSize::Duel
                } else if player_count <= 4 {
                    MapSize::Small
                } else {
                    MapSize::Standard
                },
                ..MatchConfig::default()
            },
            names,
        )
    }

    // -- queries -----------------------------------------------------------

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn player(&self, id: PlayerId) -> &Player {
        &self.players[id.index()]
    }

    pub fn player_mut(&mut self, id: PlayerId) -> &mut Player {
        &mut self.players[id.index()]
    }

    pub fn active_players(&self) -> impl Iterator<Item = &Player> {
        self.players.iter().filter(|p| p.is_active() && !p.id.is_gaia())
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(id)
    }

    pub fn unit(&self, id: EntityId) -> Option<&Unit> {
        self.entities.get(id).and_then(|e| e.as_unit())
    }

    pub fn building(&self, id: EntityId) -> Option<&Building> {
        self.entities.get(id).and_then(|e| e.as_building())
    }

    /// All entities owned by `owner`.
    pub fn entities_of(&self, owner: PlayerId) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities
            .iter()
            .filter(move |(_, e)| e.owner() == owner)
    }

    pub fn population_of(&self, owner: PlayerId) -> i32 {
        self.player(owner).population
    }

    /// Total supply a player currently provides with houses/town centres.
    pub fn provided_population(&self, owner: PlayerId) -> i32 {
        let mut total = 0;
        for (_, e) in self.entities.iter() {
            if let Entity::Building(b) = e {
                if b.owner == owner && b.finished {
                    total += self.catalog.building(b.def).pop_provided;
                }
            }
        }
        total.min(self.config.hard_pop_cap)
    }

    pub fn can_afford(&self, owner: PlayerId, cost: Cost) -> bool {
        self.player(owner).can_afford(cost)
    }

    /// Drain pending events (called by the server after each tick).
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn emit(&mut self, event: GameEvent) {
        if self.events.len() >= EVENT_BUFFER_LIMIT {
            self.events.remove(0);
        }
        self.events.push(event);
    }

    // -- entity creation ---------------------------------------------------

    /// Spawn a unit. Fails when the owner cannot pay population for it.
    pub fn spawn_unit(
        &mut self,
        def: UnitDefId,
        owner: PlayerId,
        pos: Vec2,
    ) -> Result<EntityId, CommandError> {
        let (pop, hp, is_worker) = {
            let unit_def = self.catalog.unit(def);
            (unit_def.pop, unit_def.max_hp, unit_def.role.is_worker())
        };
        if !self.player(owner).has_population_room(pop) {
            return Err(CommandError::PopulationCap);
        }
        let mut unit = Unit::new(def, owner, pos, hp);
        let (epoch, civ_bonus) = {
            let p = self.player(owner);
            (p.epoch, p.civ.combat_bonus_milli())
        };
        let (melee, ranged) = civ_bonus;
        let epoch_bonus = epoch.combat_scale_milli() - 1000;
        let role_ranged = self.catalog.unit(def).role.is_ranged();
        unit.damage_bonus_milli = epoch_bonus + if role_ranged { ranged } else { melee };
        let id = self.entities.insert(Entity::Unit(unit));
        self.player_mut(owner).population += pop;
        if is_worker {
            self.worker_count[owner.index()] += 1;
        }
        Ok(id)
    }

    /// Place a building. `finished` controls whether it is instant.
    pub fn spawn_building(
        &mut self,
        def: BuildingDefId,
        owner: PlayerId,
        origin: TilePos,
        finished: bool,
    ) -> Result<EntityId, CommandError> {
        let building_def = self.catalog.building(def);
        let footprint = building_def.footprint;
        if !self.fits_footprint(origin, footprint) {
            return Err(CommandError::OutOfBounds);
        }
        let pos = building_center(origin, footprint);
        let max_hp = building_def.max_hp;
        let mut building = Building::new(def, owner, origin, pos, max_hp, footprint);
        if finished {
            building.finished = true;
            building.work_done = building_def.build_ticks as u32;
            building.hp = max_hp;
        }
        let id = self.entities.insert(Entity::Building(building));
        self.map.set_blocked_rect(origin, footprint, true);
        self.recompute_player_caches();
        Ok(id)
    }

    /// Apply raw damage to an entity. Used by admin actions, scripted
    /// scenarios and tests; gameplay combat goes through the sim systems.
    pub fn damage_entity(&mut self, target: EntityId, amount: i32, source: PlayerId) {
        self.apply_damage_internal(target, amount, source, None);
    }

    pub fn fits_footprint(&self, origin: TilePos, footprint: i32) -> bool {
        if origin.x < 1 || origin.y < 1 {
            return false;
        }
        if origin.x + footprint > self.map.width - 1 || origin.y + footprint > self.map.height - 1 {
            return false;
        }
        for y in origin.y..origin.y + footprint {
            for x in origin.x..origin.x + footprint {
                if !self.map.buildable(x, y) {
                    return false;
                }
            }
        }
        true
    }

    pub(crate) fn recompute_player_caches(&mut self) {
        self.production_buildings.iter_mut().for_each(|v| *v = 0);
        self.worker_count.iter_mut().for_each(|v| *v = 0);
        for (_, e) in self.entities.iter() {
            match e {
                Entity::Building(b) if b.finished => {
                    let def = self.catalog.building(b.def);
                    if def.role.is_production() {
                        self.production_buildings[b.owner.index()] += 1;
                    }
                }
                Entity::Unit(u) => {
                    let def = self.catalog.unit(u.def);
                    if def.role.is_worker() {
                        self.worker_count[u.owner.index()] += 1;
                    }
                }
                _ => {}
            }
        }
    }

    // -- setup -------------------------------------------------------------

    fn spawn_starting_bases(&mut self) {
        let starts = self.map.starts.clone();
        let player_ids: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| !p.id.is_gaia())
            .map(|p| p.id)
            .collect();

        for (i, owner) in player_ids.iter().enumerate() {
            let Some(start) = starts.get(i).copied() else {
                continue;
            };
            let center = self
                .map
                .nearest_walkable(start, 12)
                .unwrap_or(TilePos::new(self.map.width / 2, self.map.height / 2));

            // Snap the town centre to a spot where a 4x4 footprint fits.
            let tc_origin = self
                .find_free_spot(center, 4, 14)
                .unwrap_or(TilePos::new(center.x.saturating_sub(2), center.y.saturating_sub(2)));
            let _ = self.spawn_building(
                crate::ids::BuildingDefId(crate::defs::BUILDING_TOWN_CENTER),
                *owner,
                tc_origin,
                true,
            );

            // Support buildings around the town centre.
            let supports = [
                (crate::defs::BUILDING_HOUSE, 1),
                (crate::defs::BUILDING_HOUSE, 1),
                (crate::defs::BUILDING_STORAGE, 2),
                (crate::defs::BUILDING_BARRACKS, 3),
            ];
            for (offset_i, (def, size)) in supports.iter().enumerate() {
                let angle_i = offset_i as i32;
                let center_tile = TilePos::new(
                    tc_origin.x + 6 * ((angle_i % 2) * 2 - 1),
                    tc_origin.y + 5 + (angle_i / 2) * 5,
                );
                if let Some(spot) = self.find_free_spot(center_tile, *size, 12) {
                    let _ = self.spawn_building(
                        crate::ids::BuildingDefId(*def),
                        *owner,
                        spot,
                        true,
                    );
                }
            }

            // Workers and a small starting guard.
            let spawn = building_center(tc_origin, 4);
            let workers = self.config.starting_workers;
            let warriors = self.config.starting_warriors;
            for w in 0..workers {
                let off = formation_offset(w as usize, workers as usize, 1);
                let pos = spawn + off;
                let _ = self.spawn_unit(UnitDefId(UNIT_WORKER), *owner, pos);
            }
            for w in 0..warriors {
                let off = formation_offset(w as usize, warriors as usize, 2);
                let pos = spawn + off;
                let _ = self.spawn_unit(UnitDefId(UNIT_CLUBMAN), *owner, pos);
            }

            // Reset the starting bank to the configured value (spawning is free).
            let start_res = self.config.starting_resources;
            self.player_mut(*owner).resources = start_res;
            self.player_mut(*owner).pop_cap = 15;
        }
    }

    /// Find a free `size`x`size` footprint near `center` using a spiral scan.
    fn find_free_spot(&self, center: TilePos, size: i32, radius: i32) -> Option<TilePos> {
        for r in 0..=radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let origin = TilePos::new(center.x + dx - size / 2, center.y + dy - size / 2);
                    if self.fits_footprint(origin, size) {
                        return Some(origin);
                    }
                }
            }
        }
        None
    }

    // -- fog ---------------------------------------------------------------

    pub(crate) fn rebuild_fog(&mut self) {
        for f in &mut self.fog {
            f.clear_visible();
        }
        // Gather (owner, position, sight) with a snapshot to satisfy borrowck.
        let mut view: Vec<(usize, Vec2, i32)> = Vec::with_capacity(self.entities.len());
        for (_, e) in self.entities.iter() {
            let owner = e.owner();
            if owner.is_gaia() {
                continue;
            }
            let sight = match e {
                Entity::Unit(u) => self.catalog.unit(u.def).sight_milli,
                Entity::Building(b) => {
                    let def = self.catalog.building(b.def);
                    def.sight * 256
                }
            };
            view.push((owner.index(), e.pos(), sight));
        }
        for (idx, pos, sight) in view {
            if let Some(fog) = self.fog.get_mut(idx) {
                fog.reveal_circle(pos, sight);
            }
        }
    }

    pub fn is_visible_to(&self, player: PlayerId, pos: Vec2) -> bool {
        self.fog
            .get(player.index())
            .map(|f| f.is_visible(pos))
            .unwrap_or(false)
    }

    /// Refresh fog if due; returns true when it did.
    pub(crate) fn maybe_refresh_fog(&mut self) -> bool {
        if self.tick.is_multiple_of(FOG_REFRESH_TICKS) {
            self.rebuild_fog();
            true
        } else {
            false
        }
    }

    // -- elimination / victory --------------------------------------------

    pub(crate) fn eliminate(&mut self, owner: PlayerId) {
        let tick = self.tick.0;
        let p = self.player_mut(owner);
        if !p.is_active() {
            return;
        }
        p.status = PlayerStatus::Eliminated;
        p.eliminated_at = Some(tick);
        self.emit(GameEvent::PlayerEliminated { owner });
    }

    /// A player is out when their town centre and all production buildings die.
    pub(crate) fn check_eliminations(&mut self) {
        let ids: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| p.is_active() && !p.id.is_gaia())
            .map(|p| p.id)
            .collect();
        for id in ids {
            let mut has_town_center = false;
            let mut has_production = false;
            for (_, e) in self.entities.iter() {
                if let Entity::Building(b) = e {
                    if b.owner == id && !b.finished {
                        continue;
                    }
                    if b.owner == id {
                        let role = self.catalog.building(b.def).role;
                        if role == crate::defs::BuildingRole::TownCenter {
                            has_town_center = true;
                        }
                        if role.is_production() {
                            has_production = true;
                        }
                    }
                }
            }
            if !has_town_center && !has_production {
                self.eliminate(id);
            }
        }
    }

    pub(crate) fn check_victory(&mut self) {
        if !self.config.has_victory() || self.outcome != MatchOutcome::Ongoing {
            return;
        }
        match self.config.mode {
            GameMode::Team => {
                let mut teams: Vec<(u8, bool)> = Vec::new();
                for p in self.players.iter().filter(|p| !p.id.is_gaia()) {
                    match teams.iter_mut().find(|(t, _)| *t == p.team) {
                        Some((_, alive)) => *alive |= p.is_active(),
                        None => teams.push((p.team, p.is_active())),
                    }
                }
                let alive: Vec<&(u8, bool)> = teams.iter().filter(|(_, a)| *a).collect();
                if alive.len() == 1 {
                    let team = alive[0].0;
                    if let Some(winner) = self
                        .players
                        .iter()
                        .find(|p| !p.id.is_gaia() && p.team == team && p.is_active())
                    {
                        self.outcome = MatchOutcome::Victory(winner.id);
                    }
                } else if alive.is_empty() {
                    self.outcome = MatchOutcome::Finished;
                }
            }
            _ => {
                let active: Vec<PlayerId> = self
                    .players
                    .iter()
                    .filter(|p| p.is_active() && !p.id.is_gaia())
                    .map(|p| p.id)
                    .collect();
                if active.len() == 1 {
                    self.outcome = MatchOutcome::Victory(active[0]);
                } else if active.is_empty() {
                    self.outcome = MatchOutcome::Finished;
                }
            }
        }
    }

    /// Advance the epoch for a player. Called by the command handler.
    pub(crate) fn try_advance_epoch(&mut self, owner: PlayerId) -> Result<Epoch, CommandError> {
        let player = self.player(owner);
        if !player.is_active() {
            return Err(CommandError::PlayerNotActive);
        }
        let Some(next) = player.epoch.next() else {
            return Err(CommandError::InvalidOrder);
        };
        let req = player.epoch.advance_requirement();
        let buildings = self
            .entities
            .iter()
            .filter(|(_, e)| matches!(e, Entity::Building(b) if b.finished && b.owner == owner))
            .count() as u16;
        if buildings < req.buildings {
            return Err(CommandError::PrerequisiteMissing);
        }
        if self.worker_count[owner.index()] < req.workers {
            return Err(CommandError::PrerequisiteMissing);
        }
        let cost = req.resources;
        // The requirement is "resources in the bank": they are consumed.
        if !self.player_mut(owner).pay(cost) {
            return Err(CommandError::InsufficientResources);
        }
        self.player_mut(owner).epoch = next;
        self.player_mut(owner).pending_epoch = None;
        self.emit(GameEvent::EpochAdvanced {
            owner,
            epoch: next.index() as u8,
        });
        // New epoch adds population headroom.
        self.player_mut(owner).pop_cap = self.provided_population(owner).min(self.config.hard_pop_cap);
        Ok(next)
    }

    /// Add passive income from farms/economic buildings, once per second.
    pub(crate) fn apply_income(&mut self) {
        if !self.tick.is_multiple_of(crate::time::TICKS_PER_SECOND) {
            return;
        }
        let mut per_player: Vec<Cost> = vec![Cost::ZERO; self.players.len()];
        for (_, e) in self.entities.iter() {
            if let Entity::Building(b) = e {
                if b.finished && !b.owner.is_gaia() {
                    let def = self.catalog.building(b.def);
                    if def.income.total() > 0 {
                        per_player[b.owner.index()] =
                            per_player[b.owner.index()].checked_add(def.income);
                    }
                }
            }
        }
        for (i, inc) in per_player.iter().enumerate() {
            if inc.total() > 0 {
                let p = &mut self.players[i];
                for k in ResourceKind::ALL {
                    let v = inc.get(k);
                    if v > 0 {
                        p.resources.set(k, p.resources.get(k) + v);
                    }
                }
            }
        }
    }
}

/// Centre of a building footprint in fixed-point world coordinates.
pub fn building_center(origin: TilePos, footprint: i32) -> Vec2 {
    Vec2::from_ints(origin.x * 2 + footprint, origin.y * 2 + footprint) * crate::math::Fixed::from_milli(500)
}

/// Deterministic spawn offsets so new units do not stack on one pixel.
pub fn formation_offset(index: usize, total: usize, ring: i32) -> Vec2 {
    if total == 0 {
        return Vec2::ZERO;
    }
    let per_ring = 8.max(total.min(24));
    let r = ring + (index / per_ring) as i32;
    let slot = index % per_ring;
    // 8 fixed directions, repeated per ring with a bigger radius.
    const DIRS: [(i32, i32); 8] = [
        (1000, 0),
        (707, 707),
        (0, 1000),
        (-707, 707),
        (-1000, 0),
        (-707, -707),
        (0, -1000),
        (707, -707),
    ];
    let (dx, dy) = DIRS[slot % 8];
    let radius = 200 + r * 120;
    Vec2::from_milli(dx * radius / 1000, dy * radius / 1000)
}

/// Production helper used by both the world and the tests.
pub fn make_production_item(unit: UnitDefId, ticks: u32) -> ProductionItem {
    ProductionItem {
        unit,
        ticks_left: ticks,
        paid: true,
    }
}
