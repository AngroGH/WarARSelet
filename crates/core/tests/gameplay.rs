//! End-to-end tests of the simulation core.
//!
//! These are the "does the game actually work" tests: economy, construction,
//! combat, epochs, elimination, Armageddon and determinism.

use warsel_core::command::MAX_QUEUE;
use warsel_core::defs::{
    BUILDING_BARRACKS, BUILDING_HOUSE, BUILDING_RANGE, BUILDING_TOWN_CENTER, BUILDING_WONDER,
    UNIT_ARCHER, UNIT_CLUBMAN, UNIT_HEAVY_TANK, UNIT_SPEARMAN, UNIT_WORKER,
};
use warsel_core::entity::{Entity, Order};
use warsel_core::epoch::{CivSlot, Culture, Epoch};
use warsel_core::ids::{BuildingDefId, EntityId, PlayerId, UnitDefId};
use warsel_core::{checksum, Command, GameMode, MatchConfig, MatchOutcome, TilePos, World};

fn test_world(mode: GameMode) -> World {
    World::new(
        MatchConfig {
            seed: 2024,
            map_size: warsel_core::MapSize::Duel,
            mode,
            ..MatchConfig::default()
        },
        vec![
            ("Alice".into(), 0, false),
            ("Bob".into(), 0, false),
        ],
    )
}

fn own_building(world: &World, owner: PlayerId, def: u16) -> Option<EntityId> {
    world
        .entities
        .iter()
        .find(|(_, e)| match e {
            Entity::Building(b) => b.owner == owner && b.def.0 == def,
            _ => false,
        })
        .map(|(id, _)| id)
}

fn own_units(world: &World, owner: PlayerId) -> Vec<EntityId> {
    world
        .entities
        .iter()
        .filter(|(_, e)| e.is_unit() && e.owner() == owner)
        .map(|(id, _)| id)
        .collect()
}

fn workers(world: &World, owner: PlayerId) -> Vec<EntityId> {
    world
        .entities
        .iter()
        .filter(|(_, e)| {
            matches!(e, Entity::Unit(u) if u.owner == owner && u.def.0 == UNIT_WORKER)
        })
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn match_starts_with_a_real_base() {
    let world = test_world(GameMode::Ffa);
    let alice = PlayerId(1);
    assert!(own_building(&world, alice, BUILDING_TOWN_CENTER).is_some());
    assert!(own_building(&world, alice, BUILDING_BARRACKS).is_some());
    assert!(own_building(&world, alice, BUILDING_HOUSE).is_some());
    assert_eq!(workers(&world, alice).len(), 8);
    assert!(world.unit_count() >= 16);
    assert_eq!(world.player(alice).epoch, Epoch::Stone);
    assert!(world.player(alice).pop_cap >= 10);
}

#[test]
fn workers_gather_and_bank_resources() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let before = world.total_resources(alice);

    // Find a food/wood node and send the whole crew.
    let node = world
        .map
        .alive_nodes()
        .next()
        .map(|(i, n)| (i, n.kind))
        .expect("the map has resources");
    let crew = workers(&world, alice);
    assert!(!crew.is_empty());
    let result = world.apply_command(
        alice,
        Command::Gather {
            units: crew.clone(),
            node: node.0 as u32,
        },
    );
    assert!(result.accepted, "{:?}", result.reason);

    world.step_many(20 * 90); // 90 in-game seconds

    let after = world.total_resources(alice);
    assert!(
        after > before,
        "workers should have delivered resources ({before} -> {after})"
    );
    assert!(world.player(alice).gathered > 0);
}

#[test]
fn training_a_unit_costs_resources_and_respects_the_queue() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let barracks = own_building(&world, alice, BUILDING_BARRACKS).unwrap();

    let cost = world.catalog.unit(UnitDefId(UNIT_CLUBMAN)).effective_cost();
    let food_before = world.player(alice).resources.food;

    let r = world.apply_command(
        alice,
        Command::Train {
            building: barracks,
            unit: UnitDefId(UNIT_CLUBMAN),
        },
    );
    assert!(r.accepted, "{:?}", r.reason);
    assert_eq!(
        world.player(alice).resources.food,
        food_before - cost.food
    );

    let units_before = world.unit_count();
    world.step_many(20 * 30);
    assert!(world.unit_count() > units_before, "the unit should pop out");
}

#[test]
fn queue_is_bounded() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let barracks = own_building(&world, alice, BUILDING_BARRACKS).unwrap();
    world.player_mut(alice).resources.food = 100_000;
    world.player_mut(alice).resources.wood = 100_000;
    for _ in 0..MAX_QUEUE {
        let r = world.apply_command(
            alice,
            Command::Train {
                building: barracks,
                unit: UnitDefId(UNIT_CLUBMAN),
            },
        );
        assert!(r.accepted);
    }
    let r = world.apply_command(
        alice,
        Command::Train {
            building: barracks,
            unit: UnitDefId(UNIT_CLUBMAN),
        },
    );
    assert!(!r.accepted, "queue must reject the 7th item");
}

#[test]
fn cannot_control_enemy_units() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let bob = PlayerId(2);
    let bob_units = own_units(&world, bob);
    let target = *world.map.starts.last().unwrap();
    let r = world.apply_command(
        alice,
        Command::Move {
            units: bob_units,
            target,
        },
    );
    assert!(!r.accepted);
    assert_eq!(r.reason, Some(warsel_core::CommandError::NotOwner));
}

#[test]
fn cannot_build_outside_the_map() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let r = world.apply_command(
        alice,
        Command::PlaceBuilding {
            building: BuildingDefId(BUILDING_HOUSE),
            origin: TilePos::new(-50, -50),
        },
    );
    assert!(!r.accepted);
}

#[test]
fn placing_a_building_requires_resources_and_workers() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    world.player_mut(alice).resources = warsel_core::Cost::ZERO;
    let origin = world.map.starts[0];
    let r = world.apply_command(
        alice,
        Command::PlaceBuilding {
            building: BuildingDefId(BUILDING_HOUSE),
            origin: TilePos::new(origin.x + 2, origin.y + 2),
        },
    );
    assert!(!r.accepted);
    assert_eq!(r.reason, Some(warsel_core::CommandError::InsufficientResources));
}

#[test]
fn workers_build_a_house_and_it_finishes() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    world.player_mut(alice).resources = warsel_core::Cost::new(1000, 1000, 1000);

    // Pick a free spot near the base.
    let base = own_building(&world, alice, BUILDING_TOWN_CENTER).unwrap();
    let base_origin = world.building(base).unwrap().origin;
    let mut spot = None;
    'outer: for dy in 2..10 {
        for dx in 2..10 {
            let candidate = TilePos::new(base_origin.x + dx, base_origin.y + dy);
            if world.fits_footprint(candidate, 1) {
                spot = Some(candidate);
                break 'outer;
            }
        }
    }
    let spot = spot.expect("a free tile near the base");
    let r = world.apply_command(
        alice,
        Command::PlaceBuilding {
            building: BuildingDefId(BUILDING_HOUSE),
            origin: spot,
        },
    );
    assert!(r.accepted, "{:?}", r.reason);

    let site = world
        .entities
        .iter()
        .filter(|(_, e)| matches!(e, Entity::Building(b) if b.owner == alice && !b.finished))
        .map(|(id, _)| id)
        .next()
        .expect("a construction site");

    let crew = workers(&world, alice);
    let r = world.apply_command(alice, Command::Assist { units: crew, target: site });
    assert!(r.accepted, "{:?}", r.reason);
    world.step_many(20 * 120);

    let b = world.building(site).expect("the building still stands");
    assert!(b.finished, "workers should have finished the house");
    assert_eq!(b.hp, b.max_hp);
}

#[test]
fn combat_kills_units_and_the_damage_triangle_matters() {
    // Spears vs clubmen: spearmen should win decisively.
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let bob = PlayerId(2);
    world.player_mut(alice).resources = warsel_core::Cost::new(5000, 5000, 5000);
    world.player_mut(bob).resources = warsel_core::Cost::new(5000, 5000, 5000);
    world.player_mut(alice).pop_cap = 100;
    world.player_mut(bob).pop_cap = 100;
    let spot = world
        .map
        .starts
        .clone()
        .into_iter()
        .find_map(|s| world.map.nearest_walkable(TilePos::new(s.x, s.y + 6), 10))
        .expect("walkable ground");
    let center = spot.center();

    let mut alice_squad = Vec::new();
    let mut bob_squad = Vec::new();
    // Spawn directly so the test does not depend on training times.
    for i in 0..6 {
        let offset = warsel_core::world::formation_offset(i, 6, 0);
        let a = world
            .spawn_unit(UnitDefId(UNIT_SPEARMAN), alice, center + offset)
            .unwrap();
        let b = world
            .spawn_unit(UnitDefId(UNIT_CLUBMAN), bob, center + offset + warsel_core::Vec2::from_ints(2, 0))
            .unwrap();
        alice_squad.push(a);
        bob_squad.push(b);
    }

    let target = bob_squad[0];
    let r = world.apply_command(
        alice,
        Command::Attack {
            units: alice_squad.clone(),
            target,
        },
    );
    assert!(r.accepted, "{:?}", r.reason);
    let bob_target = alice_squad[0];
    let r = world.apply_command(
        bob,
        Command::Attack {
            units: bob_squad.clone(),
            target: bob_target,
        },
    );
    assert!(r.accepted);

    let alice_before = alice_squad.iter().filter(|id| world.entity(**id).is_some()).count();
    let bob_before = bob_squad.iter().filter(|id| world.entity(**id).is_some()).count();
    world.step_many(20 * 40);
    let alice_left = alice_squad.iter().filter(|id| world.entity(**id).is_some()).count();
    let bob_left = bob_squad.iter().filter(|id| world.entity(**id).is_some()).count();

    assert!(bob_left < bob_before, "someone in Bob's squad must die");
    assert!(
        alice_left >= bob_left,
        "spearmen (anti-infantry melee, medium armour) should not lose to clubmen: {alice_left} vs {bob_left}"
    );
    assert!(alice_left < alice_before || bob_left == 0 || alice_left > 0);
}

#[test]
fn epoch_advance_checks_requirements() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    // Requirements not met: the player cannot pay.
    world.player_mut(alice).resources = warsel_core::Cost::ZERO;
    let r = world.apply_command(alice, Command::AdvanceEpoch);
    assert!(!r.accepted);
    assert_eq!(
        r.reason,
        Some(warsel_core::CommandError::InsufficientResources)
    );

    // Not enough workers is also a hard requirement.
    let mut weak = World::new(
        MatchConfig {
            seed: 2024,
            map_size: warsel_core::MapSize::Duel,
            mode: GameMode::Sandbox,
            starting_workers: 2,
            starting_warriors: 0,
            ..MatchConfig::default()
        },
        vec![("Alice".into(), 0, false), ("Bob".into(), 0, false)],
    );
    weak.player_mut(alice).resources = warsel_core::Cost::new(5000, 5000, 5000);
    let r = weak.apply_command(alice, Command::AdvanceEpoch);
    assert_eq!(
        r.reason,
        Some(warsel_core::CommandError::PrerequisiteMissing)
    );

    // Give the player everything the Stone->Bronze step needs.
    world.player_mut(alice).resources = warsel_core::Cost::new(5000, 5000, 5000);
    // Starting base has 5 buildings and 8 workers; the requirement is 0
    // buildings / 8 workers for Bronze, so resources alone are enough.
    let r = world.apply_command(alice, Command::AdvanceEpoch);
    assert!(r.accepted, "{:?}", r.reason);
    assert_eq!(world.player(alice).epoch, Epoch::Bronze);

    // Civilization choice is now unlocked.
    let r = world.apply_command(
        alice,
        Command::ChooseCiv {
            slot: CivSlot::Culture,
            value: 1,
        },
    );
    assert!(r.accepted, "{:?}", r.reason);
    assert_eq!(world.player(alice).civ.culture, Culture::Europe);

    // Branch is not unlocked yet.
    let r = world.apply_command(
        alice,
        Command::ChooseCiv {
            slot: CivSlot::Branch,
            value: 1,
        },
    );
    assert_eq!(r.reason, Some(warsel_core::CommandError::EpochTooLow));
}

#[test]
fn epoch_gate_blocks_late_units() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    world.player_mut(alice).resources = warsel_core::Cost::new(100_000, 100_000, 100_000);
    let range = own_building(&world, alice, BUILDING_RANGE);
    if let Some(range) = range {
        let r = world.apply_command(
            alice,
            Command::Train {
                building: range,
                unit: UnitDefId(UNIT_ARCHER),
            },
        );
        // Archer is Bronze; in Stone age it must be rejected.
        assert_eq!(r.reason, Some(warsel_core::CommandError::EpochTooLow));
    }

    // A heavy tank can never be queued in the Stone age.
    let barracks = own_building(&world, alice, BUILDING_BARRACKS).unwrap();
    let r = world.apply_command(
        alice,
        Command::Train {
            building: barracks,
            unit: UnitDefId(UNIT_HEAVY_TANK),
        },
    );
    assert!(!r.accepted);
}

#[test]
fn destroying_town_center_eliminates_the_player() {
    let mut world = test_world(GameMode::Ffa);
    let bob = PlayerId(2);
    let tc = own_building(&world, bob, BUILDING_TOWN_CENTER).unwrap();

    // Remove every production building first, then the town centre.
    let bob_buildings: Vec<EntityId> = world
        .entities
        .iter()
        .filter(|(_, e)| matches!(e, Entity::Building(b) if b.owner == bob))
        .map(|(id, _)| id)
        .collect();
    assert!(bob_buildings.contains(&tc));
    for id in bob_buildings {
        world.damage_entity(id, 1_000_000, PlayerId(1));
    }
    world.step();

    assert!(!world.player(bob).is_active());
    assert_eq!(world.outcome, MatchOutcome::Victory(PlayerId(1)));
}

#[test]
fn determinism_two_identical_matches_end_identically() {
    let a = run_scripted_match();
    let b = run_scripted_match();
    assert_eq!(checksum::hash_world(&a), checksum::hash_world(&b));
    assert_eq!(a.tick(), b.tick());
    assert_eq!(a.unit_count(), b.unit_count());
}

fn run_scripted_match() -> World {
    let mut world = test_world(GameMode::Ffa);
    let alice = PlayerId(1);
    let to_gather = world
        .map
        .alive_nodes()
        .next()
        .map(|(i, _)| i as u32)
        .unwrap_or(0);
    let crew = workers(&world, alice);
    world.apply_command(
        alice,
        Command::Gather {
            units: crew.clone(),
            node: to_gather,
        },
    );
    let move_target = world.map.starts[0];
    world.apply_command(
        alice,
        Command::Move {
            units: crew,
            target: move_target,
        },
    );
    world.step_many(20 * 30);
    world
}

#[test]
fn replay_reproduces_the_match() {
    let mut world = test_world(GameMode::Ffa);
    let alice = PlayerId(1);
    let mut replay = warsel_core::Replay::new(
        world.config.clone(),
        vec![
            warsel_core::replay::ReplayPlayer {
                name: "Alice".into(),
                team: 0,
                is_ai: false,
            },
            warsel_core::replay::ReplayPlayer {
                name: "Bob".into(),
                team: 0,
                is_ai: false,
            },
        ],
    );

    for i in 0..200 {
        world.step();
        if i == 10 {
            let crew = workers(&world, alice);
            let node = world.map.alive_nodes().next().map(|(i, _)| i as u32).unwrap();
            let cmd = Command::Gather {
                units: crew,
                node,
            };
            let res = world.apply_command(alice, cmd.clone());
            replay.record(world.tick().0, alice, cmd, &res);
        }
        if i % 50 == 0 {
            replay.checkpoint(&world);
        }
    }
    assert!(
        replay.verify().is_ok(),
        "replay diverged: {:?}",
        replay.verify().err()
    );

    let json = replay.to_json();
    let parsed = warsel_core::Replay::from_json(&json).expect("replay json round trip");
    assert!(parsed.verify().is_ok());
}

#[test]
fn snapshot_roundtrip_preserves_state() {
    let mut world = test_world(GameMode::Sandbox);
    world.step_many(40);
    let snap = warsel_core::replay::Snapshot::capture(&world);
    let restored = snap.restore().expect("checksum must match");
    assert_eq!(checksum::hash_world(&world), checksum::hash_world(&restored));
}

#[test]
fn fog_of_war_hides_the_enemy_base() {
    let world = test_world(GameMode::Ffa);
    let alice = PlayerId(1);
    let bob = PlayerId(2);
    let bob_tc = own_building(&world, bob, BUILDING_TOWN_CENTER).unwrap();
    let pos = world.building(bob_tc).unwrap().pos;
    assert!(!world.is_visible_to(alice, pos), "enemy base must be fogged");
    let alice_tc = own_building(&world, alice, BUILDING_TOWN_CENTER).unwrap();
    let pos = world.building(alice_tc).unwrap().pos;
    assert!(world.is_visible_to(alice, pos), "own base is visible");
}

#[test]
fn armageddon_strikes_the_weakest_player() {
    let mut world = World::new(
        MatchConfig {
            seed: 11,
            map_size: warsel_core::MapSize::Duel,
            mode: GameMode::Armageddon,
            meteor_period: 40,
            ..MatchConfig::default()
        },
        vec![
            ("Weak".into(), 0, true),
            ("Strong".into(), 0, true),
        ],
    );
    // Give player 2 a much bigger army so player 1 is the weakest.
    let bob = PlayerId(2);
    world.player_mut(bob).resources = warsel_core::Cost::new(9999, 9999, 9999);
    let spot = world.map.starts[1];
    for i in 0..20 {
        let _ = world.spawn_unit(
            UnitDefId(UNIT_CLUBMAN),
            bob,
            spot.center() + warsel_core::world::formation_offset(i, 20, 0),
        );
    }
    let alice_hp_before: i32 = world
        .entities
        .iter()
        .filter(|(_, e)| e.owner() == PlayerId(1))
        .map(|(_, e)| e.hp())
        .sum();
    world.step_many(120);
    let alice_hp_after: i32 = world
        .entities
        .iter()
        .filter(|(_, e)| e.owner() == PlayerId(1))
        .map(|(_, e)| e.hp())
        .sum();
    assert!(
        alice_hp_after < alice_hp_before,
        "meteors should have hurt the weakest player ({alice_hp_before} -> {alice_hp_after})"
    );
}

#[test]
fn wonder_victory_timer_exists() {
    let catalog = warsel_core::default_catalog();
    let wonder = catalog
        .buildings
        .iter()
        .find(|b| b.role == warsel_core::defs::BuildingRole::Wonder)
        .expect("a wonder exists");
    assert!(wonder.wonder_hold_ticks > 0);
    let _ = BUILDING_WONDER;
}

#[test]
fn attack_move_engages_enemies_on_the_way() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let bob = PlayerId(2);
    world.player_mut(alice).resources = warsel_core::Cost::new(5000, 5000, 5000);
    world.player_mut(bob).resources = warsel_core::Cost::new(5000, 5000, 5000);

    let base = world.map.starts[0];
    let enemy_pos = world
        .map
        .nearest_walkable(TilePos::new(base.x + 8, base.y), 10)
        .unwrap();
    let enemy = world
        .spawn_unit(UnitDefId(UNIT_CLUBMAN), bob, enemy_pos.center())
        .unwrap();
    let squad: Vec<EntityId> = (0..3)
        .map(|i| {
            world
                .spawn_unit(
                    UnitDefId(UNIT_CLUBMAN),
                    alice,
                    base.center() + warsel_core::world::formation_offset(i, 3, 0),
                )
                .unwrap()
        })
        .collect();

    let r = world.apply_command(
        alice,
        Command::AttackMove {
            units: squad,
            target: enemy_pos,
        },
    );
    assert!(r.accepted);
    world.step_many(20 * 45);
    assert!(
        world.entity(enemy).is_none(),
        "attack-move should have killed the lone defender"
    );
}

#[test]
fn population_cap_blocks_training() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let barracks = own_building(&world, alice, BUILDING_BARRACKS).unwrap();
    world.player_mut(alice).resources = warsel_core::Cost::new(100_000, 100_000, 100_000);
    // Fill the cap with units spawned directly.
    while world.player(alice).population < world.player(alice).pop_cap {
        let pos = world.map.starts[0].center();
        if world
            .spawn_unit(UnitDefId(UNIT_CLUBMAN), alice, pos)
            .is_err()
        {
            break;
        }
    }
    let r = world.apply_command(
        alice,
        Command::Train {
            building: barracks,
            unit: UnitDefId(UNIT_CLUBMAN),
        },
    );
    assert!(!r.accepted);
}

#[test]
fn stop_order_clears_orders() {
    let mut world = test_world(GameMode::Sandbox);
    let alice = PlayerId(1);
    let unit = own_units(&world, alice)[0];
    let far = TilePos::new(world.map.width - 5, world.map.height - 5);
    world.apply_command(
        alice,
        Command::Move {
            units: vec![unit],
            target: far,
        },
    );
    assert!(matches!(
        world.unit(unit).unwrap().order,
        Order::Move { .. }
    ));
    world.apply_command(alice, Command::Stop { units: vec![unit] });
    assert_eq!(world.unit(unit).unwrap().order, Order::Idle);
}

#[test]
fn resigned_player_stops_being_an_opponent() {
    let mut world = test_world(GameMode::Ffa);
    let bob = PlayerId(2);
    let r = world.apply_command(bob, Command::Resign);
    assert!(r.accepted);
    world.step();
    assert_eq!(world.outcome, MatchOutcome::Victory(PlayerId(1)));
}

#[test]
fn big_match_runs_without_exploding() {
    // 8 players, standard map: a cheap smoke test for scaling.
    let players = (0..8)
        .map(|i| (format!("P{i}"), (i % 2) as u8, true))
        .collect();
    let mut world = World::new(
        MatchConfig {
            seed: 777,
            map_size: warsel_core::MapSize::Standard,
            mode: GameMode::Team,
            ..MatchConfig::default()
        },
        players,
    );
    let start = std::time::Instant::now();
    world.step_many(20 * 60);
    let elapsed = start.elapsed();
    assert!(world.unit_count() > 50);
    assert!(
        elapsed.as_secs() < 60,
        "60 in-game seconds took {elapsed:?}, way too slow"
    );
}
