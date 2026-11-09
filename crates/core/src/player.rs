//! Player state: stockpile, population, epoch and civilization path.

use serde::{Deserialize, Serialize};

use crate::epoch::{Branch, CivPath, Country, Culture, Epoch};
use crate::ids::PlayerId;
use crate::resource::{Cost, ResourceKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PlayerStatus {
    Active,
    Eliminated,
    /// Left the match on purpose.
    Resigned,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    /// 0 = no team (FFA).
    pub team: u8,
    pub color: u8,
    pub status: PlayerStatus,
    pub is_ai: bool,
    pub resources: Cost,
    pub epoch: Epoch,
    pub civ: CivPath,
    pub population: i32,
    pub pop_cap: i32,
    /// Research flags (bitmask), used by the academy.
    pub research_bits: u32,
    /// Ticks since the wonder was completed, 0 when no wonder.
    pub wonder_ticks: u32,
    pub kills: u32,
    pub losses: u32,
    pub gathered: i64,
    /// Tick at which the player was eliminated (for scoring/spectators).
    pub eliminated_at: Option<u64>,
    /// Ticks since the epoch advance was requested (used for the "confirm"
    /// window in the UI); advancing is applied instantly on the server.
    pub pending_epoch: Option<Epoch>,
}

impl Player {
    pub fn new(id: PlayerId, name: String, team: u8) -> Self {
        Player {
            id,
            name,
            team,
            color: id.0 as u8 % 12,
            status: PlayerStatus::Active,
            is_ai: false,
            resources: Cost::new(200, 200, 100).with_gold(50),
            epoch: Epoch::Stone,
            civ: CivPath::default(),
            population: 0,
            pop_cap: 15,
            research_bits: 0,
            wonder_ticks: 0,
            kills: 0,
            losses: 0,
            gathered: 0,
            eliminated_at: None,
            pending_epoch: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == PlayerStatus::Active
    }

    /// Voluntary defeat: the player keeps their stuff but stops counting as
    /// an opponent (spectator mode).
    pub fn resigned(&mut self) {
        if self.status == PlayerStatus::Active {
            self.status = PlayerStatus::Resigned;
        }
    }

    pub fn can_afford(&self, cost: Cost) -> bool {
        for k in ResourceKind::ALL {
            let need = cost.get(k);
            if need > 0 && self.resources.get(k) < need {
                return false;
            }
        }
        true
    }

    pub fn pay(&mut self, cost: Cost) -> bool {
        if !self.can_afford(cost) {
            return false;
        }
        for k in ResourceKind::ALL {
            let need = cost.get(k);
            if need > 0 {
                self.resources.set(k, self.resources.get(k) - need);
            }
        }
        true
    }

    pub fn refund(&mut self, cost: Cost) {
        for k in ResourceKind::ALL {
            let v = cost.get(k);
            if v > 0 {
                self.resources.set(k, self.resources.get(k) + v);
            }
        }
    }

    pub fn add(&mut self, kind: ResourceKind, amount: i64) {
        let v = (self.resources.get(kind) + amount).max(0);
        self.resources.set(kind, v);
        if amount > 0 {
            self.gathered += amount;
        }
    }

    pub fn has_population_room(&self, pop: i32) -> bool {
        self.population + pop <= self.pop_cap
    }

    /// Description of the current civilization, for UI/debug.
    pub fn civ_label(&self) -> String {
        let culture = match self.civ.culture {
            Culture::Unchosen => "unchosen",
            Culture::Europe => "Europe",
            Culture::Asia => "Asia",
        };
        let branch = match self.civ.branch {
            Branch::Unchosen => "-",
            Branch::West => "West",
            Branch::East => "East",
        };
        let country = self.civ.country.name();
        let country = if self.civ.country == Country::Unchosen {
            "-"
        } else {
            country
        };
        format!("{culture}/{branch}/{country}")
    }
}
