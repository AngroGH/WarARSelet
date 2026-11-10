//! Authoritative development server.
//!
//! The server owns a [`World`] and advances it at 20 Hz. Clients connect over
//! TCP and exchange newline-delimited JSON:
//!
//! ```text
//! server -> {"type":"welcome","player":1}
//! server -> {"type":"state","tick":20,"players":[...]}
//! client -> {"type":"command","command":{"Chat":{"text":"hi"}}}
//! server -> {"type":"result","player":1,"accepted":true,"reason":null}
//! server -> {"type":"events","tick":21,"events":[...]}
//! ```
//!
//! Each connection is assigned a player slot; extra connections spectate.
//! Until a human takes a slot, a deterministic [`SimpleBot`] plays it.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;
use warsel_core::bot::SimpleBot;
use warsel_core::time::TICKS_PER_SECOND;
use warsel_core::{Command, GameMode, MapSize, MatchConfig, PlayerId, World};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Command { command: Command },
    Chat { text: String },
    Ping,
}

struct Config {
    addr: String,
    players: u16,
    seed: u64,
    mode: GameMode,
    size: MapSize,
    bots: bool,
}

fn main() {
    let config = match parse_args() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("warsel-server: {error}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let listener = match TcpListener::bind(&config.addr) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("warsel-server: cannot bind {}: {error}", config.addr);
            std::process::exit(1);
        }
    };

    println!(
        "warsel-server listening on {} (seed {}, {} players, {}, {:?})",
        config.addr,
        config.seed,
        config.players,
        config.mode.name(),
        config.size
    );
    println!("Send newline-delimited JSON commands, e.g. {{\"type\":\"ping\"}}");

    let clients: Arc<Mutex<Vec<SyncSender<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let human_mask = Arc::new(AtomicU64::new(0));
    let next_player = Arc::new(AtomicU64::new(1));
    let (command_tx, command_rx) = mpsc::channel::<(PlayerId, Command)>();

    {
        let clients = Arc::clone(&clients);
        let human_mask = Arc::clone(&human_mask);
        let next_player = Arc::clone(&next_player);
        let command_tx = command_tx.clone();
        let max_players = config.players;
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let slot = next_player.fetch_add(1, Ordering::SeqCst);
                        let player = if slot <= max_players as u64 {
                            let player = PlayerId(slot as u16);
                            human_mask.fetch_or(1u64 << (player.0 - 1), Ordering::SeqCst);
                            Some(player)
                        } else {
                            None
                        };
                        spawn_client(
                            stream,
                            player,
                            command_tx.clone(),
                            Arc::clone(&clients),
                            Arc::clone(&human_mask),
                        );
                    }
                    Err(error) => eprintln!("warsel-server: accept error: {error}"),
                }
            }
        });
    }

    run_game(config, command_rx, clients, human_mask);
}

fn run_game(
    config: Config,
    commands: Receiver<(PlayerId, Command)>,
    clients: Arc<Mutex<Vec<SyncSender<String>>>>,
    human_mask: Arc<AtomicU64>,
) {
    let defs: Vec<(String, u8, bool)> = (0..config.players)
        .map(|i| {
            let team = if config.mode == GameMode::Team {
                (i % 2) as u8 + 1
            } else {
                0
            };
            (format!("Player {}", i + 1), team, true)
        })
        .collect();
    let match_config = MatchConfig {
        seed: config.seed,
        map_size: config.size,
        mode: config.mode,
        ..MatchConfig::default()
    };
    let mut world = World::new(match_config, defs);
    let mut bots: Vec<SimpleBot> = (0..world.players.len()).map(|_| SimpleBot::new()).collect();
    let tick_period = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    let mut next_tick = Instant::now();
    let mut over_announced = false;

    loop {
        // Commands from humans.
        while let Ok((player, command)) = commands.try_recv() {
            let result = world.apply_command(player, command);
            broadcast(
                &clients,
                &json!({
                    "type": "result",
                    "player": player.0,
                    "accepted": result.accepted,
                    "reason": result.reason,
                })
                .to_string(),
            );
        }

        // Bots fill every slot that is not currently controlled by a human.
        let tick = world.tick().0;
        if config.bots && tick.is_multiple_of(10) {
            for (index, bot) in bots.iter_mut().enumerate().skip(1) {
                if index >= world.players.len() {
                    break;
                }
                let player = PlayerId(index as u16);
                if !world.player(player).is_active() {
                    continue;
                }
                let human = human_mask.load(Ordering::SeqCst) & (1u64 << (index - 1)) != 0;
                if human {
                    continue;
                }
                for command in bot.commands(&world, player) {
                    let _ = world.apply_command(player, command);
                }
            }
        }

        world.step();

        let events = world.drain_events();
        if !events.is_empty() {
            let dropped = events.len().saturating_sub(512);
            let visible = &events[..events.len().min(512)];
            broadcast(
                &clients,
                &json!({
                    "type": "events",
                    "tick": world.tick().0,
                    "events": visible,
                    "dropped": dropped,
                })
                .to_string(),
            );
        }

        if world.tick().is_multiple_of(TICKS_PER_SECOND) {
            let players: Vec<_> = world
                .players
                .iter()
                .filter(|player| !player.id.is_gaia())
                .map(|player| {
                    let human =
                        human_mask.load(Ordering::SeqCst) & (1u64 << (player.id.0 - 1)) != 0;
                    json!({
                        "id": player.id.0,
                        "name": player.name,
                        "epoch": player.epoch.name(),
                        "resources": {
                            "food": player.resources.food,
                            "wood": player.resources.wood,
                            "stone": player.resources.stone,
                            "copper": player.resources.copper,
                            "iron": player.resources.iron,
                            "gold": player.resources.gold,
                        },
                        "population": player.population,
                        "pop_cap": player.pop_cap,
                        "kills": player.kills,
                        "losses": player.losses,
                        "status": player.status,
                        "human": human,
                    })
                })
                .collect();
            broadcast(
                &clients,
                &json!({
                    "type": "state",
                    "tick": world.tick().0,
                    "outcome": world.outcome,
                    "players": players,
                })
                .to_string(),
            );
        }

        if world.is_over() && !over_announced {
            over_announced = true;
            broadcast(
                &clients,
                &json!({
                    "type": "over",
                    "tick": world.tick().0,
                    "outcome": world.outcome,
                })
                .to_string(),
            );
            println!(
                "warsel-server: match over at tick {}: {:?}",
                world.tick().0,
                world.outcome
            );
        }

        // Fixed-rate pacing; if a tick overruns, skip the missed sleep.
        next_tick += tick_period;
        let now = Instant::now();
        if next_tick > now {
            thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }
}

fn spawn_client(
    stream: TcpStream,
    player: Option<PlayerId>,
    commands: mpsc::Sender<(PlayerId, Command)>,
    clients: Arc<Mutex<Vec<SyncSender<String>>>>,
    human_mask: Arc<AtomicU64>,
) {
    let _ = stream.set_nodelay(true);
    let writer_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("warsel-server: cannot clone client socket: {error}");
            return;
        }
    };
    let (tx, rx) = mpsc::sync_channel::<String>(256);

    thread::spawn(move || {
        let mut writer = writer_stream;
        while let Ok(message) = rx.recv() {
            if writeln!(writer, "{message}").is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let welcome = json!({
        "type": "welcome",
        "protocol": 1,
        "player": player.map(|player| player.0),
        "message": if player.is_some() {
            "send {\"type\":\"command\",\"command\":{...}} lines"
        } else {
            "match is full: connected as spectator"
        },
    });
    let _ = tx.try_send(welcome.to_string());
    clients.lock().unwrap().push(tx.clone());

    if let Some(player) = player {
        println!("warsel-server: player {} connected", player.0);
    } else {
        println!("warsel-server: spectator connected");
    }

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<ClientMessage>(line) {
            Ok(ClientMessage::Command { command }) => {
                if let Some(player) = player {
                    if commands.send((player, command)).is_err() {
                        break;
                    }
                } else {
                    let _ = tx.try_send(
                        json!({
                            "type": "error",
                            "message": "spectators cannot issue commands",
                        })
                        .to_string(),
                    );
                }
            }
            Ok(ClientMessage::Chat { text }) => {
                if let Some(player) = player {
                    if commands.send((player, Command::Chat { text })).is_err() {
                        break;
                    }
                }
            }
            Ok(ClientMessage::Ping) => {
                let _ = tx.try_send(json!({ "type": "pong" }).to_string());
            }
            Err(error) => {
                let _ = tx.try_send(
                    json!({
                        "type": "error",
                        "message": format!("invalid json: {error}"),
                    })
                    .to_string(),
                );
            }
        }
    }

    if let Some(player) = player {
        human_mask.fetch_and(!(1u64 << (player.0 - 1)), Ordering::SeqCst);
        println!("warsel-server: player {} disconnected", player.0);
    } else {
        println!("warsel-server: spectator disconnected");
    }
}

fn broadcast(clients: &Arc<Mutex<Vec<SyncSender<String>>>>, message: &str) {
    let mut clients = clients.lock().unwrap();
    clients.retain(|client| match client.try_send(message.to_string()) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => true,
        Err(TrySendError::Disconnected(_)) => false,
    });
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config {
        addr: "127.0.0.1:7777".into(),
        players: 2,
        seed: 12345,
        mode: GameMode::Ffa,
        size: MapSize::Small,
        bots: true,
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
            "--no-bots" => config.bots = false,
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
warsel-server - authoritative WarARSelet dev server

USAGE:
    warsel-server [OPTIONS]

OPTIONS:
    --addr <ADDR>       Listen address          (default: 127.0.0.1:7777)
    --players <N>       Player slots, 1..=62    (default: 2)
    --seed <N>          Match seed              (default: 12345)
    --mode <MODE>       ffa | team | armageddon | survival | sandbox
    --size <SIZE>       duel | small | standard | large
    --no-bots           Wait for human players instead of running bots
    -h, --help          Show this help

PROTOCOL (newline-delimited JSON over TCP):
    {\"type\":\"command\",\"command\":{\"Chat\":{\"text\":\"hi\"}}}
    {\"type\":\"chat\",\"text\":\"hi\"}
    {\"type\":\"ping\"}

The server broadcasts:
    welcome, state (1/s), events (per tick), result, over
"
}
