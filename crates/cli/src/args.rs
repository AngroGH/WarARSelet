//! Command-line parsing for the headless tools.

use warsel_core::{GameMode, MapSize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Demo,
    Simulate,
    Map,
    Bench,
    Help,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub action: Action,
    pub seed: u64,
    pub players: u16,
    pub seconds: u32,
    /// Benchmark only: explicit tick count (overrides `--seconds`).
    pub ticks: Option<u32>,
    pub mode: GameMode,
    pub size: Option<MapSize>,
    pub json: bool,
    pub quiet: bool,
    pub replay: Option<String>,
}

impl Options {
    pub fn map_size(&self) -> MapSize {
        self.size.unwrap_or_else(|| size_for_players(self.players))
    }

    pub fn total_ticks(&self) -> u32 {
        self.seconds
            .saturating_mul(warsel_core::time::TICKS_PER_SECOND)
    }
}

pub fn size_for_players(players: u16) -> MapSize {
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

pub fn parse() -> Result<Options, String> {
    let mut opts = Options {
        action: Action::Demo,
        seed: 12345,
        players: 2,
        seconds: 180,
        ticks: None,
        mode: GameMode::Ffa,
        size: None,
        json: false,
        quiet: false,
        replay: None,
    };

    let mut action_set = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                opts.action = Action::Help;
                return Ok(opts);
            }
            "demo" | "simulate" | "map" | "bench" => {
                if action_set {
                    return Err("only one command can be given".into());
                }
                action_set = true;
                opts.action = match arg.as_str() {
                    "demo" => Action::Demo,
                    "simulate" => Action::Simulate,
                    "map" => Action::Map,
                    _ => Action::Bench,
                };
            }
            "--seed" => {
                opts.seed = next_value(&mut args, "--seed")?
                    .parse()
                    .map_err(|_| "--seed expects a non-negative integer")?;
            }
            "--players" => {
                let players: u16 = next_value(&mut args, "--players")?
                    .parse()
                    .map_err(|_| "--players expects an integer")?;
                if players == 0 || players > 62 {
                    return Err("--players must be in 1..=62".into());
                }
                opts.players = players;
            }
            "--seconds" => {
                let seconds: u32 = next_value(&mut args, "--seconds")?
                    .parse()
                    .map_err(|_| "--seconds expects an integer")?;
                if seconds == 0 {
                    return Err("--seconds must be greater than zero".into());
                }
                opts.seconds = seconds;
            }
            "--ticks" => {
                let ticks: u32 = next_value(&mut args, "--ticks")?
                    .parse()
                    .map_err(|_| "--ticks expects an integer")?;
                if ticks == 0 {
                    return Err("--ticks must be greater than zero".into());
                }
                opts.ticks = Some(ticks);
            }
            "--mode" => {
                let mode = next_value(&mut args, "--mode")?;
                opts.mode = match mode.as_str() {
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
                opts.size = Some(match size.as_str() {
                    "duel" => MapSize::Duel,
                    "small" => MapSize::Small,
                    "standard" => MapSize::Standard,
                    "large" => MapSize::Large,
                    _ => {
                        return Err(format!(
                            "unknown size '{size}' (expected duel, small, standard or large)"
                        ))
                    }
                });
            }
            "--json" => opts.json = true,
            "--quiet" => opts.quiet = true,
            "--replay" => opts.replay = Some(next_value(&mut args, "--replay")?),
            other if other.starts_with('-') => {
                return Err(format!("unknown option '{other}'"));
            }
            other => return Err(format!("unknown command '{other}'")),
        }
    }

    Ok(opts)
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

pub fn usage() -> &'static str {
    "\
warsel — headless WarARSelet tools

USAGE:
    warsel [COMMAND] [OPTIONS]

COMMANDS:
    demo        Run a bot match with a live status line (default)
    simulate    Run a bot match and print a summary
    map         Print an ASCII preview of a generated map
    bench       Measure simulation throughput

OPTIONS:
    --seed <N>        Map/game seed            (default: 12345)
    --players <N>     Player count, 1..=62     (default: 2)
    --seconds <N>     Match length in seconds  (default: 180)
    --ticks <N>       Bench tick count         (overrides --seconds)
    --mode <MODE>     ffa | team | armageddon | survival | sandbox
    --size <SIZE>     duel | small | standard | large
    --replay <PATH>   Write the command replay as JSON
    --json            Print the final report as JSON
    --quiet           Suppress the live demo output
    -h, --help        Show this help

EXAMPLES:
    warsel
    warsel simulate --seed 7 --players 4 --seconds 120 --json
    warsel map --seed 7 --players 2
    warsel bench --players 8 --ticks 1200
    warsel demo --replay match.json"
}
