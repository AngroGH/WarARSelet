//! Browser interface for WarARSelet.
//!
//! A tiny dependency-free HTTP server (std::net only) hosts a canvas client and
//! a small JSON API:
//!
//! * `GET  /api/state`    - full visible snapshot for the human player
//! * `POST /api/command`  - `{ "command": <Command> }`
//! * `POST /api/restart`  - `{ "seed": <u64> }`
//!
//! The authoritative world still lives in `warsel-core`; the browser only
//! renders what the fog of war allows it to see.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use warsel_core::bot::SimpleBot;
use warsel_core::entity::Entity;
use warsel_core::map::{MapSize, Terrain};
use warsel_core::resource::Cost;
use warsel_core::time::TICKS_PER_SECOND;
use warsel_core::{Command, GameEvent, GameMode, MatchConfig, PlayerId, TilePos, World};

const INDEX_HTML: &[u8] = include_bytes!("../web/index.html");
const APP_JS: &[u8] = include_bytes!("../web/app.js");
const STYLE_CSS: &[u8] = include_bytes!("../web/style.css");

struct Config {
    addr: String,
    players: u16,
    seed: u64,
    mode: GameMode,
    size: MapSize,
}

#[derive(Deserialize)]
struct CommandRequest {
    command: Command,
}

#[derive(Deserialize)]
struct RestartRequest {
    seed: Option<u64>,
}

struct Game {
    world: World,
    bots: Vec<SimpleBot>,
    human: PlayerId,
    players: u16,
    seed: u64,
    mode: GameMode,
    size: MapSize,
    log: VecDeque<String>,
}

impl Game {
    fn new(players: u16, seed: u64, mode: GameMode, size: MapSize) -> Self {
        let defs: Vec<(String, u8, bool)> = (0..players)
            .map(|i| {
                let name = if i == 0 {
                    "You".to_string()
                } else {
                    format!("Bot {}", i)
                };
                let team = if mode == GameMode::Team {
                    (i % 2) as u8 + 1
                } else {
                    0
                };
                (name, team, i != 0)
            })
            .collect();
        let config = MatchConfig {
            seed,
            map_size: size,
            mode,
            ..MatchConfig::default()
        };
        let world = World::new(config, defs);
        let bots = (0..world.players.len()).map(|_| SimpleBot::new()).collect();
        let mut log = VecDeque::new();
        log.push_back(format!(
            "Match started: seed {seed}, {players} players, {}, {:?}",
            mode.name(),
            size
        ));
        Game {
            world,
            bots,
            human: PlayerId(1),
            players,
            seed,
            mode,
            size,
            log,
        }
    }

    fn restart(&mut self, seed: u64) {
        *self = Game::new(self.players, seed, self.mode, self.size);
    }

    fn step(&mut self) {
        let tick = self.world.tick().0;
        if tick.is_multiple_of(10) {
            // Index 0 is Gaia, index 1 is the human. Only bots above index 1.
            for (index, bot) in self.bots.iter_mut().enumerate().skip(2) {
                if index >= self.world.players.len() {
                    break;
                }
                let player = PlayerId(index as u16);
                if !self.world.player(player).is_active() {
                    continue;
                }
                for command in bot.commands(&self.world, player) {
                    let _ = self.world.apply_command(player, command);
                }
            }
        }

        self.world.step();

        let now = self.world.tick().0;
        for event in self.world.drain_events() {
            if let Some(line) = describe_event(&self.world, event) {
                self.log
                    .push_back(format!("[{:>6.1}s] {}", now as f64 / 20.0, line));
                while self.log.len() > 300 {
                    self.log.pop_front();
                }
            }
        }
    }

    fn state(&self) -> Value {
        let world = &self.world;
        let human = self.human;
        let player = world.player(human);
        let fog = &world.fog[human.index()];

        let mut terrain = Vec::with_capacity(world.map.height as usize);
        let mut visible = Vec::with_capacity(world.map.height as usize);
        let mut explored = Vec::with_capacity(world.map.height as usize);
        for y in 0..world.map.height {
            let mut t = String::with_capacity(world.map.width as usize);
            let mut v = String::with_capacity(world.map.width as usize);
            let mut e = String::with_capacity(world.map.width as usize);
            for x in 0..world.map.width {
                let pos = TilePos::new(x, y);
                t.push(match world.map.terrain(x, y) {
                    Terrain::Grass => 'g',
                    Terrain::Water => 'w',
                    Terrain::Rock => 'r',
                });
                v.push(if fog.is_visible_tile(pos) { '1' } else { '0' });
                e.push(if fog.is_explored_tile(pos) { '1' } else { '0' });
            }
            terrain.push(t);
            visible.push(v);
            explored.push(e);
        }

        let nodes: Vec<Value> = world
            .map
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.amount > 0)
            .map(|(index, node)| {
                json!({
                    "i": index,
                    "x": node.pos.x,
                    "y": node.pos.y,
                    "kind": node.kind.name(),
                    "amount": node.amount,
                })
            })
            .collect();

        let mut entities = Vec::new();
        for (id, entity) in world.entities.iter() {
            if entity.owner() != human && !world.is_visible_to(human, entity.pos()) {
                continue;
            }
            match entity {
                Entity::Unit(unit) => {
                    let def = world.catalog.unit(unit.def);
                    entities.push(json!({
                        "kind": "unit",
                        "id": id,
                        "owner": unit.owner.0,
                        "def": unit.def.0,
                        "name": def.name,
                        "role": format!("{:?}", def.role),
                        "x": unit.pos.x.to_f32(),
                        "y": unit.pos.y.to_f32(),
                        "hp": unit.hp,
                        "max_hp": def.max_hp,
                        "state": format!("{:?}", unit.state),
                        "cargo": unit.cargo.amount,
                        "cargo_kind": unit.cargo.kind.map(|kind| kind.name()),
                    }));
                }
                Entity::Building(building) => {
                    let def = world.catalog.building(building.def);
                    entities.push(json!({
                        "kind": "building",
                        "id": id,
                        "owner": building.owner.0,
                        "def": building.def.0,
                        "name": def.name,
                        "role": format!("{:?}", def.role),
                        "x": building.pos.x.to_f32(),
                        "y": building.pos.y.to_f32(),
                        "origin_x": building.origin.x,
                        "origin_y": building.origin.y,
                        "footprint": def.footprint,
                        "hp": building.hp,
                        "max_hp": building.max_hp,
                        "finished": building.finished,
                        "queue": building.queue.iter().map(|item| json!({
                            "unit": item.unit.0,
                            "name": world.catalog.unit(item.unit).name,
                            "ticks_left": item.ticks_left,
                        })).collect::<Vec<_>>(),
                        "rally": building.rally.map(|rally| json!({
                            "x": rally.x.to_f32(),
                            "y": rally.y.to_f32(),
                        })),
                    }));
                }
            }
        }

        let players: Vec<Value> = world
            .players
            .iter()
            .filter(|player| !player.id.is_gaia())
            .map(|player| {
                json!({
                    "id": player.id.0,
                    "name": player.name,
                    "color": player.color,
                    "resources": cost_json(player.resources),
                    "population": player.population,
                    "pop_cap": player.pop_cap,
                    "epoch": player.epoch.index(),
                    "epoch_name": player.epoch.name(),
                    "kills": player.kills,
                    "losses": player.losses,
                    "status": format!("{:?}", player.status),
                    "is_human": player.id == human,
                })
            })
            .collect();

        json!({
            "tick": world.tick().0,
            "seconds": world.tick().seconds(),
            "outcome": world.outcome,
            "human": human.0,
            "players": players,
            "player": {
                "id": player.id.0,
                "name": player.name,
                "resources": cost_json(player.resources),
                "population": player.population,
                "pop_cap": player.pop_cap,
                "epoch": player.epoch.index(),
                "epoch_name": player.epoch.name(),
                "kills": player.kills,
                "losses": player.losses,
                "status": format!("{:?}", player.status),
            },
            "map": {
                "width": world.map.width,
                "height": world.map.height,
                "terrain": terrain,
                "visible": visible,
                "explored": explored,
                "nodes": nodes,
            },
            "entities": entities,
            "catalog": catalog_json(world),
            "log": self.log.iter().cloned().collect::<Vec<_>>(),
        })
    }
}

fn describe_event(world: &World, event: GameEvent) -> Option<String> {
    match event {
        GameEvent::UnitTrained { owner, entity } => {
            let name = world
                .unit(entity)
                .map(|unit| world.catalog.unit(unit.def).name)
                .unwrap_or("unit");
            Some(format!("{owner} trained {name}"))
        }
        GameEvent::BuildingCompleted { owner, entity } => {
            let name = world
                .building(entity)
                .map(|building| world.catalog.building(building.def).name)
                .unwrap_or("building");
            Some(format!("{owner} completed {name}"))
        }
        GameEvent::BuildingPlaced { owner, entity } => {
            let name = world
                .building(entity)
                .map(|building| world.catalog.building(building.def).name)
                .unwrap_or("building");
            Some(format!("{owner} placed {name}"))
        }
        GameEvent::EntityKilled { entity, killer } => Some(format!("{killer} killed {entity}")),
        GameEvent::EpochAdvanced { owner, epoch } => Some(format!("{owner} reached epoch {epoch}")),
        GameEvent::CivChosen { owner, slot, value } => {
            Some(format!("{owner} chose civilization slot {slot} = {value}"))
        }
        GameEvent::WonderCompleted { owner } => Some(format!("{owner} completed a wonder")),
        GameEvent::PlayerEliminated { owner } => Some(format!("{owner} was eliminated")),
        GameEvent::MeteorStrike { pos, radius_milli } => Some(format!(
            "Meteor strike at ({:.1}, {:.1}), radius {:.1}",
            pos.x.to_f32(),
            pos.y.to_f32(),
            radius_milli as f64 / 1000.0
        )),
        GameEvent::CommandRejected { owner, reason } => {
            Some(format!("{owner} command rejected (code {reason})"))
        }
        GameEvent::Damage { .. } | GameEvent::ResourceGathered { .. } => None,
    }
}

fn cost_json(cost: Cost) -> Value {
    json!({
        "food": cost.food,
        "wood": cost.wood,
        "stone": cost.stone,
        "copper": cost.copper,
        "iron": cost.iron,
        "gold": cost.gold,
    })
}

fn catalog_json(world: &World) -> Value {
    let units: Vec<Value> = world
        .catalog
        .units
        .iter()
        .map(|unit| {
            json!({
                "id": unit.id.0,
                "key": unit.key,
                "name": unit.name,
                "epoch": unit.epoch.index(),
                "role": format!("{:?}", unit.role),
                "cost": cost_json(unit.cost),
                "pop": unit.pop,
                "build_ticks": unit.build_ticks,
                "attack": unit.attack,
                "max_hp": unit.max_hp,
                "armor": format!("{:?}", unit.armor),
                "range": unit.attack_range,
                "speed": unit.speed_milli,
            })
        })
        .collect();
    let buildings: Vec<Value> = world
        .catalog
        .buildings
        .iter()
        .map(|building| {
            json!({
                "id": building.id.0,
                "key": building.key,
                "name": building.name,
                "epoch": building.epoch.index(),
                "role": format!("{:?}", building.role),
                "cost": cost_json(building.cost),
                "build_ticks": building.build_ticks,
                "footprint": building.footprint,
                "pop_provided": building.pop_provided,
                "required_epoch": building.required_epoch.index(),
                "trains": building.trains.iter().map(|unit| unit.0).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "units": units, "buildings": buildings })
}

fn main() {
    let config = match parse_args() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("warsel-web: {error}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let listener = match TcpListener::bind(&config.addr) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("warsel-web: cannot bind {}: {error}", config.addr);
            std::process::exit(1);
        }
    };

    println!(
        "warsel-web: http://{}/  (seed {}, {} players, {}, {:?})",
        config.addr,
        config.seed,
        config.players,
        config.mode.name(),
        config.size
    );
    println!("warsel-web: you control Player 1; other slots are bots");

    let game = Arc::new(Mutex::new(Game::new(
        config.players,
        config.seed,
        config.mode,
        config.size,
    )));

    {
        let game = Arc::clone(&game);
        thread::spawn(move || loop {
            {
                let mut game = game.lock().unwrap();
                game.step();
            }
            thread::sleep(Duration::from_millis(1000 / TICKS_PER_SECOND as u64));
        });
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let game = Arc::clone(&game);
                thread::spawn(move || handle_connection(stream, game));
            }
            Err(error) => eprintln!("warsel-web: accept error: {error}"),
        }
    }
}

fn handle_connection(mut stream: TcpStream, game: Arc<Mutex<Game>>) {
    let (method, target, body) = match read_request(&mut stream) {
        Ok(request) => request,
        Err(_) => return,
    };
    let path = target.split('?').next().unwrap_or("/");

    match (method.as_str(), path) {
        ("GET", "/") => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            INDEX_HTML,
        ),
        ("GET", "/app.js") => respond(
            &mut stream,
            "200 OK",
            "text/javascript; charset=utf-8",
            APP_JS,
        ),
        ("GET", "/style.css") => {
            respond(&mut stream, "200 OK", "text/css; charset=utf-8", STYLE_CSS)
        }
        ("GET", "/api/state") => {
            let state = {
                let game = game.lock().unwrap();
                game.state()
            };
            respond_json(&mut stream, "200 OK", &state);
        }
        ("GET", "/api/health") => {
            respond_json(&mut stream, "200 OK", &json!({ "ok": true }));
        }
        ("POST", "/api/command") => {
            let request: CommandRequest = match serde_json::from_slice(&body) {
                Ok(request) => request,
                Err(error) => {
                    respond_json(
                        &mut stream,
                        "400 Bad Request",
                        &json!({ "error": format!("invalid command json: {error}") }),
                    );
                    return;
                }
            };
            let (accepted, reason, epoch) = {
                let mut game = game.lock().unwrap();
                let human = game.human;
                let result = game.world.apply_command(human, request.command);
                (result.accepted, result.reason, result.epoch)
            };
            respond_json(
                &mut stream,
                "200 OK",
                &json!({
                    "accepted": accepted,
                    "reason": reason,
                    "epoch": epoch,
                }),
            );
        }
        ("POST", "/api/restart") => {
            let request: RestartRequest = if body.is_empty() {
                RestartRequest { seed: None }
            } else {
                match serde_json::from_slice(&body) {
                    Ok(request) => request,
                    Err(error) => {
                        respond_json(
                            &mut stream,
                            "400 Bad Request",
                            &json!({ "error": format!("invalid restart json: {error}") }),
                        );
                        return;
                    }
                }
            };
            let state = {
                let mut game = game.lock().unwrap();
                let seed = request
                    .seed
                    .unwrap_or_else(|| game.seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
                game.restart(seed);
                game.state()
            };
            respond_json(&mut stream, "200 OK", &state);
        }
        _ => respond(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"not found",
        ),
    }
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<(String, String, Vec<u8>)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    if request_line.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "empty request",
        ));
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    if content_length > 1_048_576 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request body too large",
        ));
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;
    Ok((method, target, body))
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn respond_json(stream: &mut TcpStream, status: &str, value: &Value) {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    respond(stream, status, "application/json; charset=utf-8", &body);
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config {
        addr: "127.0.0.1:8080".into(),
        players: 2,
        seed: 12345,
        mode: GameMode::Ffa,
        size: MapSize::Duel,
    };
    let mut size_set = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            "--addr" => config.addr = next_value(&mut args, "--addr")?,
            "--players" => {
                let players: u16 = next_value(&mut args, "--players")?
                    .parse()
                    .map_err(|_| "--players expects an integer")?;
                if players == 0 || players > 62 {
                    return Err("--players must be in 1..=62".into());
                }
                config.players = players;
            }
            "--seed" => {
                config.seed = next_value(&mut args, "--seed")?
                    .parse()
                    .map_err(|_| "--seed expects a non-negative integer")?;
            }
            "--mode" => {
                let mode = next_value(&mut args, "--mode")?;
                config.mode = match mode.as_str() {
                    "ffa" => GameMode::Ffa,
                    "team" => GameMode::Team,
                    "armageddon" => GameMode::Armageddon,
                    "survival" => GameMode::Survival,
                    "sandbox" => GameMode::Sandbox,
                    _ => {
                        return Err(format!(
                            "unknown mode '{mode}' (expected ffa, team, armageddon, survival or sandbox)"
                        ))
                    }
                };
            }
            "--size" => {
                let size = next_value(&mut args, "--size")?;
                config.size = match size.as_str() {
                    "duel" => MapSize::Duel,
                    "small" => MapSize::Small,
                    "standard" => MapSize::Standard,
                    "large" => MapSize::Large,
                    _ => {
                        return Err(format!(
                            "unknown size '{size}' (expected duel, small, standard or large)"
                        ))
                    }
                };
                size_set = true;
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    if !size_set {
        config.size = size_for_players(config.players);
    }
    Ok(config)
}

fn size_for_players(players: u16) -> MapSize {
    if players <= 2 {
        MapSize::Duel
    } else if players <= 4 {
        MapSize::Small
    } else if players <= 8 {
        MapSize::Standard
    } else {
        MapSize::Large
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> &'static str {
    "\
warsel-web - browser interface for WarARSelet

USAGE:
    warsel-web [OPTIONS]

OPTIONS:
    --addr <ADDR>     HTTP listen address      (default: 127.0.0.1:8080)
    --players <N>     Player slots, 1..=62     (default: 2)
    --seed <N>        Match seed               (default: 12345)
    --mode <MODE>     ffa | team | armageddon | survival | sandbox
    --size <SIZE>     duel | small | standard | large
    -h, --help        Show this help

Open the printed URL in a browser. You control Player 1.
"
}
