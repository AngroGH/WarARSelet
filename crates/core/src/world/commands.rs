//! Command validation and application.
//!
//! Every rule of the lobby/anti-cheat contract lives here: ownership checks,
//! epoch gates, resource payment, population limits, bounds checks and rate
//! limits. The simulation itself never trusts a client-supplied value.

use crate::command::{Command, CommandResult, MAX_CHAT_LEN, MAX_QUEUE, MAX_UNITS_PER_ORDER};
use crate::defs::{BuildingRole, UNIT_WORKER};
use crate::entity::{Entity, Order, UnitState};
use crate::epoch::{resource_available, CivSlot, Epoch};
use crate::error::CommandError;
use crate::ids::{EntityId, PlayerId};
use crate::map::MapSize;
use crate::math::{TilePos, Vec2};
use crate::resource::Cargo;

use super::{building_center, MatchOutcome, World};

impl World {
    /// Validate and apply one command. Never panics on client input.
    pub fn apply_command(&mut self, player: PlayerId, command: Command) -> CommandResult {
        match self.try_apply(player, command) {
            Ok(mut result) => {
                result.accepted = true;
                result
            }
            Err(e) => {
                self.emit(crate::entity::GameEvent::CommandRejected {
                    owner: player,
                    reason: e as u8,
                });
                CommandResult::rejected(e)
            }
        }
    }

    fn require_active(&self, player: PlayerId) -> Result<(), CommandError> {
        let p = self
            .players
            .get(player.index())
            .ok_or(CommandError::PlayerNotActive)?;
        if player.is_gaia() || !p.is_active() {
            return Err(CommandError::PlayerNotActive);
        }
        Ok(())
    }

    fn owned_building(
        &self,
        player: PlayerId,
        id: EntityId,
    ) -> Result<&crate::entity::Building, CommandError> {
        let b = self.building(id).ok_or(CommandError::EntityNotFound)?;
        if b.owner != player {
            return Err(CommandError::NotOwner);
        }
        Ok(b)
    }

    /// Validate a list of unit ids: all exist, are units, and belong to the
    /// player. Returns the ids that are valid; an entirely foreign or
    /// nonexistent list is a hard error (likely a hacked client).
    fn owned_units(
        &self,
        player: PlayerId,
        units: &[EntityId],
    ) -> Result<Vec<EntityId>, CommandError> {
        if units.is_empty() || units.len() > MAX_UNITS_PER_ORDER {
            return Err(CommandError::Malformed);
        }
        let mut valid = Vec::with_capacity(units.len());
        for id in units {
            let Some(e) = self.entity(*id) else {
                continue; // stale ids are dropped silently
            };
            let Some(u) = e.as_unit() else {
                continue;
            };
            if u.owner != player {
                // A foreign id in a big selection is a stink bomb; reject it.
                return Err(CommandError::NotOwner);
            }
            valid.push(*id);
        }
        if valid.is_empty() {
            return Err(CommandError::EntityNotFound);
        }
        Ok(valid)
    }

    fn try_apply(
        &mut self,
        player: PlayerId,
        command: Command,
    ) -> Result<CommandResult, CommandError> {
        self.require_active(player)?;

        match command {
            Command::Chat { text } => {
                if text.len() > MAX_CHAT_LEN * 4 {
                    return Err(CommandError::Malformed);
                }
                // Chat is broadcast by the network layer; the core just
                // acknowledges it.
                Ok(CommandResult::ok())
            }
            Command::Resign => {
                self.player_mut(player).resigned();
                self.emit(crate::entity::GameEvent::PlayerEliminated { owner: player });
                self.check_victory();
                Ok(CommandResult::ok())
            }
            Command::AdvanceEpoch => {
                let epoch = self.try_advance_epoch(player)?;
                let mut r = CommandResult::ok();
                r.epoch = Some(epoch);
                Ok(r)
            }
            Command::ChooseCiv { slot, value } => {
                self.choose_civ(player, slot, value)?;
                Ok(CommandResult::ok())
            }
            Command::Train { building, unit } => {
                self.queue_unit(player, building, unit)?;
                Ok(CommandResult::ok())
            }
            Command::CancelProduction { building, index } => {
                self.cancel_production(player, building, index)?;
                Ok(CommandResult::ok())
            }
            Command::PlaceBuilding { building, origin } => {
                self.place_building(player, building, origin)?;
                Ok(CommandResult::ok())
            }
            Command::CancelBuilding { building } => {
                self.cancel_building(player, building)?;
                Ok(CommandResult::ok())
            }
            Command::Move { units, target } => {
                let valid = self.owned_units(player, &units)?;
                self.order_move(player, &valid, target, false)?;
                Ok(CommandResult::ok())
            }
            Command::AttackMove { units, target } => {
                let valid = self.owned_units(player, &units)?;
                self.order_move(player, &valid, target, true)?;
                Ok(CommandResult::ok())
            }
            Command::Attack { units, target } => {
                let valid = self.owned_units(player, &units)?;
                self.order_attack(player, &valid, target)?;
                Ok(CommandResult::ok())
            }
            Command::Gather { units, node } => {
                let valid = self.owned_units(player, &units)?;
                self.order_gather(player, &valid, node)?;
                Ok(CommandResult::ok())
            }
            Command::Assist { units, target } => {
                let valid = self.owned_units(player, &units)?;
                self.order_assist(player, &valid, target)?;
                Ok(CommandResult::ok())
            }
            Command::Stop { units } => {
                let valid = self.owned_units(player, &units)?;
                self.order_stop(player, &valid);
                Ok(CommandResult::ok())
            }
            Command::Hold { units } => {
                let valid = self.owned_units(player, &units)?;
                self.order_hold(player, &valid);
                Ok(CommandResult::ok())
            }
            Command::SetRally { building, target } => {
                self.owned_building(player, building)?;
                if !self.map.in_bounds_pos(target) {
                    return Err(CommandError::OutOfBounds);
                }
                let rally = target.center();
                if let Some(b) = self.entity_mut(building).and_then(|e| e.as_building_mut()) {
                    b.rally = Some(rally);
                }
                Ok(CommandResult::ok())
            }
        }
    }

    fn choose_civ(
        &mut self,
        player: PlayerId,
        slot: CivSlot,
        value: u16,
    ) -> Result<(), CommandError> {
        let epoch = self.player(player).epoch;
        let required = match slot {
            CivSlot::Culture => Epoch::Bronze,
            CivSlot::Branch => Epoch::EarlyMedieval,
            CivSlot::Country => Epoch::Industry1,
        };
        if epoch < required {
            return Err(CommandError::EpochTooLow);
        }
        let civ = &mut self.player_mut(player).civ;
        if !civ.choose(slot, value) {
            return Err(CommandError::InvalidCivChoice);
        }
        let slot_u8 = match slot {
            CivSlot::Culture => 0u8,
            CivSlot::Branch => 1,
            CivSlot::Country => 2,
        };
        self.emit(crate::entity::GameEvent::CivChosen {
            owner: player,
            slot: slot_u8,
            value,
        });
        Ok(())
    }

    fn queue_unit(
        &mut self,
        player: PlayerId,
        building: EntityId,
        unit: crate::ids::UnitDefId,
    ) -> Result<(), CommandError> {
        let player_epoch = self.player(player).epoch;
        let unit_def = self
            .catalog
            .units
            .get(unit.0 as usize)
            .ok_or(CommandError::EntityNotFound)?
            .clone();

        let (owner, finished, trains, queue_len) = {
            let b = self.owned_building(player, building)?;
            if !b.finished {
                return Err(CommandError::InvalidOrder);
            }
            (
                b.owner,
                b.finished,
                self.catalog.building(b.def).trains,
                b.queue.len(),
            )
        };
        debug_assert!(finished);
        if owner != player {
            return Err(CommandError::NotOwner);
        }
        if !trains.contains(unit) {
            return Err(CommandError::InvalidOrder);
        }
        if queue_len >= MAX_QUEUE {
            return Err(CommandError::InvalidOrder);
        }
        if unit_def.epoch > player_epoch {
            return Err(CommandError::EpochTooLow);
        }
        if !self.player(player).has_population_room(unit_def.pop) {
            return Err(CommandError::PopulationCap);
        }
        let cost = unit_def.effective_cost();
        if !self.player_mut(player).pay(cost) {
            return Err(CommandError::InsufficientResources);
        }
        let item = crate::entity::ProductionItem {
            unit,
            ticks_left: unit_def.build_ticks as u32,
            paid: true,
        };
        if let Some(b) = self.entity_mut(building).and_then(|e| e.as_building_mut()) {
            b.queue.push(item);
        }
        Ok(())
    }

    fn cancel_production(
        &mut self,
        player: PlayerId,
        building: EntityId,
        index: u8,
    ) -> Result<(), CommandError> {
        let (owner, item) = {
            let b = self.owned_building(player, building)?;
            let item = *b
                .queue
                .get(index as usize)
                .ok_or(CommandError::InvalidOrder)?;
            (b.owner, item)
        };
        if owner != player {
            return Err(CommandError::NotOwner);
        }
        let cost = self.catalog.unit(item.unit).effective_cost();
        if let Some(b) = self.entity_mut(building).and_then(|e| e.as_building_mut()) {
            b.queue.remove(index as usize);
        }
        if item.paid {
            self.player_mut(player).refund(cost);
        }
        Ok(())
    }

    fn place_building(
        &mut self,
        player: PlayerId,
        def_id: crate::ids::BuildingDefId,
        origin: TilePos,
    ) -> Result<(), CommandError> {
        let def = self
            .catalog
            .buildings
            .get(def_id.0 as usize)
            .ok_or(CommandError::EntityNotFound)?
            .clone();
        let player_epoch = self.player(player).epoch;
        if def.required_epoch > player_epoch {
            return Err(CommandError::EpochTooLow);
        }
        if !self.map.in_bounds_pos(origin) {
            return Err(CommandError::OutOfBounds);
        }
        if !self.fits_footprint(origin, def.footprint) {
            return Err(CommandError::OutOfBounds);
        }
        // Buildings must be placed near an existing building or worker.
        if !self.near_own_structure(player, origin, def.footprint) {
            return Err(CommandError::Unreachable);
        }
        if def.role == BuildingRole::Wonder {
            let has_wonder = self.entities.iter().any(|(_, e)| {
                matches!(e, Entity::Building(b) if b.owner == player && self.catalog.building(b.def).role == BuildingRole::Wonder)
            });
            if has_wonder {
                return Err(CommandError::PrerequisiteMissing);
            }
        }
        let cost = def.cost;
        if !self.player_mut(player).pay(cost) {
            return Err(CommandError::InsufficientResources);
        }
        let id = self.spawn_building(def_id, player, origin, false)?;
        self.emit(crate::entity::GameEvent::BuildingPlaced {
            owner: player,
            entity: id,
        });
        Ok(())
    }

    /// Construction is allowed within a radius of the player's own buildings.
    fn near_own_structure(&self, player: PlayerId, origin: TilePos, footprint: i32) -> bool {
        const PLACEMENT_RADIUS: i32 = 14;
        let center = TilePos::new(origin.x + footprint / 2, origin.y + footprint / 2);
        self.entities.iter().any(|(_, e)| {
            if let Entity::Building(b) = e {
                if b.owner == player {
                    return b.origin.dist(center) <= PLACEMENT_RADIUS + footprint;
                }
            }
            matches!(e, Entity::Unit(u) if u.owner == player && TilePos::from_vec(u.pos).dist(center) <= PLACEMENT_RADIUS)
        })
    }

    fn cancel_building(
        &mut self,
        player: PlayerId,
        building: EntityId,
    ) -> Result<(), CommandError> {
        let (def, refund) = {
            let b = self.owned_building(player, building)?;
            if b.finished {
                return Err(CommandError::InvalidOrder);
            }
            (b.def, self.catalog.building(b.def).cost)
        };
        let origin = self
            .building(building)
            .map(|b| (b.origin, self.catalog.building(def).footprint))
            .ok_or(CommandError::EntityNotFound)?;
        self.map.set_blocked_rect(origin.0, origin.1, false);
        self.entities.remove(building);
        // 75% refund.
        let refund = crate::resource::Cost {
            food: refund.food * 3 / 4,
            wood: refund.wood * 3 / 4,
            stone: refund.stone * 3 / 4,
            copper: refund.copper * 3 / 4,
            iron: refund.iron * 3 / 4,
            gold: refund.gold * 3 / 4,
        };
        self.player_mut(player).refund(refund);
        self.recompute_player_caches();
        Ok(())
    }

    fn order_move(
        &mut self,
        player: PlayerId,
        units: &[EntityId],
        target: TilePos,
        attack_move: bool,
    ) -> Result<(), CommandError> {
        if !self.map.in_bounds_pos(target) {
            return Err(CommandError::OutOfBounds);
        }
        let goal = self
            .map
            .nearest_walkable(target, 16)
            .ok_or(CommandError::Unreachable)?;
        // Spread the group in a deterministic grid around the goal so units
        // do not stack on a single tile.
        let cols = (units.len() as f64).sqrt().ceil() as i32;
        let cols = cols.max(1);
        for (i, id) in units.iter().enumerate() {
            let Some(u) = self.entity(*id).and_then(|e| e.as_unit()) else {
                continue;
            };
            if u.owner != player {
                continue;
            }
            let offset = grid_offset(i as i32, cols);
            let candidate = TilePos::new(goal.x + offset.0, goal.y + offset.1);
            let dest = self.map.nearest_walkable(candidate, 6).unwrap_or(goal);
            let start = TilePos::from_vec(u.pos);
            let path = if start == dest {
                Vec::new()
            } else {
                self.pathfinder
                    .find_path(&self.map, start, dest, 6000)
                    .unwrap_or_default()
            };
            if let Some(u) = self.entity_mut(*id).and_then(|e| e.as_unit_mut()) {
                u.clear_path();
                u.path = path;
                u.target = None;
                u.gather_node = None;
                u.order = if attack_move {
                    Order::AttackMove {
                        target: dest.center(),
                    }
                } else {
                    Order::Move {
                        target: dest.center(),
                    }
                };
                u.state = UnitState::Moving;
            }
        }
        Ok(())
    }

    fn order_attack(
        &mut self,
        player: PlayerId,
        units: &[EntityId],
        target: EntityId,
    ) -> Result<(), CommandError> {
        let target_entity = self.entity(target).ok_or(CommandError::EntityNotFound)?;
        if target_entity.owner() == player {
            return Err(CommandError::NotOwner);
        }
        for id in units {
            if let Some(u) = self.entity_mut(*id).and_then(|e| e.as_unit_mut()) {
                u.order = Order::Attack { target };
                u.target = Some(target);
                u.state = UnitState::Attacking;
                u.gather_node = None;
                u.clear_path();
            }
        }
        Ok(())
    }

    fn order_gather(
        &mut self,
        player: PlayerId,
        units: &[EntityId],
        node: u32,
    ) -> Result<(), CommandError> {
        let Some(n) = self.map.node(node as usize) else {
            return Err(CommandError::EntityNotFound);
        };
        if n.amount <= 0 {
            return Err(CommandError::Unreachable);
        }
        let kind = n.kind;
        if !resource_available(self.player(player).epoch, kind) {
            return Err(CommandError::EpochTooLow);
        }
        let pos = n.pos;
        let mut assigned = 0;
        for id in units {
            let Some(is_worker) = self
                .unit(*id)
                .map(|u| self.catalog.unit(u.def).key == "worker")
                .filter(|_| true)
            else {
                continue;
            };
            if !is_worker {
                continue;
            }
            // Workers first walk to a tile adjacent to the node.
            let approach = self.map.nearest_walkable(pos, 6).unwrap_or(pos);
            let start = self
                .unit(*id)
                .map(|u| TilePos::from_vec(u.pos))
                .unwrap_or(TilePos::new(0, 0));
            let path = self
                .pathfinder
                .find_path(&self.map, start, approach, 4000)
                .unwrap_or_default();
            if let Some(u) = self.entity_mut(*id).and_then(|e| e.as_unit_mut()) {
                u.order = Order::Gather {
                    node: node as usize,
                };
                u.gather_node = Some(node as usize);
                u.target = None;
                u.clear_path();
                u.path = path;
                u.state = UnitState::Gathering;
                u.cargo = Cargo {
                    kind: Some(kind),
                    amount: 0,
                };
            }
            assigned += 1;
        }
        if assigned == 0 {
            return Err(CommandError::InvalidOrder);
        }
        Ok(())
    }

    fn order_assist(
        &mut self,
        player: PlayerId,
        units: &[EntityId],
        target: EntityId,
    ) -> Result<(), CommandError> {
        // `b` is a shared borrow used only for the target position.
        let (target_pos, target_owner) = {
            let b = self.building(target).ok_or(CommandError::EntityNotFound)?;
            (b.pos, b.owner)
        };
        if target_owner != player {
            return Err(CommandError::NotOwner);
        }
        for id in units {
            let Some(u) = self.unit(*id) else { continue };
            if self.catalog.unit(u.def).build_work == 0 {
                continue;
            }
            let start = TilePos::from_vec(u.pos);
            let dest = self
                .map
                .nearest_walkable(TilePos::from_vec(target_pos), 6)
                .unwrap_or(TilePos::from_vec(target_pos));
            let path = self
                .pathfinder
                .find_path(&self.map, start, dest, 3000)
                .unwrap_or_default();
            if let Some(u) = self.entity_mut(*id).and_then(|e| e.as_unit_mut()) {
                u.order = Order::Build { target };
                u.clear_path();
                u.path = path;
                u.gather_node = None;
                u.state = UnitState::Building;
            }
        }
        Ok(())
    }

    fn order_stop(&mut self, player: PlayerId, units: &[EntityId]) {
        for id in units {
            if let Some(u) = self.entity_mut(*id).and_then(|e| e.as_unit_mut()) {
                if u.owner != player {
                    continue;
                }
                u.order = Order::Idle;
                u.target = None;
                u.gather_node = None;
                u.clear_path();
                u.state = UnitState::Idle;
            }
        }
    }

    fn order_hold(&mut self, player: PlayerId, units: &[EntityId]) {
        for id in units {
            if let Some(u) = self.entity_mut(*id).and_then(|e| e.as_unit_mut()) {
                if u.owner != player {
                    continue;
                }
                u.order = Order::Hold;
                u.clear_path();
                u.state = UnitState::Idle;
            }
        }
    }

    /// Maximum number of players for the configured map size.
    pub fn player_limit(&self) -> u16 {
        match self.config.map_size {
            MapSize::Duel => 2,
            MapSize::Small => 8,
            MapSize::Standard => 24,
            MapSize::Large => self.config.max_players,
        }
    }

    pub fn is_over(&self) -> bool {
        self.outcome != MatchOutcome::Ongoing
    }

    /// Convenience: total unit count, for tests and metrics.
    pub fn unit_count(&self) -> usize {
        self.entities.iter().filter(|(_, e)| e.is_unit()).count()
    }

    pub fn building_count(&self) -> usize {
        self.entities
            .iter()
            .filter(|(_, e)| e.is_building())
            .count()
    }

    /// Sum of all stockpiles, a cheap "economy health" metric.
    pub fn total_resources(&self, owner: PlayerId) -> i64 {
        self.player(owner).resources.total()
    }

    pub fn gather_node_for(&self, id: EntityId) -> Option<usize> {
        self.unit(id).and_then(|u| u.gather_node)
    }

    pub fn worker_def() -> crate::ids::UnitDefId {
        crate::ids::UnitDefId(UNIT_WORKER)
    }

    pub(crate) fn update_population(&mut self, owner: PlayerId) {
        let pop: i32 = self
            .entities
            .iter()
            .filter_map(|(_, e)| e.as_unit())
            .filter(|u| u.owner == owner)
            .map(|u| self.catalog.unit(u.def).pop)
            .sum();
        let cap = self.provided_population(owner);
        let hard = self.config.hard_pop_cap;
        let p = self.player_mut(owner);
        p.population = pop;
        p.pop_cap = cap.max(5).min(hard);
    }

    pub(crate) fn update_all_population(&mut self) {
        let ids: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| !p.id.is_gaia())
            .map(|p| p.id)
            .collect();
        for id in ids {
            self.update_population(id);
        }
    }

    /// Entities near a position (deterministic order).
    pub fn query_nearby(&mut self, pos: Vec2, radius: i32) -> Vec<EntityId> {
        let mut out = Vec::new();
        self.grid.query_radius(pos, radius, &mut out);
        out.retain(|id| self.entity(*id).is_some());
        out
    }

    pub(crate) fn building_center(&self, id: EntityId) -> Option<crate::math::Vec2> {
        let b = self.building(id)?;
        Some(building_center(
            b.origin,
            self.catalog.building(b.def).footprint,
        ))
    }
}

/// Deterministic grid offsets for group movement.
fn grid_offset(index: i32, cols: i32) -> (i32, i32) {
    let row = index / cols;
    let col = index % cols;
    (col - cols / 2, row - cols / 2)
}
