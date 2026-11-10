//! A small deterministic bot used by the CLI demo and the dev server.
//!
//! The bot only issues ordinary [`Command`]s and has no privileged access to
//! the world. That keeps the server authoritative and makes bot matches
//! replayable exactly like human matches.

use crate::command::Command;
use crate::defs::{BuildingRole, BUILDING_HOUSE, UNIT_WORKER};
use crate::entity::{Entity, Order, UnitState};
use crate::epoch::{Branch, CivSlot, Country, Culture, Epoch};
use crate::ids::{BuildingDefId, EntityId, PlayerId, UnitDefId};
use crate::math::{TilePos, Vec2};
use crate::resource::ResourceKind;
use crate::world::World;

/// A cheap "good enough" bot: economy first, then the best unit it can afford,
/// then periodic attacks at the nearest enemy.
#[derive(Clone, Debug)]
pub struct SimpleBot {
    last_attack_tick: u64,
    last_build_tick: u64,
    last_epoch_tick: u64,
    last_civ_tick: u64,
    /// Epoch during which the bot is deliberately stockpiling resources.
    save_epoch: Option<Epoch>,
}

impl Default for SimpleBot {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleBot {
    pub fn new() -> Self {
        SimpleBot {
            last_attack_tick: 0,
            last_build_tick: 0,
            last_epoch_tick: 0,
            last_civ_tick: 0,
            save_epoch: None,
        }
    }

    /// Produce the commands the bot wants to issue this decision tick.
    /// The caller applies them in order and remains the authority.
    pub fn commands(&mut self, world: &World, player: PlayerId) -> Vec<Command> {
        let mut out = Vec::new();
        let p = world.player(player);
        if !p.is_active() {
            return out;
        }
        let tick = world.tick().0;
        let epoch = p.epoch;

        let Some((tc_id, tc_pos)) = town_center(world, player) else {
            return out;
        };

        // While the next epoch is unaffordable, stop buying units and let the
        // economy save up. Without this the bot spends every scrap of food on
        // clubmen and never reaches Bronze.
        let epoch_req = if epoch.next().is_some() {
            Some(epoch.advance_requirement())
        } else {
            None
        };
        // Latch "saving" until the epoch actually changes. Recomputing it
        // every tick makes the bot spend the stockpile on units the moment it
        // briefly crosses the threshold, so the advance check never sees it.
        if self.save_epoch.map_or(false, |saved| saved != epoch) {
            self.save_epoch = None;
        }
        if epoch_req.is_none() {
            self.save_epoch = None;
        }
        if let Some(req) = epoch_req {
            if !world.can_afford(player, req.resources) {
                self.save_epoch = Some(epoch);
            }
        }
        let saving_for_epoch = epoch_req.is_some() && self.save_epoch == Some(epoch);
        // After two minutes the demo should produce a winner rather than two
        // turtles stockpiling for the next age forever.
        let late_game = tick >= 20 * 120;

        let finished_buildings = world
            .entities
            .iter()
            .filter(|(_, e)| matches!(e, Entity::Building(b) if b.owner == player && b.finished))
            .count() as u16;
        let worker_count = count_workers(world, player) as u16;

        // -- civilization choices -----------------------------------------
        if tick >= self.last_civ_tick + 100 {
            self.last_civ_tick = tick;
            if epoch >= Epoch::Bronze && p.civ.culture == Culture::Unchosen {
                // Alternate Europe / Asia so demo matches show both paths.
                let value = if player.0 % 2 == 0 { 1 } else { 2 };
                out.push(Command::ChooseCiv {
                    slot: CivSlot::Culture,
                    value,
                });
            } else if epoch >= Epoch::EarlyMedieval
                && p.civ.culture != Culture::Unchosen
                && p.civ.branch == Branch::Unchosen
            {
                out.push(Command::ChooseCiv {
                    slot: CivSlot::Branch,
                    value: 1,
                });
            } else if epoch >= Epoch::Industry1
                && p.civ.culture != Culture::Unchosen
                && p.civ.country == Country::Unchosen
            {
                let value = if p.civ.culture == Culture::Europe {
                    1
                } else {
                    5
                };
                out.push(Command::ChooseCiv {
                    slot: CivSlot::Country,
                    value,
                });
            }
        }

        // -- epoch advance (before any spending commands) -------------------
        if let Some(req) = epoch_req {
            if tick >= self.last_epoch_tick + 200 {
                self.last_epoch_tick = tick;
                if finished_buildings >= req.buildings
                    && worker_count >= req.workers
                    && world.can_afford(player, req.resources)
                {
                    out.push(Command::AdvanceEpoch);
                }
            }
        }

        // -- construction sites --------------------------------------------
        let mut idle = idle_workers(world, player);
        let unfinished_site = world.entities.iter().find_map(|(id, e)| match e {
            Entity::Building(b) if b.owner == player && !b.finished => Some(id),
            _ => None,
        });
        if let Some(site) = unfinished_site {
            if !idle.is_empty() {
                let crew: Vec<EntityId> = idle.iter().take(3).map(|(id, _)| *id).collect();
                out.push(Command::Assist {
                    units: crew.clone(),
                    target: site,
                });
                idle.retain(|(id, _)| !crew.contains(id));
            }
        }

        // -- idle workers -> nearest resource ------------------------------
        for (i, (id, pos)) in idle.into_iter().enumerate() {
            // Roughly 50% food, 25% wood, 25% stone: enough food for units
            // and enough stone/wood for houses and epoch requirements.
            let kinds = match i % 4 {
                0 | 1 => [ResourceKind::Food, ResourceKind::Wood, ResourceKind::Stone],
                2 => [ResourceKind::Wood, ResourceKind::Food, ResourceKind::Stone],
                _ => [ResourceKind::Stone, ResourceKind::Food, ResourceKind::Wood],
            };
            for kind in kinds {
                if let Some(node) = nearest_node(world, pos, kind, 80) {
                    out.push(Command::Gather {
                        units: vec![id],
                        node: node as u32,
                    });
                    break;
                }
            }
        }

        // -- housing --------------------------------------------------------
        let pop_room = p.pop_cap - p.population;
        let house_cost = world.catalog.building(BuildingDefId(BUILDING_HOUSE)).cost;
        let house_under_construction = world.entities.iter().any(|(_, e)| {
            matches!(e, Entity::Building(b)
                if b.owner == player
                    && !b.finished
                    && world.catalog.building(b.def).role == BuildingRole::House)
        });
        let need_buildings = epoch_req.map_or(false, |req| finished_buildings < req.buildings);
        if (pop_room <= 4 || need_buildings)
            && (!saving_for_epoch || need_buildings || late_game)
            && !house_under_construction
            && tick >= self.last_build_tick + 200
            && world.can_afford(player, house_cost)
        {
            if let Some(origin) = find_build_spot(world, tc_pos) {
                self.last_build_tick = tick;
                out.push(Command::PlaceBuilding {
                    building: BuildingDefId(BUILDING_HOUSE),
                    origin,
                });
            }
        }

        // -- workers from the town centre ----------------------------------
        let tc_queue = world.building(tc_id).map(|b| b.queue.len()).unwrap_or(0);
        let worker_floor = epoch_req.map(|req| req.workers).unwrap_or(0);
        let worker_target = 10u16.max(worker_floor);
        let worker_def = world.catalog.unit(UnitDefId(UNIT_WORKER));
        if tc_queue < 2
            && worker_count < worker_target
            && (!saving_for_epoch || worker_count < worker_floor)
            && p.has_population_room(worker_def.pop)
            && world.can_afford(player, worker_def.effective_cost())
        {
            out.push(Command::Train {
                building: tc_id,
                unit: UnitDefId(UNIT_WORKER),
            });
        }

        // -- military production -------------------------------------------
        for (bid, b) in world.entities.iter().filter_map(|(id, e)| match e {
            Entity::Building(b) if b.owner == player && b.finished => Some((id, b)),
            _ => None,
        }) {
            if saving_for_epoch && !late_game {
                continue;
            }
            let bdef = world.catalog.building(b.def);
            if !bdef.role.is_production() || bdef.role == BuildingRole::TownCenter {
                continue;
            }
            if b.queue.len() >= 2 {
                continue;
            }
            let mut best: Option<UnitDefId> = None;
            for unit in bdef.trains.iter() {
                let udef = world.catalog.unit(unit);
                if udef.epoch > epoch {
                    continue;
                }
                if !p.has_population_room(udef.pop) {
                    continue;
                }
                if !world.can_afford(player, udef.effective_cost()) {
                    continue;
                }
                best = Some(unit);
            }
            if let Some(unit) = best {
                out.push(Command::Train {
                    building: bid,
                    unit,
                });
            }
        }

        // -- attack ---------------------------------------------------------
        if tick >= self.last_attack_tick + 160 {
            let army: Vec<EntityId> = world
                .entities
                .iter()
                .filter_map(|(id, e)| match e {
                    Entity::Unit(u)
                        if u.owner == player
                            && u.hp > 0
                            && world.catalog.unit(u.def).role.is_military() =>
                    {
                        Some(id)
                    }
                    _ => None,
                })
                .collect();
            if army.len() >= 4 {
                if let Some(target) = nearest_enemy(world, player, tc_pos) {
                    self.last_attack_tick = tick;
                    out.push(Command::Attack {
                        units: army,
                        target,
                    });
                }
            }
        }

        out
    }
}

fn town_center(world: &World, player: PlayerId) -> Option<(EntityId, Vec2)> {
    world.entities.iter().find_map(|(id, e)| match e {
        Entity::Building(b)
            if b.owner == player
                && b.finished
                && world.catalog.building(b.def).role == BuildingRole::TownCenter =>
        {
            Some((id, b.pos))
        }
        _ => None,
    })
}

fn idle_workers(world: &World, player: PlayerId) -> Vec<(EntityId, Vec2)> {
    world
        .entities
        .iter()
        .filter_map(|(id, e)| match e {
            Entity::Unit(u)
                if u.owner == player
                    && world.catalog.unit(u.def).role.is_worker()
                    && u.order == Order::Idle
                    && u.state != UnitState::Dying =>
            {
                Some((id, u.pos))
            }
            _ => None,
        })
        .collect()
}

fn count_workers(world: &World, player: PlayerId) -> usize {
    world
        .entities
        .iter()
        .filter(|(_, e)| {
            matches!(e, Entity::Unit(u)
                if u.owner == player && world.catalog.unit(u.def).role.is_worker())
        })
        .count()
}

fn nearest_node(world: &World, from: Vec2, kind: ResourceKind, max_radius: i32) -> Option<usize> {
    let origin = TilePos::from_vec(from);
    world
        .map
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == kind && n.amount > 0)
        .map(|(i, n)| (origin.dist(n.pos), i))
        .filter(|(dist, _)| *dist <= max_radius)
        .min()
        .map(|(_, i)| i)
}

fn find_build_spot(world: &World, center: Vec2) -> Option<TilePos> {
    let c = TilePos::from_vec(center);
    for r in 1..=14i32 {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let p = TilePos::new(c.x + dx, c.y + dy);
                if world.fits_footprint(p, 1) {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn nearest_enemy(world: &World, player: PlayerId, from: Vec2) -> Option<EntityId> {
    world
        .entities
        .iter()
        .filter(|(_, e)| e.owner() != player && !e.owner().is_gaia() && e.is_alive())
        .map(|(id, e)| {
            (
                Vec2::dist(from, e.pos()).to_int(),
                if e.is_building() { 0u8 } else { 1u8 },
                id,
            )
        })
        .min()
        .map(|(_, _, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn bot_runs_an_economy() {
        let mut world = World::skirmish(7, 2);
        let mut bots = [SimpleBot::new(), SimpleBot::new()];
        for tick in 0..2400 {
            if tick % 10 == 0 {
                for (i, bot) in bots.iter_mut().enumerate() {
                    let player = PlayerId((i + 1) as u16);
                    for command in bot.commands(&world, player) {
                        let _ = world.apply_command(player, command);
                    }
                }
            }
            world.step();
        }
        assert!(
            world.player(PlayerId(1)).gathered > 0,
            "the bot should have delivered resources"
        );
        assert!(world.unit_count() > 0);
    }
}
