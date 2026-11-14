//! Procedural map generation and terrain grid.
//!
//! Maps are generated from a seed with integer-only noise, so the same seed
//! produces byte-identical terrain on every machine.

use serde::{Deserialize, Serialize};

use crate::math::{Fixed, TilePos};
use crate::resource::ResourceKind;
use crate::rng::Rng;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Terrain {
    #[default]
    Grass,
    Water,
    Rock,
}

impl Terrain {
    pub fn is_walkable(self) -> bool {
        self == Terrain::Grass
    }

    pub fn is_buildable(self) -> bool {
        self == Terrain::Grass
    }
}

/// A finite resource deposit. Nodes are not entities: they live in the map
/// and are consumed by workers, which keeps the entity arena small.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceNode {
    pub kind: ResourceKind,
    pub pos: TilePos,
    pub amount: i32,
    pub max_amount: i32,
    /// Trees / berry bushes: workers gather from any adjacent tile.
    pub blocking: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum MapSize {
    Duel,
    Small,
    Standard,
    Large,
}

impl MapSize {
    pub fn dims(self) -> i32 {
        match self {
            MapSize::Duel => 96,
            MapSize::Small => 128,
            MapSize::Standard => 160,
            MapSize::Large => 200,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Map {
    pub width: i32,
    pub height: i32,
    terrain: Vec<Terrain>,
    /// Building footprints and other hard blockers.
    blocked: Vec<bool>,
    pub nodes: Vec<ResourceNode>,
    /// Node index per tile, `-1` when empty.
    node_at: Vec<i32>,
    pub starts: Vec<TilePos>,
}

impl Map {
    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y as usize) * (self.width as usize) + (x as usize)
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    pub fn in_bounds_pos(&self, p: TilePos) -> bool {
        self.in_bounds(p.x, p.y)
    }

    pub fn terrain(&self, x: i32, y: i32) -> Terrain {
        if !self.in_bounds(x, y) {
            return Terrain::Rock;
        }
        self.terrain[self.idx(x, y)]
    }

    pub fn terrain_at(&self, p: TilePos) -> Terrain {
        self.terrain(p.x, p.y)
    }

    /// Walkable for ground units.
    pub fn walkable(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y)
            && self.terrain[self.idx(x, y)].is_walkable()
            && !self.blocked[self.idx(x, y)]
            && self.node_at[self.idx(x, y)] < 0
    }

    /// Buildable tile (ignores units).
    pub fn buildable(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y)
            && self.terrain[self.idx(x, y)].is_buildable()
            && !self.blocked[self.idx(x, y)]
            && self.node_at[self.idx(x, y)] < 0
    }

    pub fn walkable_pos(&self, p: TilePos) -> bool {
        self.walkable(p.x, p.y)
    }

    /// Mark a square footprint as blocked/unblocked.
    pub fn set_blocked_rect(&mut self, origin: TilePos, size: i32, value: bool) {
        for y in origin.y..origin.y + size {
            for x in origin.x..origin.x + size {
                if self.in_bounds(x, y) {
                    let i = self.idx(x, y);
                    self.blocked[i] = value;
                }
            }
        }
    }

    pub fn is_blocked(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y) && self.blocked[self.idx(x, y)]
    }

    /// Nearest walkable tile using a deterministic ring search.
    pub fn nearest_walkable(&self, from: TilePos, max_radius: i32) -> Option<TilePos> {
        if self.walkable_pos(from) {
            return Some(from);
        }
        for r in 1..=max_radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let p = TilePos::new(from.x + dx, from.y + dy);
                    if self.walkable_pos(p) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }

    pub fn node_index_at(&self, p: TilePos) -> Option<usize> {
        if !self.in_bounds_pos(p) {
            return None;
        }
        let i = self.node_at[self.idx(p.x, p.y)];
        if i >= 0 {
            Some(i as usize)
        } else {
            None
        }
    }

    pub fn node(&self, index: usize) -> Option<&ResourceNode> {
        self.nodes.get(index)
    }

    /// Remove up to `amount` from a node, returning the amount actually taken.
    pub fn take_from_node(&mut self, index: usize, amount: i32) -> i32 {
        let Some(node) = self.nodes.get_mut(index) else {
            return 0;
        };
        let taken = amount.min(node.amount).max(0);
        node.amount -= taken;
        taken
    }

    /// Nearest node of `kind` visible from `from` (ring search up to `radius`).
    pub fn nearest_node(&self, from: Vec2ish, kind: ResourceKind, radius: i32) -> Option<usize> {
        let origin = TilePos::new(from.0, from.1);
        let mut best: Option<(i32, usize)> = None;
        for r in 0..=radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let p = TilePos::new(origin.x + dx, origin.y + dy);
                    let Some(i) = self.node_index_at(p) else {
                        continue;
                    };
                    let n = &self.nodes[i];
                    if n.kind != kind || n.amount <= 0 {
                        continue;
                    }
                    let d = p.dist(origin);
                    if best.map_or(true, |(bd, _)| d < bd) {
                        best = Some((d, i));
                    }
                }
            }
            // Everything beyond the first two rings is already farther than
            // anything we could find later, so stop early when we have a hit.
            if let Some((d, _)) = best {
                if d <= r {
                    break;
                }
            }
        }
        best.map(|(_, i)| i)
    }

    pub fn alive_nodes(&self) -> impl Iterator<Item = (usize, &ResourceNode)> {
        self.nodes.iter().enumerate().filter(|(_, n)| n.amount > 0)
    }

    pub fn total_nodes_of(&self, kind: ResourceKind) -> i32 {
        self.nodes
            .iter()
            .filter(|n| n.kind == kind && n.amount > 0)
            .count() as i32
    }
}

/// Tiny helper so `nearest_node` does not depend on `Vec2` import ordering.
#[derive(Clone, Copy)]
pub struct Vec2ish(pub i32, pub i32);

impl From<TilePos> for Vec2ish {
    fn from(p: TilePos) -> Self {
        Vec2ish(p.x, p.y)
    }
}

impl Map {
    /// Procedurally generate a map: continents, scattered deposits and one
    /// start position per player.
    pub fn generate(seed: u64, size: MapSize, players: u16) -> Map {
        let dim = size.dims();
        let mut rng = Rng::seeded(seed);
        let w = dim;
        let h = dim;

        // Value-noise height field, two octaves.
        let low = noise_field(&mut rng, 8);
        let high = noise_field(&mut rng, 24);
        let mut height = vec![0i32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let l = sample_field(&low, 8, 8, x, y, w, h, 1400);
                let hi = sample_field(&high, 24, 24, x, y, w, h, 700);
                let edge = edge_falloff(x, y, w, h);
                height[(y * w + x) as usize] = ((l + hi) * edge) / 1000;
            }
        }

        let mut terrain = vec![Terrain::Grass; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = height[(y * w + x) as usize];
                let t = if v < 400 {
                    Terrain::Water
                } else if v > 1480 {
                    Terrain::Rock
                } else {
                    Terrain::Grass
                };
                terrain[(y * w + x) as usize] = t;
            }
        }

        let mut map = Map {
            width: w,
            height: h,
            terrain,
            blocked: vec![false; (w * h) as usize],
            nodes: Vec::new(),
            node_at: vec![-1; (w * h) as usize],
            starts: Vec::new(),
        };

        // Start positions spread on a ring, snapped to the closest large
        // grass patch.
        let count = players.max(2);
        let radius = (w * 34) / 100;
        let cx = w / 2;
        let cy = h / 2;
        for i in 0..count {
            // Deterministic ring distribution without floating point angles:
            // use a fixed lookup table of unit directions.
            let (dx, dy) = ring_dir(i, count);
            let mut tx = cx + dx * radius / 1000;
            let mut ty = cy + dy * radius / 1000;
            tx = tx.clamp(4, w - 5);
            ty = ty.clamp(4, h - 5);
            let snapped = map
                .nearest_grass_area(TilePos::new(tx, ty), 24)
                .unwrap_or(TilePos::new(cx, cy));
            map.starts.push(snapped);
        }
        // Deduplicate overlapping starts (small maps, many players).
        for i in 0..map.starts.len() {
            for j in 0..i {
                if map.starts[i].dist(map.starts[j]) < 10 {
                    let alt = map
                        .nearest_grass_area(
                            TilePos::new(
                                map.starts[i].x + rng.in_range(-20, 21),
                                map.starts[i].y + rng.in_range(-20, 21),
                            ),
                            30,
                        )
                        .unwrap_or(map.starts[i]);
                    map.starts[i] = alt;
                }
            }
        }

        // Resource deposits near every start (fair opening) ...
        for start in map.starts.clone() {
            map.scatter_cluster(&mut rng, start, ResourceKind::Food, 3, 12, 60, 500);
            map.scatter_cluster(&mut rng, start, ResourceKind::Wood, 4, 16, 90, 400);
            map.scatter_cluster(&mut rng, start, ResourceKind::Stone, 2, 8, 60, 350);
        }
        // ... and contested deposits across the whole map.
        let mut placed = 0;
        let mut guard = 0;
        while placed < 26 && guard < 5000 {
            guard += 1;
            let p = TilePos::new(rng.in_range(4, w - 4), rng.in_range(4, h - 4));
            if !map.buildable(p.x, p.y) {
                continue;
            }
            let kind = match rng.below(4) {
                0 => ResourceKind::Stone,
                1 => ResourceKind::Copper,
                2 => ResourceKind::Iron,
                _ => ResourceKind::Gold,
            };
            map.scatter_cluster(&mut rng, p, kind, 1, 6, 40, 300);
            placed += 1;
        }
        // Forest patches for wood.
        for _ in 0..(w * h / 900) {
            let p = TilePos::new(rng.in_range(4, w - 4), rng.in_range(4, h - 4));
            if map.buildable(p.x, p.y) {
                map.scatter_cluster(&mut rng, p, ResourceKind::Wood, 10, 20, 60, 250);
            }
        }

        // Guarantee a connected playable area: flood fill from the first start
        // and turn every unreachable grass pocket into rock. Without this the
        // generator can create islands (and resource clusters on them) that
        // units can never path to.
        map.prune_unreachable();
        map.prune_orphan_nodes();
        map
    }

    /// Flood fill from the first start; unreachable land becomes rock.
    fn prune_unreachable(&mut self) {
        let Some(start) = self.starts.first().copied() else {
            return;
        };
        let start = match self.nearest_walkable(start, 10) {
            Some(p) => p,
            None => return,
        };
        let mut reachable = vec![false; (self.width * self.height) as usize];
        let mut queue = std::collections::VecDeque::with_capacity(1024);
        queue.push_back(start);
        reachable[self.idx(start.x, start.y)] = true;
        while let Some(p) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nx = p.x + dx;
                let ny = p.y + dy;
                if !self.in_bounds(nx, ny) {
                    continue;
                }
                let i = self.idx(nx, ny);
                if reachable[i] {
                    continue;
                }
                if !self.walkable(nx, ny) {
                    continue;
                }
                reachable[i] = true;
                queue.push_back(TilePos::new(nx, ny));
            }
        }
        for y in 0..self.height {
            for x in 0..self.width {
                let i = self.idx(x, y);
                if !reachable[i] && self.terrain[i] == Terrain::Grass {
                    self.terrain[i] = Terrain::Rock;
                }
            }
        }
    }

    /// Drop resource nodes that no walkable tile can reach (they became
    /// isolated when the map was pruned).
    fn prune_orphan_nodes(&mut self) {
        let mut keep = vec![true; self.nodes.len()];
        for (i, node) in self.nodes.iter().enumerate() {
            let p = node.pos;
            let mut reachable = false;
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                if self.walkable(p.x + dx, p.y + dy) {
                    reachable = true;
                    break;
                }
            }
            keep[i] = reachable;
        }
        if keep.iter().all(|k| *k) {
            return;
        }
        let mut remapped = Vec::with_capacity(self.nodes.len());
        let mut new_index = vec![-1i32; self.nodes.len()];
        for (i, node) in self.nodes.iter().enumerate() {
            if keep[i] {
                new_index[i] = remapped.len() as i32;
                remapped.push(node.clone());
            }
        }
        self.nodes = remapped;
        self.node_at.fill(-1);
        for (i, node) in self.nodes.iter().enumerate() {
            let idx = self.idx(node.pos.x, node.pos.y);
            self.node_at[idx] = i as i32;
            let _ = new_index[i];
        }
    }

    fn nearest_grass_area(&self, from: TilePos, radius: i32) -> Option<TilePos> {
        let mut best: Option<(i32, TilePos)> = None;
        for r in 0..=radius {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let p = TilePos::new(from.x + dx, from.y + dy);
                    if !self.in_bounds_pos(p) {
                        continue;
                    }
                    if self.terrain_at(p) != Terrain::Grass {
                        continue;
                    }
                    // Prefer spots with a lot of free land around them.
                    let mut free = 0;
                    for oy in -2..=2 {
                        for ox in -2..=2 {
                            if self.terrain(p.x + ox, p.y + oy) == Terrain::Grass {
                                free += 1;
                            }
                        }
                    }
                    if free < 20 {
                        continue;
                    }
                    let score = (p.dist(from), p);
                    if best.map_or(true, |(bd, _)| score.0 < bd) {
                        best = Some(score);
                    }
                    if r == 0 {
                        return Some(p);
                    }
                }
            }
            if let Some((d, p)) = best {
                if d <= r {
                    return Some(p);
                }
            }
        }
        best.map(|(_, p)| p)
    }

    #[allow(clippy::too_many_arguments)]
    fn scatter_cluster(
        &mut self,
        rng: &mut Rng,
        center: TilePos,
        kind: ResourceKind,
        min_tiles: i32,
        max_tiles: i32,
        amount_per_tile: i32,
        spread: i32,
    ) {
        let n = rng.in_range(min_tiles, max_tiles + 1);
        let mut placed = 0;
        let mut attempts = 0;
        while placed < n && attempts < n * 30 + 40 {
            attempts += 1;
            let x = center.x + rng.in_range(-spread, spread + 1);
            let y = center.y + rng.in_range(-spread, spread + 1);
            if !self.buildable(x, y) {
                continue;
            }
            let p = TilePos::new(x, y);
            if self.node_index_at(p).is_some() {
                continue;
            }
            let amount = amount_per_tile + rng.in_range(0, amount_per_tile / 2 + 1);
            self.nodes.push(ResourceNode {
                kind,
                pos: p,
                amount,
                max_amount: amount,
                blocking: matches!(kind, ResourceKind::Wood),
            });
            let i = self.idx(x, y);
            self.node_at[i] = (self.nodes.len() - 1) as i32;
            placed += 1;
        }
    }
}

pub fn edge_falloff(x: i32, y: i32, w: i32, h: i32) -> i32 {
    let margin = 4;
    let dx = x.min(w - 1 - x).min(margin);
    let dy = y.min(h - 1 - y).min(margin);
    let d = dx.min(dy);
    // 0 at the border, 1000 past the margin.
    1000 * d / margin.max(1)
}

fn ring_dir(i: u16, count: u16) -> (i32, i32) {
    // 32 fixed unit directions, no trig and no floats.
    const DIRS: [(i32, i32); 32] = [
        (1000, 0),
        (981, 195),
        (924, 383),
        (831, 556),
        (707, 707),
        (556, 831),
        (383, 924),
        (195, 981),
        (0, 1000),
        (-195, 981),
        (-383, 924),
        (-556, 831),
        (-707, 707),
        (-831, 556),
        (-924, 383),
        (-981, 195),
        (-1000, 0),
        (-981, -195),
        (-924, -383),
        (-831, -556),
        (-707, -707),
        (-556, -831),
        (-383, -924),
        (-195, -981),
        (0, -1000),
        (195, -981),
        (383, -924),
        (556, -831),
        (707, -707),
        (831, -556),
        (924, -383),
        (981, -195),
    ];
    let stride = (32 / count.max(1) as usize).max(1);
    DIRS[(i as usize * stride) % 32]
}

fn noise_field(rng: &mut Rng, cells: usize) -> Vec<i32> {
    (0..(cells * cells))
        .map(|_| rng.in_range(0, 1001))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn sample_field(
    field: &[i32],
    gw: usize,
    gh: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    amplitude: i32,
) -> i32 {
    // Map tile -> grid space in fixed point.
    let gx = Fixed::from_int(x) * Fixed::from_int(gw as i32) / Fixed::from_int(w);
    let gy = Fixed::from_int(y) * Fixed::from_int(gh as i32) / Fixed::from_int(h);
    let x0 = gx.to_int().clamp(0, gw as i32 - 1);
    let y0 = gy.to_int().clamp(0, gh as i32 - 1);
    let x1 = (x0 + 1).min(gw as i32 - 1);
    let y1 = (y0 + 1).min(gh as i32 - 1);
    let fx = gx - Fixed::from_int(x0);
    let fy = gy - Fixed::from_int(y0);

    let at = |xx: i32, yy: i32| -> Fixed { Fixed::from_int(field[yy as usize * gw + xx as usize]) };
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
    let value = top + (bottom - top) * fy;
    value.to_int() * amplitude / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() {
        let a = Map::generate(42, MapSize::Duel, 4);
        let b = Map::generate(42, MapSize::Duel, 4);
        assert_eq!(a.terrain, b.terrain);
        assert_eq!(a.starts, b.starts);
        assert_eq!(a.nodes.len(), b.nodes.len());
        assert_eq!(a.nodes[0].pos, b.nodes[0].pos);
    }

    #[test]
    fn generation_creates_playable_starts() {
        let map = Map::generate(7, MapSize::Duel, 4);
        assert_eq!(map.starts.len(), 4);
        for s in &map.starts {
            assert!(map.walkable_pos(*s) || map.nearest_walkable(*s, 6).is_some());
        }
        assert!(map.nodes.len() > 30);
    }

    #[test]
    fn unique_seeds_differ() {
        let a = Map::generate(1, MapSize::Duel, 2);
        let b = Map::generate(2, MapSize::Duel, 2);
        assert_ne!(a.terrain, b.terrain);
    }
}
