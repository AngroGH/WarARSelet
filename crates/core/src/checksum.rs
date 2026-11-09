//! Simulation state checksum.
//!
//! The server and clients (or a replay verifier) can compare hashes every N
//! ticks to detect desync. The hash covers only gameplay-relevant state, in a
//! fixed order, so it is stable across runs.

use crate::entity::Entity;
use crate::world::World;

/// FNV-1a based incremental hasher (deterministic, platform independent).
#[derive(Clone, Copy, Debug)]
pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> Self {
        StateHasher(0xcbf2_9ce4_8422_2325)
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write_u8(&mut self, v: u8) {
        self.0 ^= v as u64;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.write_u8((v & 0xff) as u8);
        self.write_u8((v >> 8) as u8);
    }

    pub fn write_u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.write_u8(b);
        }
    }

    pub fn write_u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.write_u8(b);
        }
    }

    pub fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// Hash the complete gameplay state of a world.
///
/// Iteration order is the arena slot order, which is insertion order with
/// recycling, therefore fully deterministic.
pub fn hash_world(world: &World) -> u64 {
    let mut h = StateHasher::new();
    h.write_u64(world.tick.0);
    h.write_u32(world.players.len() as u32);
    for p in &world.players {
        h.write_u16(p.id.0);
        h.write_i32(p.resources.food as i32);
        h.write_i32(p.resources.wood as i32);
        h.write_i32(p.resources.stone as i32);
        h.write_i32(p.resources.copper as i32);
        h.write_i32(p.resources.iron as i32);
        h.write_i32(p.resources.gold as i32);
        h.write_u8(p.epoch.index() as u8);
        h.write_i32(p.population);
        h.write_u8(p.status as u8);
    }
    for (id, e) in world.entities.iter() {
        h.write_u32(id.index.0);
        h.write_u16(id.generation);
        h.write_u16(e.owner().0);
        h.write_i32(e.hp());
        let pos = e.pos();
        h.write_i32(pos.x.to_bits());
        h.write_i32(pos.y.to_bits());
        match e {
            Entity::Unit(u) => {
                h.write_u8(1);
                h.write_u16(u.def.0);
                h.write_u8(u.state as u8);
                h.write_i32(u.cargo.amount);
            }
            Entity::Building(b) => {
                h.write_u8(2);
                h.write_u16(b.def.0);
                h.write_u32(b.work_done);
                h.write_u8(b.finished as u8);
            }
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn identical_worlds_hash_identically() {
        let a = World::skirmish(99, 2);
        let b = World::skirmish(99, 2);
        assert_eq!(hash_world(&a), hash_world(&b));
    }

    #[test]
    fn different_seeds_differ() {
        let a = World::skirmish(1, 2);
        let b = World::skirmish(2, 2);
        assert_ne!(hash_world(&a), hash_world(&b));
    }
}
