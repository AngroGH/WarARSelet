//! Fog of war.
//!
//! Visibility is computed on a coarse grid (2x2 tiles). Clients only receive
//! entities inside their own visible cells, which is the server-side half of
//! the anti-cheat contract: a hacked client cannot render what it was never
//! told about.

use serde::{Deserialize, Serialize};

use crate::map::Map;
use crate::math::{TilePos, Vec2};

pub const FOG_CELL: i32 = 2;
/// How often the visible set is recomputed (ticks). 10 ticks == 0.5 s.
pub const FOG_REFRESH_TICKS: u32 = 10;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FogGrid {
    cols: i32,
    rows: i32,
    /// 1 == currently visible.
    visible: Vec<u8>,
    /// 1 == ever seen.
    explored: Vec<u8>,
}

impl FogGrid {
    pub fn new(map: &Map) -> Self {
        let cols = (map.width + FOG_CELL - 1) / FOG_CELL;
        let rows = (map.height + FOG_CELL - 1) / FOG_CELL;
        let n = (cols * rows) as usize;
        FogGrid {
            cols,
            rows,
            visible: vec![0; n],
            explored: vec![0; n],
        }
    }

    pub fn resize(&mut self, map: &Map) {
        *self = FogGrid::new(map);
    }

    pub fn clear_visible(&mut self) {
        self.visible.fill(0);
    }

    fn cell_of(&self, x: i32, y: i32) -> Option<usize> {
        let cx = x / FOG_CELL;
        let cy = y / FOG_CELL;
        if cx < 0 || cy < 0 || cx >= self.cols || cy >= self.rows {
            return None;
        }
        Some((cy * self.cols + cx) as usize)
    }

    pub fn reveal_circle(&mut self, center: Vec2, radius: i32) {
        if radius <= 0 {
            return;
        }
        let cx = center.x.to_int();
        let cy = center.y.to_int();
        let r_tiles = (radius + 255) / 256;
        let r2 = radius as i64 * radius as i64;
        let min_x = ((cx - r_tiles) / FOG_CELL).max(0);
        let max_x = ((cx + r_tiles) / FOG_CELL).min(self.cols - 1);
        let min_y = ((cy - r_tiles) / FOG_CELL).max(0);
        let max_y = ((cy + r_tiles) / FOG_CELL).min(self.rows - 1);
        for gy in min_y..=max_y {
            for gx in min_x..=max_x {
                // Distance from the circle centre to the cell centre.
                let tile_x = gx * FOG_CELL + FOG_CELL / 2;
                let tile_y = gy * FOG_CELL + FOG_CELL / 2;
                let dx = (tile_x - cx) as i64;
                let dy = (tile_y - cy) as i64;
                if dx * dx + dy * dy <= r2 {
                    let i = (gy * self.cols + gx) as usize;
                    self.visible[i] = 1;
                    self.explored[i] = 1;
                }
            }
        }
    }

    pub fn is_visible_tile(&self, p: TilePos) -> bool {
        match self.cell_of(p.x, p.y) {
            Some(i) => self.visible[i] == 1,
            None => false,
        }
    }

    pub fn is_visible(&self, pos: Vec2) -> bool {
        self.is_visible_tile(TilePos::from_vec(pos))
    }

    pub fn is_explored_tile(&self, p: TilePos) -> bool {
        match self.cell_of(p.x, p.y) {
            Some(i) => self.explored[i] == 1,
            None => false,
        }
    }

    pub fn visible_ratio(&self) -> i32 {
        let seen = self.visible.iter().filter(|v| **v == 1).count();
        if self.visible.is_empty() {
            0
        } else {
            (seen * 1000 / self.visible.len()) as i32
        }
    }

    pub fn explored_ratio(&self) -> i32 {
        let seen = self.explored.iter().filter(|v| **v == 1).count();
        if self.explored.is_empty() {
            0
        } else {
            (seen * 1000 / self.explored.len()) as i32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapSize;

    #[test]
    fn fog_reveals_and_persists() {
        let map = Map::generate(3, MapSize::Duel, 2);
        let mut fog = FogGrid::new(&map);
        let p = TilePos::new(20, 20);
        assert!(!fog.is_visible_tile(p));
        fog.reveal_circle(p.center(), 6 * 256);
        assert!(fog.is_visible_tile(p));
        assert!(fog.is_explored_tile(p));
        fog.clear_visible();
        assert!(!fog.is_visible_tile(p));
        assert!(fog.is_explored_tile(p));
    }
}
