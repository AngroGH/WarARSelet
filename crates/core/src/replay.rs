//! Replays and periodic state snapshots.
//!
//! A replay is just `config + seed + the ordered command stream`. Because the
//! simulation is deterministic, replaying the commands reproduces the match
//! exactly. Snapshots (world state + checksum) are used for reconnecting
//! clients and for desync detection.

use serde::{Deserialize, Serialize};

use crate::checksum::hash_world;
use crate::command::{Command, CommandResult};
use crate::ids::PlayerId;
use crate::world::{MatchConfig, World};

pub const REPLAY_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayHeader {
    pub version: u32,
    pub config: MatchConfig,
    pub players: Vec<ReplayPlayer>,
    pub created_tick: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayPlayer {
    pub name: String,
    pub team: u8,
    pub is_ai: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayRecord {
    pub tick: u64,
    pub player: u16,
    pub command: Command,
    /// Accepted flag and rejection reason, for post-mortems.
    pub accepted: bool,
    pub reason: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Replay {
    pub header: ReplayHeader,
    pub records: Vec<ReplayRecord>,
    /// State hash per recorded checkpoint (tick -> hash).
    pub checkpoints: Vec<(u64, u64)>,
}

impl Replay {
    pub fn new(config: MatchConfig, players: Vec<ReplayPlayer>) -> Self {
        Replay {
            header: ReplayHeader {
                version: REPLAY_FORMAT_VERSION,
                config,
                players,
                created_tick: 0,
            },
            records: Vec::new(),
            checkpoints: Vec::new(),
        }
    }

    pub fn record(&mut self, tick: u64, player: PlayerId, command: Command, result: &CommandResult) {
        let reason = result.reason.map(|r| r as u8);
        self.records.push(ReplayRecord {
            tick,
            player: player.0,
            command,
            accepted: result.accepted,
            reason,
        });
    }

    pub fn checkpoint(&mut self, world: &World) {
        self.checkpoints.push((world.tick().0, hash_world(world)));
    }

    /// Rebuild a world from the header and replay every accepted command in
    /// order, advancing the clock to each command's tick.
    pub fn replay(&self) -> World {
        let players = self
            .header
            .players
            .iter()
            .map(|p| (p.name.clone(), p.team, p.is_ai))
            .collect();
        let mut world = World::new(self.header.config.clone(), players);
        let mut clock = 0u64;
        for r in &self.records {
            if !r.accepted {
                continue;
            }
            while clock < r.tick {
                world.step();
                clock += 1;
            }
            let _ = world.apply_command(PlayerId(r.player), r.command.clone());
        }
        world
    }

    /// Verify that the replayed world matches all recorded checkpoints.
    pub fn verify(&self) -> Result<(), (u64, u64, u64)> {
        let players = self
            .header
            .players
            .iter()
            .map(|p| (p.name.clone(), p.team, p.is_ai))
            .collect();
        let mut world = World::new(self.header.config.clone(), players);
        let mut ci = 0;
        let mut record_i = 0;
        let last_tick = self.checkpoints.last().map(|(t, _)| *t).unwrap_or(0);
        for tick in 0..=last_tick {
            while record_i < self.records.len() && self.records[record_i].tick < tick {
                record_i += 1;
            }
            while record_i < self.records.len() && self.records[record_i].tick == tick {
                let r = &self.records[record_i];
                if r.accepted {
                    let _ = world.apply_command(PlayerId(r.player), r.command.clone());
                }
                record_i += 1;
            }
            while ci < self.checkpoints.len() && self.checkpoints[ci].0 == tick {
                let expected = self.checkpoints[ci].1;
                let actual = hash_world(&world);
                if expected != actual {
                    return Err((tick, expected, actual));
                }
                ci += 1;
            }
            if tick < last_tick {
                world.step();
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn from_json(json: &str) -> Option<Replay> {
        serde_json::from_str(json).ok()
    }

    pub fn duration_ticks(&self) -> u64 {
        self.checkpoints.last().map(|(t, _)| *t).unwrap_or(0)
    }
}

/// Full snapshot for a reconnecting client.
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub hash: u64,
    pub world: World,
}

impl Snapshot {
    pub fn capture(world: &World) -> Self {
        Snapshot {
            tick: world.tick().0,
            hash: hash_world(world),
            world: world.clone(),
        }
    }

    /// Restore a world, verifying its checksum first.
    pub fn restore(self) -> Option<World> {
        if hash_world(&self.world) == self.hash {
            Some(self.world)
        } else {
            None
        }
    }
}
