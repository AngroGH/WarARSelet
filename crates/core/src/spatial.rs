//! Uniform spatial hash grid for neighbour queries.
//!
//! Uses plain `Vec` buckets and deterministic insertion order, so target
//! acquisition is reproducible tick-for-tick (a `HashMap` would not be).

use serde::{Deserialize, Serialize};

use crate::ids::EntityId;
use crate::math::{Vec2, SCALE};
use crate::map::Map;

pub const DEFAULT_CELL_TILES: i32 = 4;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SpatialGrid {
    cell: i32,
    cols: i32,
    rows: i32,
    buckets: Vec<Vec<EntityId>>,
}

impl SpatialGrid {
    pub fn new(map: &Map, cell: i32) -> Self {
        let cell = cell.max(1);
        let cols = map.width / cell + 1;
        let rows = map.height / cell + 1;
        SpatialGrid {
            cell,
            cols,
            rows,
            buckets: (0..(cols * rows)).map(|_| Vec::new()).collect(),
        }
    }

    pub fn clear(&mut self) {
        for b in &mut self.buckets {
            b.clear();
        }
    }

    #[inline]
    fn cell_index(&self, x: i32, y: i32) -> Option<usize> {
        let cx = x / self.cell;
        let cy = y / self.cell;
        if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
            return None;
        }
        Some((cy * self.cols + cx) as usize)
    }

    pub fn insert(&mut self, id: EntityId, pos: Vec2) {
        if let Some(i) = self.cell_index(pos.x.to_int(), pos.y.to_int()) {
            self.buckets[i].push(id);
        }
    }

    /// Collect entities within `radius` (in fixed units) of `center`.
    /// The output order is deterministic: bucket order, then insertion order.
    pub fn query_radius(&self, center: Vec2, radius: i32, out: &mut Vec<EntityId>) {
        out.clear();
        if radius <= 0 {
            return;
        }
        let r_tiles = (radius + SCALE - 1) / SCALE;
        let min_cx = ((center.x.to_int() - r_tiles) / self.cell).max(0);
        let max_cx = ((center.x.to_int() + r_tiles) / self.cell).min(self.cols - 1);
        let min_cy = ((center.y.to_int() - r_tiles) / self.cell).max(0);
        let max_cy = ((center.y.to_int() + r_tiles) / self.cell).min(self.rows - 1);
        let r2 = radius as i64 * radius as i64;
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                let b = &self.buckets[(cy * self.cols + cx) as usize];
                for id in b {
                    // Distance filtering happens in the caller (it knows the
                    // exact position); here we only prune whole buckets.
                    out.push(*id);
                }
            }
        }
        // Cheap sanity guard: the caller still validates the distance.
        let _ = r2;
    }

    pub fn cell_size(&self) -> i32 {
        self.cell
    }
}
