//! Deterministic grid A* with 8-way movement.
//!
//! Nodes are tile cells. Costs are integers (cardinal 10, diagonal 14) and
//! the heap tie-breaks on insertion order, so equal-cost paths always resolve
//! the same way on every machine.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::map::Map;
use crate::math::TilePos;

pub const CARDINAL: u32 = 10;
pub const DIAGONAL: u32 = 14;

/// Reusable A* scratch space. Keeping it in one struct avoids re-allocating
/// multi-megabyte vectors for every order.
#[derive(Default, Clone)]
pub struct Pathfinder {
    g_score: Vec<u32>,
    came_from: Vec<i32>,
    stamped: Vec<u32>,
    stamp: u32,
    width: i32,
    height: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Node {
    /// f = g + h, then g, then insertion order: fully deterministic ordering.
    f: u32,
    g: u32,
    idx: u32,
    order: u32,
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Natural ordering; the heap stores `Reverse<Node>` so the smallest
        // f is popped first. Tie-break on g, then insertion order, then index
        // to keep the search fully deterministic.
        self.f
            .cmp(&other.f)
            .then_with(|| self.g.cmp(&other.g))
            .then_with(|| other.order.cmp(&self.order))
            .then_with(|| other.idx.cmp(&self.idx))
    }
}

const UNREACHABLE: u32 = u32::MAX;

impl Pathfinder {
    pub fn new(map: &Map) -> Self {
        let n = (map.width * map.height) as usize;
        Pathfinder {
            g_score: vec![UNREACHABLE; n],
            came_from: vec![-1; n],
            stamped: vec![0; n],
            stamp: 0,
            width: map.width,
            height: map.height,
        }
    }

    pub fn resize(&mut self, map: &Map) {
        let n = (map.width * map.height) as usize;
        self.g_score = vec![UNREACHABLE; n];
        self.came_from = vec![-1; n];
        self.stamped = vec![0; n];
        self.stamp = 0;
        self.width = map.width;
        self.height = map.height;
    }

    #[inline]
    fn idx(&self, x: i32, y: i32) -> usize {
        (y as usize) * (self.width as usize) + (x as usize)
    }

    /// Find a walkable path from `start` to `goal`.
    ///
    /// `start` and `goal` are expected to be walkable; if `goal` is not, the
    /// caller should snap it first (see `Map::nearest_walkable`).
    /// `blocked` lets a caller treat extra cells as impassable (unit
    /// avoidance is handled by a cheap local repulsion instead).
    pub fn find_path(
        &mut self,
        map: &Map,
        start: TilePos,
        goal: TilePos,
        max_nodes: u32,
    ) -> Option<Vec<TilePos>> {
        if start == goal {
            return Some(Vec::new());
        }
        if !self.in_bounds(start) || !self.in_bounds(goal) {
            return None;
        }

        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            // Stamp overflow: reset bookkeeping (rare, once per 4B searches).
            self.stamped.fill(0);
            self.g_score.fill(UNREACHABLE);
            self.stamp = 1;
        }
        let stamp = self.stamp;

        let start_i = self.idx(start.x, start.y);
        let goal_i = self.idx(goal.x, goal.y);
        self.g_score[start_i] = 0;
        self.stamped[start_i] = stamp;
        self.came_from[start_i] = -1;

        let mut open: BinaryHeap<Reverse<Node>> = BinaryHeap::with_capacity(256);
        let mut order = 0u32;
        open.push(Reverse(Node {
            f: heuristic(start, goal) as u32,
            g: 0,
            idx: start_i as u32,
            order,
        }));

        let mut expanded = 0u32;
        let mut best_idx = start_i;
        let mut best_h = heuristic(start, goal);

        while let Some(Reverse(node)) = open.pop() {
            if node.idx as usize == goal_i {
                return Some(self.reconstruct(goal_i, start_i));
            }
            let cur = node.idx as usize;
            if node.g > self.g_score[cur] {
                continue;
            }
            expanded += 1;
            if expanded > max_nodes {
                // Budget exceeded: keep whatever partial progress we have so
                // units keep drifting toward the goal, but never report a
                // bogus "arrived" path.
                if best_idx != start_i {
                    return Some(self.reconstruct(best_idx, start_i));
                }
                return None;
            }

            let cx = (cur as i32) % self.width;
            let cy = (cur as i32) / self.width;

            for (dx, dy, cost) in NEIGHBOURS {
                let nx = cx + dx;
                let ny = cy + dy;
                if !self.in_bounds(TilePos::new(nx, ny)) {
                    continue;
                }
                if !map.walkable(nx, ny) {
                    continue;
                }
                // Disallow cutting diagonal corners through blocked tiles.
                if dx != 0
                    && dy != 0
                    && (!map.walkable(cx + dx, cy) || !map.walkable(cx, cy + dy))
                {
                    continue;
                }
                let ni = self.idx(nx, ny);
                let tentative = node.g + cost;
                if self.stamped[ni] != stamp || tentative < self.g_score[ni] {
                    self.stamped[ni] = stamp;
                    self.g_score[ni] = tentative;
                    self.came_from[ni] = cur as i32;
                    let h = heuristic(TilePos::new(nx, ny), goal);
                    if h < best_h {
                        best_h = h;
                        best_idx = ni;
                    }
                    order = order.wrapping_add(1);
                    open.push(Reverse(Node {
                        f: tentative + h as u32,
                        g: tentative,
                        idx: ni as u32,
                        order,
                    }));
                }
            }
        }

        if best_idx != start_i {
            Some(self.reconstruct(best_idx, start_i))
        } else {
            None
        }
    }

    fn in_bounds(&self, p: TilePos) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < self.width && p.y < self.height
    }

    fn reconstruct(&self, mut idx: usize, start: usize) -> Vec<TilePos> {
        let mut out = Vec::new();
        while idx != start {
            let x = (idx as i32) % self.width;
            let y = (idx as i32) / self.width;
            out.push(TilePos::new(x, y));
            let prev = self.came_from[idx];
            if prev < 0 {
                break;
            }
            idx = prev as usize;
        }
        out.reverse();
        out
    }
}

const NEIGHBOURS: [(i32, i32, u32); 8] = [
    (1, 0, CARDINAL),
    (-1, 0, CARDINAL),
    (0, 1, CARDINAL),
    (0, -1, CARDINAL),
    (1, 1, DIAGONAL),
    (1, -1, DIAGONAL),
    (-1, 1, DIAGONAL),
    (-1, -1, DIAGONAL),
];

#[inline]
fn heuristic(a: TilePos, b: TilePos) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    let diag = dx.min(dy);
    let straight = dx.max(dy) - diag;
    diag * DIAGONAL + straight * CARDINAL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapSize;

    fn open_map() -> Map {
        // A tiny hand-made map: all grass by generating then flattening water.
        let mut m = Map::generate(1, MapSize::Duel, 2);
        m.starts.clear();
        m
    }

    #[test]
    fn path_exists_on_generated_map() {
        let map = open_map();
        let start = *map
            .starts
            .first()
            .unwrap_or(&TilePos::new(0, 0));
        let _ = start;
        let a = (0..map.width)
            .flat_map(|x| (0..map.height).map(move |y| TilePos::new(x, y)))
            .find(|p| map.walkable_pos(*p))
            .expect("some walkable tile");
        let mut pf = Pathfinder::new(&map);
        // Walk to a nearby walkable tile.
        let b = (0..map.width)
            .flat_map(|x| (0..map.height).map(move |y| TilePos::new(x, y)))
            .filter(|p| map.walkable_pos(*p) && p.dist(a) > 8)
            .next();
        if let Some(b) = b {
            let path = pf.find_path(&map, a, b, 100_000);
            assert!(path.is_some(), "expected a path between walkable tiles");
            let path = path.unwrap();
            assert!(!path.is_empty());
            // Path must only contain walkable tiles and end at the goal.
            for step in &path {
                assert!(map.walkable_pos(*step));
            }
            assert_eq!(*path.last().unwrap(), b);
        }
    }

    #[test]
    fn same_start_goal_is_empty_path() {
        let map = open_map();
        let mut pf = Pathfinder::new(&map);
        let a = TilePos::new(5, 5);
        assert_eq!(pf.find_path(&map, a, a, 1000), Some(Vec::new()));
    }
}
