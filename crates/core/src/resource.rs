//! Resources and stockpiles.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum ResourceKind {
    Food,
    Wood,
    Stone,
    /// Unlocked from the Bronze age.
    Copper,
    /// Unlocked from the Iron age.
    Iron,
    Gold,
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 6] = [
        ResourceKind::Food,
        ResourceKind::Wood,
        ResourceKind::Stone,
        ResourceKind::Copper,
        ResourceKind::Iron,
        ResourceKind::Gold,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ResourceKind::Food => "food",
            ResourceKind::Wood => "wood",
            ResourceKind::Stone => "stone",
            ResourceKind::Copper => "copper",
            ResourceKind::Iron => "iron",
            ResourceKind::Gold => "gold",
        }
    }
}

/// Six resource amounts. `i64` because 62-player economies need headroom.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Cost {
    pub food: i64,
    pub wood: i64,
    pub stone: i64,
    pub copper: i64,
    pub iron: i64,
    pub gold: i64,
}

impl Cost {
    pub const ZERO: Cost = Cost {
        food: 0,
        wood: 0,
        stone: 0,
        copper: 0,
        iron: 0,
        gold: 0,
    };

    pub const fn new(food: i64, wood: i64, stone: i64) -> Cost {
        Cost {
            food,
            wood,
            stone,
            copper: 0,
            iron: 0,
            gold: 0,
        }
    }

    pub const fn with_gold(mut self, gold: i64) -> Cost {
        self.gold = gold;
        self
    }

    pub const fn with_copper(mut self, copper: i64) -> Cost {
        self.copper = copper;
        self
    }

    pub const fn with_iron(mut self, iron: i64) -> Cost {
        self.iron = iron;
        self
    }

    pub fn get(&self, kind: ResourceKind) -> i64 {
        match kind {
            ResourceKind::Food => self.food,
            ResourceKind::Wood => self.wood,
            ResourceKind::Stone => self.stone,
            ResourceKind::Copper => self.copper,
            ResourceKind::Iron => self.iron,
            ResourceKind::Gold => self.gold,
        }
    }

    pub fn set(&mut self, kind: ResourceKind, value: i64) {
        match kind {
            ResourceKind::Food => self.food = value,
            ResourceKind::Wood => self.wood = value,
            ResourceKind::Stone => self.stone = value,
            ResourceKind::Copper => self.copper = value,
            ResourceKind::Iron => self.iron = value,
            ResourceKind::Gold => self.gold = value,
        }
    }

    pub fn checked_add(self, other: Cost) -> Cost {
        Cost {
            food: self.food + other.food,
            wood: self.wood + other.wood,
            stone: self.stone + other.stone,
            copper: self.copper + other.copper,
            iron: self.iron + other.iron,
            gold: self.gold + other.gold,
        }
    }

    /// Saturating subtraction; use with care (clamp to zero explicitly).
    pub fn sub_clamped(self, other: Cost) -> Cost {
        Cost {
            food: (self.food - other.food).max(0),
            wood: (self.wood - other.wood).max(0),
            stone: (self.stone - other.stone).max(0),
            copper: (self.copper - other.copper).max(0),
            iron: (self.iron - other.iron).max(0),
            gold: (self.gold - other.gold).max(0),
        }
    }

    pub fn total(&self) -> i64 {
        self.food + self.wood + self.stone + self.copper + self.iron + self.gold
    }
}

/// What a worker is currently carrying.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Cargo {
    pub kind: Option<ResourceKind>,
    pub amount: i32,
}

impl Cargo {
    pub fn is_empty(&self) -> bool {
        self.amount <= 0 || self.kind.is_none()
    }

    pub fn take(&mut self) -> (Option<ResourceKind>, i32) {
        let out = (self.kind, self.amount);
        self.kind = None;
        self.amount = 0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_math() {
        let a = Cost::new(100, 50, 0).with_gold(10);
        let b = Cost::new(20, 25, 0).with_gold(5);
        assert_eq!(a.checked_add(b).food, 120);
        assert_eq!(a.sub_clamped(b).wood, 25);
        assert_eq!(a.get(ResourceKind::Gold), 10);
        assert_eq!(a.total(), 160);
    }
}
