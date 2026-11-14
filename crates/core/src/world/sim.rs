//! The per-tick simulation systems.
//!
//! Order matters and must never change mid-patch, otherwise replays break:
//! economy -> construction -> production -> combat -> movement ->
//! death/cleanup -> fog -> victory.

use crate::defs::{BuildingRole, DamageType, UnitRole, UNIT_CLUBMAN};
use crate::entity::{Entity, GameEvent, Order, UnitState};
use crate::epoch::Epoch;
use crate::ids::{EntityId, PlayerId, UnitDefId};
use crate::map::MapSize;
use crate::math::{TilePos, Vec2};
use crate::resource::{Cargo, ResourceKind};
use crate::time::TICKS_PER_SECOND;

use super::{formation_offset, MatchOutcome, World};

/// How often the spatial grid is rebuilt (ticks). Units move slowly relative
/// to the tick rate, so 2 ticks is plenty and keeps combat cheap.
const GRID_REFRESH_TICKS: u32 = 2;
/// Melee attack slack added to the range check (units have a body).
const MELEE_SLACK: i32 = 300;
/// How often a unit may retry pathing around an obstacle.
const REPATH_INTERVAL: u64 = 20;

impl World {
    /// Run exactly one simulation tick. Deterministic for a given state.
    pub fn step(&mut self) {
        if self.is_over() {
            // Keep the clock running so spectators see time pass, but freeze
            // gameplay. Sandbox never "ends", it just keeps simulating.
            self.tick = self.tick.next();
            return;
        }

        self.tick = self.tick.next();
        let t = self.tick;

        if t.is_multiple_of(GRID_REFRESH_TICKS) {
            self.rebuild_spatial_grid();
        }

        self.apply_income();
        self.tick_construction();
        self.tick_production();
        self.tick_combat();
        self.tick_movement();
        self.tick_cleanup();
        self.maybe_refresh_fog();
        self.check_eliminations();
        self.check_victory();
        if self.config.mode.has_meteors() {
            self.tick_meteors();
        }
        if self.config.mode == crate::world::GameMode::Survival {
            self.tick_survival(t);
        }
        if t.is_multiple_of(TICKS_PER_SECOND) {
            self.update_all_population();
        }
    }

    /// Run N ticks (used by tests and the headless server).
    pub fn step_many(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
            if self.outcome != MatchOutcome::Ongoing
                && self.config.mode != crate::world::GameMode::Sandbox
            {
                break;
            }
        }
    }

    fn rebuild_spatial_grid(&mut self) {
        self.grid.clear();
        let mut entries: Vec<(EntityId, Vec2)> = Vec::with_capacity(self.entities.len());
        for (id, e) in self.entities.iter() {
            if e.is_unit() {
                entries.push((id, e.pos()));
            }
        }
        for (id, pos) in entries {
            self.grid.insert(id, pos);
        }
    }

    // -- construction ------------------------------------------------------

    fn tick_construction(&mut self) {
        // Collect sites first: mutating the arena while iterating is not
        // possible, and we want a stable order.
        let mut sites: Vec<(EntityId, PlayerId, TilePos, bool, u32, u32)> = Vec::new();
        for (id, e) in self.entities.iter() {
            if let Entity::Building(b) = e {
                if !b.finished {
                    sites.push((id, b.owner, b.origin, b.finished, b.work_done, 0));
                }
            }
        }

        // Builders: workers with a Build order standing next to a site.
        let mut contributions: Vec<(EntityId, u32)> = Vec::new();
        for (uid, u) in self
            .entities
            .iter()
            .filter_map(|(id, e)| e.as_unit().map(|u| (id, u)))
        {
            let Order::Build { target } = u.order else {
                continue;
            };
            let Some((site_id, owner, origin, _, _, _)) =
                sites.iter().find(|(sid, _, _, _, _, _)| *sid == target)
            else {
                continue;
            };
            if *owner != u.owner {
                continue;
            }
            let site_pos = *origin;
            let worker_tile = TilePos::from_vec(u.pos);
            if worker_tile.dist(site_pos) <= 4 {
                let rate = self.catalog.unit(u.def).build_work.max(1);
                contributions.push((*site_id, rate as u32));
                let _ = uid;
            }
        }

        let mut completed: Vec<(EntityId, PlayerId)> = Vec::new();
        for (site_id, amount) in contributions {
            let total = self
                .entity(site_id)
                .and_then(|e| e.as_building())
                .map(|b| self.catalog.building(b.def).build_ticks as u32)
                .unwrap_or(0);
            if let Some(b) = self.entity_mut(site_id).and_then(|e| e.as_building_mut()) {
                b.work_done += amount;
                // Construction site repairs itself while being built.
                b.hp = (b.hp + (b.max_hp / 200).max(1)).min(b.max_hp);
                if b.work_done >= total {
                    b.work_done = total;
                    b.finished = true;
                    b.hp = b.max_hp;
                    completed.push((site_id, b.owner));
                }
            }
        }

        for (id, owner) in completed {
            self.emit(GameEvent::BuildingCompleted { owner, entity: id });
            if let Some(b) = self.building(id) {
                if self.catalog.building(b.def).role == BuildingRole::Wonder {
                    self.emit(GameEvent::WonderCompleted { owner });
                }
            }
        }
        if !sites.is_empty() {
            self.recompute_player_caches();
        }
    }

    // -- production --------------------------------------------------------

    fn tick_production(&mut self) {
        let mut produced: Vec<(EntityId, PlayerId, UnitDefId, Vec2, bool, u32, bool)> = Vec::new();
        let mut updated: Vec<(EntityId, u32)> = Vec::new();

        for (id, e) in self.entities.iter() {
            let Entity::Building(b) = e else { continue };
            if !b.finished || b.queue.is_empty() {
                continue;
            }
            let item = b.queue[0];
            let wonder_role = self.catalog.building(b.def).role == BuildingRole::Wonder;
            produced.push((
                id,
                b.owner,
                item.unit,
                b.rally.unwrap_or(b.pos),
                wonder_role,
                item.ticks_left,
                false,
            ));
        }

        for (building_id, owner, unit, rally, _wonder, ticks_left, _) in produced {
            let speed = self.production_speed_milli(owner);
            let decrement = (1000i64 * speed as i64 / 1000).max(1) as u32;
            let new_ticks = ticks_left.saturating_sub(decrement);
            updated.push((building_id, new_ticks));

            if new_ticks == 0 {
                let spawn_pos = self
                    .map
                    .nearest_walkable(TilePos::from_vec(rally), 8)
                    .map(|t| t.center())
                    .unwrap_or(rally);
                // Try to spawn; if the population cap was hit meanwhile, the
                // unit is lost with a partial refund (classic RTS behaviour).
                match self.spawn_unit(unit, owner, spawn_pos) {
                    Ok(new_id) => {
                        self.emit(GameEvent::UnitTrained {
                            owner,
                            entity: new_id,
                        });
                        if let Some(b) = self
                            .entity_mut(building_id)
                            .and_then(|e| e.as_building_mut())
                        {
                            b.queue.remove(0);
                            b.produce_timer = 0;
                        }
                    }
                    Err(_) => {
                        // Refund and drop the queue item to avoid a stall.
                        let cost = self.catalog.unit(unit).effective_cost();
                        self.player_mut(owner).refund(cost);
                        if let Some(b) = self
                            .entity_mut(building_id)
                            .and_then(|e| e.as_building_mut())
                        {
                            b.queue.remove(0);
                        }
                    }
                }
            }
        }
        for (building_id, ticks_left) in updated {
            if let Some(b) = self
                .entity_mut(building_id)
                .and_then(|e| e.as_building_mut())
            {
                if let Some(item) = b.queue.first_mut() {
                    item.ticks_left = ticks_left;
                }
            }
        }
    }

    fn production_speed_milli(&self, owner: PlayerId) -> i32 {
        let p = self.player(owner);
        let mut speed = 1000 + p.epoch.index() as i32 * 60;
        // The academy research (bit 0) gives +15%.
        if p.research_bits & 1 != 0 {
            speed += 150;
        }
        speed
    }

    // -- combat ------------------------------------------------------------

    fn tick_combat(&mut self) {
        // Snapshot positions so both sides of a fight see the same tick.
        self.scratch_ids.clear();
        let mut units: Vec<EntityId> = Vec::new();
        for (id, e) in self.entities.iter() {
            if e.is_unit() {
                units.push(id);
            }
        }
        let mut buildings: Vec<EntityId> = Vec::new();
        for (id, e) in self.entities.iter() {
            if e.is_building() {
                buildings.push(id);
            }
        }

        for id in units {
            self.tick_unit_combat(id);
        }
        for id in buildings {
            self.tick_tower_combat(id);
        }
    }

    fn tick_unit_combat(&mut self, id: EntityId) {
        let Some(u) = self.unit(id) else { return };
        if u.hp <= 0 {
            return;
        }
        let def = self.catalog.unit(u.def).clone();
        let owner = u.owner;
        let pos = u.pos;
        let cooldown = u.attack_cooldown;
        let current_target = u.target;
        let is_worker = def.role.is_worker();

        // Validate the current target still exists and is hostile.
        let mut target = current_target.filter(|t| {
            self.entity(*t)
                .map(|e| e.owner() != owner && e.is_alive())
                .unwrap_or(false)
        });

        // Workers never auto-acquire: they only fight when explicitly told.
        let order = u.order;
        let wants_auto_acquire = !is_worker
            && matches!(
                order,
                Order::Idle | Order::Hold | Order::Move { .. } | Order::AttackMove { .. }
            );

        // Attack-move and explicit attack orders override the idle check.
        let explicit = matches!(order, Order::Attack { .. } | Order::AttackMove { .. });

        let range = (def.attack_range * 256)
            + if def.attack_range <= 1 {
                MELEE_SLACK
            } else {
                0
            };

        if target.is_none() && (wants_auto_acquire || explicit) {
            let search = if def.role.is_ranged() {
                (def.attack_range * 256).max(def.sight_milli)
            } else {
                def.sight_milli
            };
            target = self.acquire_target(owner, pos, search, explicit);
        }

        let Some(target_id) = target else {
            // Nothing to fight: drop the target and let movement resume.
            if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                if matches!(u.order, Order::Attack { .. }) {
                    u.order = Order::Idle;
                    u.state = UnitState::Idle;
                }
                u.target = None;
                if is_worker {
                    u.state = match u.order {
                        Order::Gather { .. } | Order::GatherNode { .. } => UnitState::Gathering,
                        Order::Build { .. } => UnitState::Building,
                        _ => UnitState::Idle,
                    };
                } else if u.state == UnitState::Attacking {
                    u.state = UnitState::Idle;
                }
            }
            return;
        };

        let target_pos = self.entity(target_id).map(|e| e.pos()).unwrap_or(pos);
        let dist_sq = Vec2::dist_sq(pos, target_pos);
        let in_range = dist_sq <= (range as i64) * (range as i64);

        // Compute the chase path *before* taking a mutable borrow of the unit.
        let chase_path = if !in_range {
            let holds = self
                .unit(id)
                .map(|u| u.order == Order::Hold || u.has_path())
                .unwrap_or(false);
            if holds {
                None
            } else {
                let start = TilePos::from_vec(pos);
                let dest = self
                    .map
                    .nearest_walkable(TilePos::from_vec(target_pos), 8)
                    .unwrap_or(TilePos::from_vec(target_pos));
                self.pathfinder
                    .find_path(&self.map, start, dest, 2500)
                    .map(Some)
                    .unwrap_or(None)
            }
        } else {
            None
        };

        if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
            u.target = Some(target_id);
            u.state = UnitState::Attacking;
            if u.order == Order::Idle {
                u.order = Order::Attack { target: target_id };
            }
            match chase_path {
                Some(path) => {
                    u.clear_path();
                    u.path = path;
                }
                None if in_range => u.clear_path(),
                None => {}
            }
        }

        if in_range && cooldown == 0 {
            self.deal_damage(id, target_id, &def);
            if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                u.attack_cooldown = def.attack_period as u32;
            }
        }
    }

    /// Find a hostile entity within `radius`. Deterministic: nearest, then
    /// lowest entity index.
    fn acquire_target(
        &self,
        owner: PlayerId,
        from: Vec2,
        radius: i32,
        aggressive_only: bool,
    ) -> Option<EntityId> {
        let mut best: Option<(i32, EntityId)> = None;
        // Buildings are only auto-attacked when they are the only threat.
        let mut best_building: Option<(i32, EntityId)> = None;
        for (id, e) in self.entities.iter() {
            if e.owner() == owner || !e.is_alive() {
                continue;
            }
            if e.owner().is_gaia() {
                continue;
            }
            let dist = Vec2::dist(from, e.pos()).to_int();
            if dist * 256 > radius {
                continue;
            }
            if e.is_building() && !aggressive_only {
                let key = (dist, id);
                if best_building.is_none_or(|b| key < b) {
                    best_building = Some(key);
                }
                continue;
            }
            let key = (dist, id);
            if best.is_none_or(|b| key < b) {
                best = Some(key);
            }
        }
        best.or(best_building).map(|(_, id)| id)
    }

    fn deal_damage(&mut self, attacker: EntityId, target: EntityId, def: &crate::defs::UnitDef) {
        let Some(attacker_owner) = self.entity(attacker).map(|e| e.owner()) else {
            return;
        };
        let bonus = self
            .entity(attacker)
            .and_then(|e| e.as_unit())
            .map(|u| u.damage_bonus_milli)
            .unwrap_or(0);
        let Some(target_armor) = self.entity(target).map(|e| match e {
            Entity::Unit(_) => self
                .unit(target)
                .map(|u| self.catalog.unit(u.def).armor)
                .unwrap_or(crate::defs::ArmorType::Light),
            Entity::Building(_) => crate::defs::ArmorType::Structure,
        }) else {
            return;
        };
        let effectiveness = def.damage_type.effectiveness(target_armor);
        let raw = def.attack as i64 * (1000 + bonus as i64) / 1000;
        let damage = (raw * effectiveness as i64 / 1000).max(1) as i32;
        self.apply_damage_internal(target, damage, attacker_owner, Some(attacker));
    }

    fn tick_tower_combat(&mut self, id: EntityId) {
        let Some(b) = self.building(id) else { return };
        let def = self.catalog.building(b.def).clone();
        if def.attack <= 0 || !b.finished || b.attack_cooldown > 0 {
            if b.attack_cooldown > 0 {
                if let Some(b) = self.entity_mut(id).and_then(|e| e.as_building_mut()) {
                    b.attack_cooldown -= 1;
                }
            }
            return;
        }
        let owner = b.owner;
        let pos = b.pos;
        let range = def.attack_range * 256;
        let target = self.acquire_target(owner, pos, range, true);
        let Some(target_id) = target else {
            return;
        };
        let target_armor = self.entity(target_id).map(|e| match e {
            Entity::Unit(u) => self.catalog.unit(u.def).armor,
            Entity::Building(_) => crate::defs::ArmorType::Structure,
        });
        let Some(armor) = target_armor else { return };
        let effectiveness = def.damage_type.effectiveness(armor);
        let damage = (def.attack as i64 * effectiveness as i64 / 1000).max(1) as i32;
        self.apply_damage_internal(target_id, damage, owner, None);
        if let Some(b) = self.entity_mut(id).and_then(|e| e.as_building_mut()) {
            b.attack_cooldown = def.attack_period as u32;
        }
    }

    /// Central damage entry point: clamps, emits events, handles death.
    pub(crate) fn apply_damage_internal(
        &mut self,
        target: EntityId,
        amount: i32,
        attacker: PlayerId,
        attacker_entity: Option<EntityId>,
    ) {
        if amount <= 0 {
            return;
        }
        let Some(target_owner) = self.entity(target).map(|e| e.owner()) else {
            return;
        };
        let mut killed = false;
        if let Some(e) = self.entity_mut(target) {
            match e {
                Entity::Unit(u) => {
                    u.hp -= amount;
                    if u.hp <= 0 {
                        u.hp = 0;
                        u.state = UnitState::Dying;
                        killed = true;
                    }
                }
                Entity::Building(b) => {
                    b.hp -= amount;
                    if b.hp <= 0 {
                        b.hp = 0;
                        killed = true;
                    }
                }
            }
        }
        self.emit(GameEvent::Damage { target, amount });
        if killed {
            self.kill_entity(target, attacker);
            let _ = (target_owner, attacker_entity);
        }
    }

    fn kill_entity(&mut self, id: EntityId, killer: PlayerId) {
        let Some(entity) = self.entities.remove(id) else {
            return;
        };
        self.emit(GameEvent::EntityKilled { entity: id, killer });
        if !killer.is_gaia() && killer != entity.owner() {
            if let Some(k) = self.players.get_mut(killer.index()) {
                k.kills += 1;
            }
        }
        if let Some(l) = self.players.get_mut(entity.owner().index()) {
            l.losses += 1;
        }
        match entity {
            Entity::Unit(u) => {
                let pop = self.catalog.unit(u.def).pop;
                if let Some(p) = self.players.get_mut(u.owner.index()) {
                    p.population = (p.population - pop).max(0);
                }
                // Dead workers drop whatever they carried.
                // (Cargo is simply lost; keeps the sim cheap and is standard.)
            }
            Entity::Building(b) => {
                let footprint = self.catalog.building(b.def).footprint;
                self.map.set_blocked_rect(b.origin, footprint, false);
                // Killing a building cancels its queue with a refund.
                let owner = b.owner;
                for item in b.queue {
                    if item.paid {
                        let cost = self.catalog.unit(item.unit).effective_cost();
                        self.player_mut(owner).refund(cost);
                    }
                }
                self.recompute_player_caches();
            }
        }
    }

    // -- movement ----------------------------------------------------------

    fn tick_movement(&mut self) {
        let mut ids: Vec<EntityId> = Vec::new();
        for (id, e) in self.entities.iter() {
            if e.is_unit() {
                ids.push(id);
            }
        }
        for id in ids {
            self.tick_unit_movement(id);
        }
        // Cooldowns tick down after movement so a unit that steps into range
        // can attack on the next tick.
        for (_, e) in self.entities.iter_mut() {
            if let Entity::Unit(u) = e {
                u.attack_cooldown = u.attack_cooldown.saturating_sub(1);
            }
        }
    }

    fn tick_unit_movement(&mut self, id: EntityId) {
        let Some(u) = self.unit(id) else { return };
        if u.hp <= 0 {
            return;
        }
        let def = self.catalog.unit(u.def).clone();
        let speed = def.speed_milli;
        let order = u.order;
        let pos = u.pos;

        let mut desired: Option<Vec2> = None;
        let mut arrived_action: Option<ArrivedAction> = None;

        match order {
            Order::Move { target } | Order::AttackMove { target } => {
                if Vec2::dist(pos, target).to_int() <= 1 {
                    arrived_action = Some(ArrivedAction::Stop);
                } else {
                    desired = Some(target);
                }
            }
            Order::Attack { target } => {
                let Some(t) = self.entity(target) else {
                    return self.finish_movement(id, Some(ArrivedAction::Stop));
                };
                let tpos = t.pos();
                let range = (def.attack_range * 256) + MELEE_SLACK;
                if Vec2::dist(pos, tpos).to_int() * 256 > range {
                    desired = Some(tpos);
                }
            }
            Order::Gather { node } | Order::GatherNode { node } => {
                if def.role.is_worker() {
                    self.tick_worker_gather(id, node, &def);
                    return;
                }
            }
            Order::Build { target } | Order::Repair { target } => {
                let (bpos, finished) = match self.building(target) {
                    Some(b) => (b.pos, b.finished),
                    None => return self.finish_movement(id, Some(ArrivedAction::Stop)),
                };
                if finished {
                    // The site was completed (or cancelled) while walking
                    // there; release the builder so the bot can reassign it.
                    return self.finish_movement(id, Some(ArrivedAction::Stop));
                }
                if TilePos::from_vec(pos).dist(TilePos::from_vec(bpos)) > 4 {
                    desired = Some(bpos);
                } else if matches!(order, Order::Repair { .. }) {
                    // Repair is instant in the MVP once in range.
                    arrived_action = Some(ArrivedAction::Stop);
                }
            }
            Order::Idle | Order::Hold => {}
        }

        if let Some(dest) = desired {
            self.step_toward(id, dest, speed as u32, &def);
        } else if arrived_action.is_some() {
            self.finish_movement(id, arrived_action);
        }
    }

    fn finish_movement(&mut self, id: EntityId, action: Option<ArrivedAction>) {
        let Some(action) = action else { return };
        match action {
            ArrivedAction::Stop => {
                if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                    u.clear_path();
                    u.target = None;
                    u.order = Order::Idle;
                    u.state = UnitState::Idle;
                }
            }
        }
    }

    /// Move one unit one tick along its path toward `dest`.
    fn step_toward(
        &mut self,
        id: EntityId,
        dest: Vec2,
        speed_milli: u32,
        def: &crate::defs::UnitDef,
    ) {
        // Take the path out of the unit to avoid holding a borrow of `self`.
        let Some((mut pos, mut path, mut cursor, mut last_repath)) =
            self.entity_mut(id).and_then(|e| e.as_unit_mut()).map(|u| {
                (
                    u.pos,
                    std::mem::take(&mut u.path),
                    u.path_cursor,
                    u.last_repath_tick,
                )
            })
        else {
            return;
        };
        let tick = self.tick.0;

        // Ensure a path exists toward `dest`.
        if cursor >= path.len() {
            let start = TilePos::from_vec(pos);
            let goal = self
                .map
                .nearest_walkable(TilePos::from_vec(dest), 10)
                .unwrap_or(TilePos::from_vec(dest));
            if start != goal && tick.saturating_sub(last_repath) >= REPATH_INTERVAL {
                let new_path = self.pathfinder.find_path(&self.map, start, goal, 4000);
                if let Some(np) = new_path {
                    path = np;
                    cursor = 0;
                    last_repath = tick;
                }
            }
        }

        // Step in fixed units: `speed_milli` is milli-tiles per second.
        // bits_per_tick = speed_milli / 1000 * 256 / TICKS_PER_SECOND
        let step = (speed_milli as i64 * 256 / 1000 / TICKS_PER_SECOND as i64) as i32;
        let mut remaining = step.max(1);

        while remaining > 0 {
            let waypoint = if cursor < path.len() {
                path[cursor].center()
            } else {
                dest
            };
            let dist_units = Vec2::dist(pos, waypoint);
            let dist = dist_units.to_bits();
            if dist <= remaining {
                pos = waypoint;
                remaining -= dist;
                if cursor < path.len() {
                    cursor += 1;
                } else {
                    break;
                }
            } else {
                let dir = (waypoint - pos).normalized();
                pos = pos + dir * crate::math::Fixed::from_bits(remaining);
                remaining = 0;
            }
        }

        // Keep the unit inside the map.
        let max_x = (self.map.width - 1) * 256;
        let max_y = (self.map.height - 1) * 256;
        pos.x = pos.x.clamp(
            crate::math::Fixed::ZERO,
            crate::math::Fixed::from_bits(max_x),
        );
        pos.y = pos.y.clamp(
            crate::math::Fixed::ZERO,
            crate::math::Fixed::from_bits(max_y),
        );

        if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
            u.pos = pos;
            u.path = path;
            u.path_cursor = cursor;
            u.last_repath_tick = last_repath;
            if matches!(u.state, UnitState::Idle) {
                u.state = UnitState::Moving;
            }
        }
        let _ = def;
    }

    // -- gathering ---------------------------------------------------------

    fn tick_worker_gather(&mut self, id: EntityId, node: usize, def: &crate::defs::UnitDef) {
        let Some(n) = self.map.node(node) else {
            self.finish_movement(id, Some(ArrivedAction::Stop));
            return;
        };
        let node_pos = n.pos;
        let kind = n.kind;
        let node_amount = n.amount;
        let Some(u) = self.unit(id) else { return };
        let pos = u.pos;
        let cargo = u.cargo;
        let owner = u.owner;
        let capacity = def.capacity.max(1);
        let gather_rate = def.gather_rate.max(1);

        // A loaded worker always heads home. This check must come before the
        // "walk to the node" branch: otherwise a returning worker is sent back
        // to the node every tick and never deposits anything.
        if cargo.amount >= capacity {
            let Some(target) = self.nearest_drop_off(owner, pos) else {
                // Nowhere to drop: wait with the load until a drop-off exists.
                if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                    u.state = UnitState::Gathering;
                }
                return;
            };
            if TilePos::from_vec(pos).dist(TilePos::from_vec(target)) > 3 {
                self.step_toward(id, target, def.speed_milli as u32, def);
                if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                    u.state = UnitState::Returning;
                }
            } else {
                self.deposit_cargo(id, node, kind, cargo.amount);
            }
            return;
        }

        if node_amount <= 0 {
            // The node was emptied by another worker first.
            if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                u.order = Order::Idle;
                u.gather_node = None;
                u.cargo = Cargo::default();
                u.state = UnitState::Idle;
            }
            return;
        }

        let near = TilePos::from_vec(pos).dist(node_pos) <= 2;
        if !near {
            // Walk to the node.
            self.step_toward(id, node_pos.center(), def.speed_milli as u32, def);
            return;
        }

        // Harvest. Consumption of the node happens on deposit, not on
        // pick-up: keeps the node a stable gather target for the whole trip.
        let per_tick = (gather_rate * 1000 / 3).max(1);
        let mut amount = cargo.amount;
        if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
            u.work_accum += per_tick;
            while u.work_accum >= 1000 && amount < capacity {
                u.work_accum -= 1000;
                amount += 1;
            }
            u.cargo = Cargo {
                kind: Some(kind),
                amount,
            };
            u.state = UnitState::Gathering;
        }
    }

    fn nearest_drop_off(&self, owner: PlayerId, from: Vec2) -> Option<Vec2> {
        let mut best: Option<(i32, Vec2)> = None;
        for (_, e) in self.entities.iter() {
            if let Entity::Building(b) = e {
                if b.owner != owner || !b.finished {
                    continue;
                }
                let role = self.catalog.building(b.def).role;
                if matches!(role, BuildingRole::TownCenter | BuildingRole::Storage) {
                    let d = Vec2::dist(from, b.pos).to_int();
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, b.pos));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }

    fn deposit_cargo(&mut self, id: EntityId, node: usize, kind: ResourceKind, amount: i32) {
        let Some(owner) = self.unit(id).map(|u| u.owner) else {
            return;
        };
        if amount <= 0 {
            return;
        }
        let taken = self.map.take_from_node(node, amount);
        if taken <= 0 {
            // Node is empty: workers go idle.
            if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
                u.order = Order::Idle;
                u.gather_node = None;
                u.cargo = Cargo::default();
                u.state = UnitState::Idle;
            }
            return;
        }
        self.player_mut(owner).add(kind, taken as i64);
        self.emit(GameEvent::ResourceGathered {
            owner,
            kind,
            amount: taken,
        });
        if let Some(u) = self.entity_mut(id).and_then(|e| e.as_unit_mut()) {
            u.cargo = Cargo {
                kind: Some(kind),
                amount: 0,
            };
            u.state = UnitState::Gathering;
        }
    }

    // -- cleanup -----------------------------------------------------------

    fn tick_cleanup(&mut self) {
        // Nothing to do: entities are removed on death. This hook exists for
        // corpses, decals and other future presentation-side state.
        let mut dead: Vec<EntityId> = Vec::new();
        for (id, e) in self.entities.iter() {
            if e.is_unit() && e.hp() <= 0 {
                dead.push(id);
            }
        }
        for id in dead {
            let owner = self.entity(id).map(|e| e.owner());
            let Some(entity) = self.entities.remove(id) else {
                continue;
            };
            if let Entity::Unit(u) = entity {
                let pop = self.catalog.unit(u.def).pop;
                if let Some(p) = self.players.get_mut(u.owner.index()) {
                    p.population = (p.population - pop).max(0);
                }
            }
            let _ = owner;
        }
    }

    // -- arms race: meteors -------------------------------------------------

    fn tick_meteors(&mut self) {
        if !self.tick.is_multiple_of(self.config.meteor_period) {
            return;
        }
        // The player with the smallest territory (approximated by entity count
        // and population) gets hit first.
        let weakest = self
            .players
            .iter()
            .filter(|p| p.is_active() && !p.id.is_gaia())
            .min_by_key(|p| (self.territory_score(p.id), p.id.0))
            .map(|p| p.id);
        let Some(victim) = weakest else { return };
        let target = self
            .entities
            .iter()
            .filter(|(_, e)| e.owner() == victim)
            .filter_map(|(id, _)| self.building_center(id))
            .next()
            .or_else(|| {
                self.entities
                    .iter()
                    .find(|(_, e)| e.owner() == victim)
                    .map(|(_, e)| e.pos())
            });
        let Some(center) = target else { return };

        self.meteor_count += 1;
        let radius = (self.meteor_count.min(6) as i32) * 512;
        self.emit(GameEvent::MeteorStrike {
            pos: center,
            radius_milli: radius,
        });
        let mut hits: Vec<EntityId> = Vec::new();
        for (id, e) in self.entities.iter() {
            if e.owner() == victim && Vec2::dist(center, e.pos()).to_int() * 256 <= radius {
                hits.push(id);
            }
        }
        let damage = 180 + self.meteor_count as i32 * 40;
        for id in hits {
            let is_building = self.entity(id).map(|e| e.is_building()).unwrap_or(false);
            let dealt = if is_building { damage * 4 } else { damage };
            self.apply_damage_internal(id, dealt, PlayerId::GAIA, None);
        }
    }

    fn territory_score(&self, owner: PlayerId) -> i32 {
        let mut score = 0;
        for (_, e) in self.entities.iter() {
            if e.owner() == owner {
                score += match e {
                    Entity::Unit(_) => 2,
                    Entity::Building(_) => 10,
                };
            }
        }
        score
    }

    // -- survival mode -----------------------------------------------------

    fn tick_survival(&mut self, _t: crate::time::Tick) {
        if !self.tick.is_multiple_of(self.config.meteor_period) {
            return;
        }
        let wave = self.meteor_count + 1;
        let count = (wave * 2).min(60);
        let Some(anchor) = self.active_players().next().map(|p| p.id) else {
            return;
        };
        let origin = self
            .entities
            .iter()
            .find(|(_, e)| e.owner() == anchor && e.is_building())
            .map(|(_, e)| e.pos())
            .unwrap_or(Vec2::ZERO);
        // Spawn hostiles on the map edge closest to the anchor.
        let edge_x = if origin.x.to_int() < self.map.width / 2 {
            2
        } else {
            self.map.width - 3
        };
        let edge_y = if origin.y.to_int() < self.map.height / 2 {
            2
        } else {
            self.map.height - 3
        };
        let unit = if self.player(anchor).epoch >= Epoch::Bronze {
            UnitDefId(crate::defs::UNIT_SLINGER)
        } else {
            UnitDefId(UNIT_CLUBMAN)
        };
        for i in 0..count {
            let offset = formation_offset(i as usize, count as usize, 3);
            let pos = self
                .map
                .starts
                .iter()
                .find(|s| s.x == edge_x && s.y == edge_y);
            let base = pos
                .map(|s| s.center())
                .unwrap_or(Vec2::from_ints(edge_x, edge_y));
            let _ = self.spawn_unit(unit, anchor, base + offset);
        }
        self.meteor_count += 1;
    }
}

enum ArrivedAction {
    Stop,
}

impl World {
    /// Debug helper: total damage output of a player's army.
    pub fn army_power(&self, owner: PlayerId) -> i32 {
        self.entities
            .iter()
            .filter_map(|(_, e)| e.as_unit())
            .filter(|u| u.owner == owner)
            .map(|u| {
                let def = self.catalog.unit(u.def);
                def.damage_per_second_milli() * (def.max_hp / 10).max(1)
            })
            .sum()
    }

    /// Debug helper: total map area (used in tests / metrics).
    pub fn map_area(&self) -> i32 {
        self.map.width * self.map.height
    }

    pub fn map_size(&self) -> MapSize {
        self.config.map_size
    }

    pub fn elapsed_seconds(&self) -> u32 {
        self.tick.seconds()
    }

    /// Damage type used by a unit, for UI tooltips.
    pub fn unit_damage_type(&self, id: EntityId) -> Option<DamageType> {
        self.unit(id).map(|u| self.catalog.unit(u.def).damage_type)
    }

    pub fn unit_role(&self, id: EntityId) -> Option<UnitRole> {
        self.unit(id).map(|u| self.catalog.unit(u.def).role)
    }
}
