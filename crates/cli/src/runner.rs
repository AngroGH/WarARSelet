//! Match running, reporting and benchmarking.

use std::time::{Duration, Instant};

use serde_json::json;
use warsel_core::bot::SimpleBot;
use warsel_core::checksum;
use warsel_core::entity::Entity;
use warsel_core::epoch::Epoch;
use warsel_core::replay::ReplayPlayer;
use warsel_core::{GameEvent, GameMode, MatchConfig, MatchOutcome, PlayerId, Replay, World};

use crate::args::{Action, Options};

#[derive(Clone, Debug, Default)]
pub struct EventCounts {
    pub commands: u64,
    pub trained: u64,
    pub completed: u64,
    pub damage: u64,
    pub killed: u64,
    pub gathered: u64,
    pub epochs: u64,
    pub civ: u64,
    pub wonders: u64,
    pub eliminated: u64,
    pub meteors: u64,
    pub rejected: u64,
}

impl EventCounts {
    fn record(&mut self, event: GameEvent) {
        match event {
            GameEvent::UnitTrained { .. } => self.trained += 1,
            GameEvent::BuildingCompleted { .. } => self.completed += 1,
            GameEvent::Damage { .. } => self.damage += 1,
            GameEvent::EntityKilled { .. } => self.killed += 1,
            GameEvent::ResourceGathered { .. } => self.gathered += 1,
            GameEvent::EpochAdvanced { .. } => self.epochs += 1,
            GameEvent::CivChosen { .. } => self.civ += 1,
            GameEvent::WonderCompleted { .. } => self.wonders += 1,
            GameEvent::PlayerEliminated { .. } => self.eliminated += 1,
            GameEvent::MeteorStrike { .. } => self.meteors += 1,
            GameEvent::CommandRejected { .. } => self.rejected += 1,
            GameEvent::BuildingPlaced { .. } => {}
        }
    }
}

pub struct PlayerReport {
    pub name: String,
    pub epoch: Epoch,
    pub resources: i64,
    pub population: i32,
    pub pop_cap: i32,
    pub units: usize,
    pub buildings: usize,
    pub kills: u32,
    pub losses: u32,
    pub gathered: i64,
}

pub struct Report {
    pub outcome: MatchOutcome,
    pub tick: u64,
    pub checksum: u64,
    pub players: Vec<PlayerReport>,
    pub commands: usize,
    pub replay_verified: bool,
    pub counts: EventCounts,
    pub elapsed: Duration,
}

pub fn simulate(opts: &Options) -> Result<(), String> {
    let demo = opts.action == Action::Demo && !opts.json && !opts.quiet;
    let (mut world, mut bots, defs) = create_match(opts);
    let mut replay = Replay::new(
        world.config.clone(),
        defs.iter()
            .map(|(name, team, is_ai)| ReplayPlayer {
                name: name.clone(),
                team: *team,
                is_ai: *is_ai,
            })
            .collect(),
    );
    let mut counts = EventCounts::default();

    if demo {
        println!(
            "WarARSelet demo - seed {}, {} players, {}, {:?} ({}x{})",
            opts.seed,
            opts.players,
            opts.mode.name(),
            opts.map_size(),
            opts.map_size().dims(),
            opts.map_size().dims()
        );
        println!("Bots issue ordinary commands; the world stays authoritative.\n");
        println!("{}", status_line(&world));
    }

    let started = Instant::now();
    let total_ticks = opts.total_ticks();
    for _ in 0..total_ticks {
        let tick = world.tick().0;

        if tick % 10 == 0 {
            run_bots(&mut world, &mut bots, &mut replay, &mut counts);
        }

        // Snapshot after this tick's commands and before stepping, matching
        // `Replay::verify`'s "commands at tick T, then state T" contract.
        if tick % 200 == 0 {
            replay.checkpoint(&world);
        }

        world.step();
        for event in world.drain_events() {
            counts.record(event);
            if demo {
                print_notable(event);
            }
        }

        if demo && world.tick().0 % 100 == 0 {
            println!("{}", status_line(&world));
        }
        if world.is_over() {
            break;
        }
    }

    if replay.checkpoints.last().map(|(tick, _)| *tick) != Some(world.tick().0) {
        replay.checkpoint(&world);
    }

    let verification = replay.verify();
    let report = Report {
        outcome: world.outcome,
        tick: world.tick().0,
        checksum: checksum::hash_world(&world),
        players: world
            .players
            .iter()
            .filter(|player| !player.id.is_gaia())
            .map(|player| {
                let (units, buildings) = entity_counts(&world, player.id);
                PlayerReport {
                    name: player.name.clone(),
                    epoch: player.epoch,
                    resources: player.resources.total(),
                    population: player.population,
                    pop_cap: player.pop_cap,
                    units,
                    buildings,
                    kills: player.kills,
                    losses: player.losses,
                    gathered: player.gathered,
                }
            })
            .collect(),
        commands: replay.records.len(),
        replay_verified: verification.is_ok(),
        counts,
        elapsed: started.elapsed(),
    };

    if let Some(path) = &opts.replay {
        std::fs::write(path, replay.to_json())
            .map_err(|err| format!("cannot write replay to {path}: {err}"))?;
        eprintln!("replay saved to {path}");
    }

    print_report(&report, opts, verification.err());
    Ok(())
}

pub fn bench(opts: &Options) -> Result<(), String> {
    let (mut world, mut bots, _) = create_match(opts);
    let ticks = opts.ticks.unwrap_or_else(|| opts.total_ticks());

    let started = Instant::now();
    for _ in 0..ticks {
        if world.tick().0 % 10 == 0 {
            run_bench_bots(&mut world, &mut bots);
        }
        world.step();
        let _ = world.drain_events();
    }
    let elapsed = started.elapsed();
    let seconds = elapsed.as_secs_f64();
    let ticks_per_second = if seconds > 0.0 {
        ticks as f64 / seconds
    } else {
        0.0
    };

    println!(
        "bench: {} ticks, {} players, {} entities",
        ticks,
        opts.players,
        world.entities.len()
    );
    println!(
        "  {:.0} ticks/s, {:.3} ms/tick, {:.3}s total",
        ticks_per_second,
        seconds * 1000.0 / ticks as f64,
        seconds
    );
    println!("  checksum: 0x{:016x}", checksum::hash_world(&world));
    Ok(())
}

fn create_match(opts: &Options) -> (World, Vec<SimpleBot>, Vec<(String, u8, bool)>) {
    let defs: Vec<(String, u8, bool)> = (0..opts.players)
        .map(|i| {
            let team = if opts.mode == GameMode::Team {
                (i % 2) as u8 + 1
            } else {
                0
            };
            (format!("Bot {}", i + 1), team, true)
        })
        .collect();
    let config = MatchConfig {
        seed: opts.seed,
        map_size: opts.map_size(),
        mode: opts.mode,
        ..MatchConfig::default()
    };
    let world = World::new(config, defs.clone());
    let bots = (0..world.players.len()).map(|_| SimpleBot::new()).collect();
    (world, bots, defs)
}

fn run_bots(
    world: &mut World,
    bots: &mut [SimpleBot],
    replay: &mut Replay,
    counts: &mut EventCounts,
) {
    let tick = world.tick().0;
    for (index, bot) in bots.iter_mut().enumerate().skip(1) {
        if index >= world.players.len() {
            break;
        }
        let player = PlayerId(index as u16);
        if !world.player(player).is_active() {
            continue;
        }
        for command in bot.commands(world, player) {
            let result = world.apply_command(player, command.clone());
            counts.commands += 1;
            replay.record(tick, player, command, &result);
        }
    }
}

fn run_bench_bots(world: &mut World, bots: &mut [SimpleBot]) {
    for (index, bot) in bots.iter_mut().enumerate().skip(1) {
        if index >= world.players.len() {
            break;
        }
        let player = PlayerId(index as u16);
        if !world.player(player).is_active() {
            continue;
        }
        for command in bot.commands(world, player) {
            let _ = world.apply_command(player, command);
        }
    }
}

fn status_line(world: &World) -> String {
    let mut line = format!("[{:>4}s]", world.tick().seconds());
    let mut shown = 0;
    for player in world.players.iter().filter(|player| !player.id.is_gaia()) {
        let (units, buildings) = entity_counts(world, player.id);
        if shown > 0 {
            line.push_str(" | ");
        }
        line.push_str(&format!(
            "{} {:>9} {:>6}r {:>3}/{:<3}p {:>3}u {:>2}b",
            player.id,
            short_epoch(player.epoch),
            player.resources.total(),
            player.population,
            player.pop_cap,
            units,
            buildings
        ));
        shown += 1;
        if shown == 8 {
            line.push_str(" | ...");
            break;
        }
    }
    line
}

fn entity_counts(world: &World, player: PlayerId) -> (usize, usize) {
    let mut units = 0;
    let mut buildings = 0;
    for (_, entity) in world.entities.iter() {
        if entity.owner() != player {
            continue;
        }
        if matches!(entity, Entity::Unit(_)) {
            units += 1;
        } else {
            buildings += 1;
        }
    }
    (units, buildings)
}

fn short_epoch(epoch: Epoch) -> &'static str {
    match epoch {
        Epoch::Stone => "Stone",
        Epoch::Bronze => "Bronze",
        Epoch::Iron => "Iron",
        Epoch::EarlyMedieval => "EarlyMed",
        Epoch::LateMedieval => "LateMed",
        Epoch::Industry1 => "Ind1",
        Epoch::Industry2 => "Ind2",
    }
}

fn print_notable(event: GameEvent) {
    match event {
        GameEvent::EpochAdvanced { owner, epoch } => {
            println!("  + {owner} advanced to epoch {epoch}");
        }
        GameEvent::PlayerEliminated { owner } => {
            println!("  ! {owner} eliminated");
        }
        GameEvent::WonderCompleted { owner } => {
            println!("  * {owner} completed a wonder");
        }
        GameEvent::MeteorStrike { pos, radius_milli } => {
            println!(
                "  * meteor at ({}, {}) milli-tiles, radius {radius_milli}",
                pos.x.to_milli(),
                pos.y.to_milli()
            );
        }
        _ => {}
    }
}

fn print_report(report: &Report, opts: &Options, verify_error: Option<(u64, u64, u64)>) {
    if opts.json {
        let summary = json!({
            "seed": opts.seed,
            "mode": opts.mode.name(),
            "map_size": format!("{:?}", opts.map_size()),
            "players": report.players.iter().map(|player| json!({
                "name": player.name,
                "epoch": player.epoch.name(),
                "resources": player.resources,
                "population": player.population,
                "pop_cap": player.pop_cap,
                "units": player.units,
                "buildings": player.buildings,
                "kills": player.kills,
                "losses": player.losses,
                "gathered": player.gathered,
            })).collect::<Vec<_>>(),
            "outcome": outcome_json(&report.outcome),
            "tick": report.tick,
            "seconds": report.tick as f64 / warsel_core::time::TICKS_PER_SECOND as f64,
            "checksum": format!("0x{:016x}", report.checksum),
            "commands": report.commands,
            "replay_verified": report.replay_verified,
            "events": {
                "trained": report.counts.trained,
                "completed": report.counts.completed,
                "damage": report.counts.damage,
                "killed": report.counts.killed,
                "gathered": report.counts.gathered,
                "epochs": report.counts.epochs,
                "civ": report.counts.civ,
                "wonders": report.counts.wonders,
                "eliminated": report.counts.eliminated,
                "meteors": report.counts.meteors,
                "rejected": report.counts.rejected,
            },
            "elapsed_ms": report.elapsed.as_millis(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".into())
        );
        return;
    }

    println!();
    println!("===== result =====");
    println!("outcome: {}", outcome_label(&report.outcome));
    println!(
        "tick: {} ({:.1}s)  checksum: 0x{:016x}",
        report.tick,
        report.tick as f64 / warsel_core::time::TICKS_PER_SECOND as f64,
        report.checksum
    );
    println!(
        "commands: {}  replay: {}",
        report.commands,
        if report.replay_verified {
            "verified"
        } else {
            "DIVERGED"
        }
    );
    if let Some((tick, expected, actual)) = verify_error {
        println!(
            "  replay mismatch at tick {tick}: expected 0x{expected:016x}, got 0x{actual:016x}"
        );
    }
    println!();
    for player in &report.players {
        println!(
            "{:<10} {:<9} {:>7}r {:>3}/{:<3}p {:>4}u {:>3}b  K/D {}/{}  gathered {}",
            player.name,
            short_epoch(player.epoch),
            player.resources,
            player.population,
            player.pop_cap,
            player.units,
            player.buildings,
            player.kills,
            player.losses,
            player.gathered
        );
    }
    println!();
    println!(
        "events: trained={} completed={} killed={} damage={} gathered={} epochs={} civ={} wonders={} eliminated={} meteors={} rejected={}",
        report.counts.trained,
        report.counts.completed,
        report.counts.killed,
        report.counts.damage,
        report.counts.gathered,
        report.counts.epochs,
        report.counts.civ,
        report.counts.wonders,
        report.counts.eliminated,
        report.counts.meteors,
        report.counts.rejected
    );
    println!("elapsed: {} ms", report.elapsed.as_millis());
}

fn outcome_label(outcome: &MatchOutcome) -> String {
    match outcome {
        MatchOutcome::Ongoing => "ongoing".into(),
        MatchOutcome::Finished => "finished (draw)".into(),
        MatchOutcome::Victory(player) => format!("victory: {player}"),
    }
}

fn outcome_json(outcome: &MatchOutcome) -> serde_json::Value {
    match outcome {
        MatchOutcome::Ongoing => json!({ "type": "ongoing" }),
        MatchOutcome::Finished => json!({ "type": "finished" }),
        MatchOutcome::Victory(player) => json!({ "type": "victory", "player": player.0 }),
    }
}
