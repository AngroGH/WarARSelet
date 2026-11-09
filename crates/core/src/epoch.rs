//! Epoch progression and in-match civilization branching.
//!
//! Per the GDD the faction is *not* picked up front:
//! 1. Bronze age  -> choose a culture (Europe / Asia)
//! 2. Middle ages -> choose a branch (West / East)
//! 3. Industry    -> choose a country (unique units and buildings)

use serde::{Deserialize, Serialize};

use crate::resource::{Cost, ResourceKind};

/// The 7 MVP epochs.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Epoch {
    #[default]
    Stone = 0,
    Bronze = 1,
    Iron = 2,
    EarlyMedieval = 3,
    LateMedieval = 4,
    Industry1 = 5,
    Industry2 = 6,
}

impl Epoch {
    pub const COUNT: usize = 7;

    pub const ALL: [Epoch; Epoch::COUNT] = [
        Epoch::Stone,
        Epoch::Bronze,
        Epoch::Iron,
        Epoch::EarlyMedieval,
        Epoch::LateMedieval,
        Epoch::Industry1,
        Epoch::Industry2,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Epoch::Stone => "Stone Age",
            Epoch::Bronze => "Bronze Age",
            Epoch::Iron => "Iron Age",
            Epoch::EarlyMedieval => "Early Middle Ages",
            Epoch::LateMedieval => "Late Middle Ages",
            Epoch::Industry1 => "Industrial Era I",
            Epoch::Industry2 => "Industrial Era II",
        }
    }

    pub fn next(self) -> Option<Epoch> {
        Epoch::ALL.get(self.index() + 1).copied()
    }

    /// Damage/armour scaling: an army two epochs behind is nearly hopeless,
    /// exactly as the GDD demands.
    pub fn combat_scale_milli(self) -> i32 {
        match self {
            Epoch::Stone => 1000,
            Epoch::Bronze => 1300,
            Epoch::Iron => 1650,
            Epoch::EarlyMedieval => 2050,
            Epoch::LateMedieval => 2500,
            Epoch::Industry1 => 3100,
            Epoch::Industry2 => 3900,
        }
    }

    /// Population and cost multiplier applied to units of this epoch.
    pub fn growth_milli(self) -> i32 {
        1000 + self.index() as i32 * 120
    }

    /// Which civilization choices unlock at this epoch.
    pub fn civ_choice_slot(self) -> Option<CivSlot> {
        match self {
            Epoch::Bronze => Some(CivSlot::Culture),
            Epoch::EarlyMedieval => Some(CivSlot::Branch),
            Epoch::Industry1 => Some(CivSlot::Country),
            _ => None,
        }
    }

    /// Requirements to advance INTO the next epoch: (buildings, workers, resources).
    pub fn advance_requirement(self) -> EpochRequirement {
        let defs = [
            EpochRequirement::new(0, 8, Cost::new(150, 150, 0)),
            EpochRequirement::new(3, 12, Cost::new(300, 300, 150)),
            EpochRequirement::new(4, 16, Cost::new(450, 450, 300)),
            EpochRequirement::new(6, 20, Cost::new(700, 700, 450)),
            EpochRequirement::new(8, 24, Cost::new(1000, 1000, 700)),
            EpochRequirement::new(10, 30, Cost::new(1500, 1500, 1000)),
            EpochRequirement::new(12, 36, Cost::new(2200, 2200, 1500)),
        ];
        defs.get(self.index()).copied().unwrap_or(EpochRequirement {
            buildings: u16::MAX,
            workers: u16::MAX,
            resources: Cost::ZERO,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EpochRequirement {
    /// Minimum number of finished buildings.
    pub buildings: u16,
    /// Minimum number of workers.
    pub workers: u16,
    /// Resources that must be *in the bank* when advancing.
    pub resources: Cost,
}

impl EpochRequirement {
    pub const fn new(buildings: u16, workers: u16, resources: Cost) -> Self {
        EpochRequirement {
            buildings,
            workers,
            resources,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum CivSlot {
    Culture,
    Branch,
    Country,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Culture {
    #[default]
    Unchosen,
    Europe,
    Asia,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Branch {
    #[default]
    Unchosen,
    /// Europe: Western, Asia: Eastern
    West,
    East,
}

impl Branch {
    pub fn is_valid_for(self, culture: Culture) -> bool {
        match culture {
            Culture::Europe => matches!(self, Branch::West | Branch::East),
            Culture::Asia => matches!(self, Branch::West | Branch::East),
            Culture::Unchosen => false,
        }
    }
}

/// Playable MVP countries.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Country {
    #[default]
    Unchosen,
    Britain,
    Germany,
    Russia,
    France,
    China,
    Japan,
    Poland,
    AustriaHungary,
    Persia,
    Italy,
    Turkey,
    India,
}

impl Country {
    pub fn is_valid_for(self, culture: Culture) -> bool {
        let europe = matches!(
            self,
            Country::Britain
                | Country::Germany
                | Country::Russia
                | Country::France
                | Country::Poland
                | Country::AustriaHungary
                | Country::Italy
        );
        let asia = matches!(
            self,
            Country::China | Country::Japan | Country::Persia | Country::Turkey | Country::India
        );
        match culture {
            Culture::Europe => europe,
            Culture::Asia => asia,
            Culture::Unchosen => false,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Country::Unchosen => "-",
            Country::Britain => "Britain",
            Country::Germany => "Germany",
            Country::Russia => "Russia",
            Country::France => "France",
            Country::China => "China",
            Country::Japan => "Japan",
            Country::Poland => "Poland",
            Country::AustriaHungary => "Austria-Hungary",
            Country::Persia => "Persia",
            Country::Italy => "Italy",
            Country::Turkey => "Turkey",
            Country::India => "India",
        }
    }
}

/// Full civilization state of one player.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct CivPath {
    pub culture: Culture,
    pub branch: Branch,
    pub country: Country,
}

impl CivPath {
    pub fn choose(&mut self, slot: CivSlot, value: u16) -> bool {
        match slot {
            CivSlot::Culture => {
                if self.culture != Culture::Unchosen {
                    return false;
                }
                let c = match value {
                    1 => Culture::Europe,
                    2 => Culture::Asia,
                    _ => return false,
                };
                self.culture = c;
                true
            }
            CivSlot::Branch => {
                if self.culture == Culture::Unchosen || self.branch != Branch::Unchosen {
                    return false;
                }
                let b = match value {
                    1 => Branch::West,
                    2 => Branch::East,
                    _ => return false,
                };
                if !b.is_valid_for(self.culture) {
                    return false;
                }
                self.branch = b;
                true
            }
            CivSlot::Country => {
                if self.culture == Culture::Unchosen || self.country != Country::Unchosen {
                    return false;
                }
                let country = match value {
                    1 => Country::Britain,
                    2 => Country::Germany,
                    3 => Country::Russia,
                    4 => Country::France,
                    5 => Country::China,
                    6 => Country::Japan,
                    7 => Country::Poland,
                    8 => Country::AustriaHungary,
                    9 => Country::Persia,
                    10 => Country::Italy,
                    11 => Country::Turkey,
                    12 => Country::India,
                    _ => return false,
                };
                if !country.is_valid_for(self.culture) {
                    return false;
                }
                self.country = country;
                true
            }
        }
    }

    /// Country preference for unit stats: every country gets a small identity.
    pub fn combat_bonus_milli(&self) -> (i32, i32) {
        // (melee, ranged)
        match self.country {
            Country::Germany | Country::Russia => (80, -20),
            Country::Britain | Country::France => (-20, 80),
            Country::Japan | Country::China => (20, 40),
            Country::Poland => (60, 20),
            Country::AustriaHungary | Country::Italy => (20, 20),
            Country::Persia | Country::Turkey => (40, -10),
            Country::India => (0, 60),
            Country::Unchosen => (0, 0),
        }
    }
}

/// Resource kinds unlocked per epoch (see GDD section 4).
pub fn resource_available(epoch: Epoch, kind: ResourceKind) -> bool {
    match kind {
        ResourceKind::Copper => epoch >= Epoch::Bronze,
        ResourceKind::Iron => epoch >= Epoch::Iron,
        ResourceKind::Gold => epoch >= Epoch::Bronze,
        ResourceKind::Food | ResourceKind::Wood | ResourceKind::Stone => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_chain() {
        assert_eq!(Epoch::Stone.next(), Some(Epoch::Bronze));
        assert_eq!(Epoch::Industry2.next(), None);
        assert!(Epoch::Industry2.combat_scale_milli() > Epoch::Stone.combat_scale_milli());
    }

    #[test]
    fn civ_branching_rules() {
        let mut civ = CivPath::default();
        assert!(!civ.choose(CivSlot::Branch, 1)); // culture not chosen yet
        assert!(civ.choose(CivSlot::Culture, 1)); // Europe
        assert!(!civ.choose(CivSlot::Culture, 2)); // already chosen
        assert!(civ.choose(CivSlot::Branch, 1)); // West
        assert!(civ.choose(CivSlot::Country, 3)); // Russia
        assert_eq!(civ.country, Country::Russia);
    }

    #[test]
    fn asia_cannot_pick_european_country() {
        let mut civ = CivPath::default();
        civ.choose(CivSlot::Culture, 2);
        assert!(!civ.choose(CivSlot::Country, 1)); // Britain is Europe-only
        assert!(civ.choose(CivSlot::Country, 5)); // China ok
    }
}
